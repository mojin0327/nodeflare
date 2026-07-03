//! Pure, filesystem-free parsers for project manifests. Every function takes file
//! *contents* as `&str` and returns detected values — no IO — so they are shared
//! verbatim between the local-FS builder and the GitHub-API inspect endpoint.
//!
//! Ported from the builder's original inline detection (crates/builder/src/flyctl.rs)
//! so there is a single source of truth for the rules.

/// Node package manager, inferred from lockfiles / `packageManager`. Drives the
/// install + script-runner prefix so a pnpm project runs `pnpm`, not `npm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NodePm {
    #[default]
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl NodePm {
    /// The CLI binary name.
    pub fn cli(self) -> &'static str {
        match self {
            NodePm::Npm => "npm",
            NodePm::Pnpm => "pnpm",
            NodePm::Yarn => "yarn",
            NodePm::Bun => "bun",
        }
    }

    /// The `run` prefix used to invoke package scripts (`npm run` / `pnpm run` / ...).
    pub fn runner(self) -> &'static str {
        match self {
            NodePm::Npm => "npm run",
            NodePm::Pnpm => "pnpm run",
            NodePm::Yarn => "yarn",
            NodePm::Bun => "bun run",
        }
    }
}

/// Extract workspace globs from a `package.json` `workspaces` field (npm/yarn/bun),
/// supporting both the array and `{ "packages": [...] }` object forms.
pub fn parse_workspaces(package_json: &str) -> Vec<String> {
    let Ok(pkg) = serde_json::from_str::<serde_json::Value>(package_json) else {
        return Vec::new();
    };
    match pkg.get("workspaces") {
        Some(serde_json::Value::Array(a)) => {
            a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
        }
        Some(serde_json::Value::Object(o)) => o
            .get("packages")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Extract package globs from a `pnpm-workspace.yaml` `packages:` list. Line-based to
/// avoid a YAML dependency: collects `- <glob>` items under `packages:` (block form)
/// or `packages: [...]` (inline flow form), strips quotes, and skips `!` exclusions.
pub fn parse_pnpm_workspace(yaml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_packages = false;
    for raw in yaml.lines() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indented = line.starts_with([' ', '\t']);
        if !indented {
            in_packages = false;
            if let Some(rest) = trimmed.strip_prefix("packages:") {
                let rest = rest.trim();
                if rest.starts_with('[') {
                    out.extend(parse_inline_glob_array(rest));
                } else {
                    in_packages = true;
                }
            }
            continue;
        }
        if in_packages {
            if let Some(item) = trimmed.strip_prefix('-') {
                let g = item.trim().trim_matches('"').trim_matches('\'').to_string();
                if !g.is_empty() && !g.starts_with('!') {
                    out.push(g);
                }
            }
        }
    }
    out
}

fn parse_inline_glob_array(s: &str) -> Vec<String> {
    s.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|p| p.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|g| !g.is_empty() && !g.starts_with('!'))
        .collect()
}

/// Whether `subdir` (repo-relative, e.g. "src/filesystem") is matched by one of the
/// workspace globs. Supports `prefix/**` (any depth), `prefix/*` (direct child),
/// `*` and literal forms.
pub fn subdir_is_workspace_member(subdir: &str, globs: &[String]) -> bool {
    let subdir = subdir.trim_matches('/');
    globs.iter().any(|g| {
        let g = g.trim_matches('/');
        if let Some(prefix) = g.strip_suffix("/**") {
            let prefix = prefix.trim_matches('/');
            prefix.is_empty()
                || (subdir.len() > prefix.len()
                    && subdir.starts_with(prefix)
                    && subdir.as_bytes().get(prefix.len()) == Some(&b'/'))
        } else if let Some(prefix) = g.strip_suffix("/*") {
            let prefix = prefix.trim_matches('/');
            match subdir.strip_prefix(prefix) {
                Some(rest) => {
                    let rest = rest.trim_start_matches('/');
                    !rest.is_empty() && !rest.contains('/')
                }
                None => false,
            }
        } else if g == "*" {
            !subdir.is_empty() && !subdir.contains('/')
        } else {
            g == subdir
        }
    })
}

/// Whether a member `tsconfig.json` `extends` a path that escapes the member directory
/// (`../...`) — a strong signal that the member shares a repo-root base config and
/// therefore cannot be type-checked/built standalone.
pub fn tsconfig_extends_parent(tsconfig: &str) -> bool {
    // tsconfig allows comments/trailing commas; a full JSONC parse is overkill for one
    // field, so locate the `"extends": "..."` string value anywhere in the content.
    let Some(after_key) = tsconfig.split("\"extends\"").nth(1) else {
        return false;
    };
    let Some(after_colon) = after_key.trim_start().strip_prefix(':') else {
        return false;
    };
    let rest = after_colon.trim_start();
    let Some(rest) = rest.strip_prefix('"') else {
        return false;
    };
    let value = rest.split('"').next().unwrap_or("");
    value.starts_with("../") || value.contains("/../")
}

/// Rewrite an entry command so it works when the build context is the repo ROOT but the
/// server lives in `subdir`. Handles `node <script> [args]` (dominant TS-MCP shape) by
/// prefixing the script path, `npm`/`pnpm` via their workspace selectors. Any other
/// shape is returned unchanged.
pub fn prefix_entry_with_subdir(entry: &str, subdir: &str) -> String {
    let subdir = subdir.trim_matches('/');
    if subdir.is_empty() {
        return entry.to_string();
    }
    let parts: Vec<&str> = entry.split_whitespace().collect();
    match parts.as_slice() {
        ["node", script, rest @ ..] if !script.starts_with('/') && !script.starts_with('-') => {
            let mut out = format!("node {}/{}", subdir, script);
            for r in rest {
                out.push(' ');
                out.push_str(r);
            }
            out
        }
        ["npm", rest @ ..] if !rest.is_empty() => {
            format!("npm --prefix {} {}", subdir, rest.join(" "))
        }
        ["pnpm", rest @ ..] if !rest.is_empty() => {
            format!("pnpm --filter ./{} {}", subdir, rest.join(" "))
        }
        _ => entry.to_string(),
    }
}

/// Derive a Node start command from `package.json` contents.
/// Priority: `scripts.start` (→ `<pm> start`) > `main` (→ `node <main>`) > `bin`.
pub fn parse_node_entry(package_json: &str, pm: NodePm) -> Option<String> {
    let pkg: serde_json::Value = serde_json::from_str(package_json).ok()?;

    if pkg
        .get("scripts")
        .and_then(|s| s.get("start"))
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
    {
        return Some(format!("{} start", pm.cli()));
    }

    if let Some(main) = pkg.get("main").and_then(|v| v.as_str()) {
        if !main.trim().is_empty() {
            return Some(format!("node {}", main.trim()));
        }
    }

    match pkg.get("bin") {
        Some(serde_json::Value::String(path)) if !path.trim().is_empty() => {
            return Some(format!("node {}", path.trim()));
        }
        Some(serde_json::Value::Object(map)) => {
            let name = pkg.get("name").and_then(|v| v.as_str());
            if let Some(n) = name {
                if let Some(path) = map.get(n).and_then(|v| v.as_str()) {
                    if !path.trim().is_empty() {
                        return Some(format!("node {}", path.trim()));
                    }
                }
            }
            for path in map.values().filter_map(|v| v.as_str()) {
                if !path.trim().is_empty() {
                    return Some(format!("node {}", path.trim()));
                }
            }
        }
        _ => {}
    }

    None
}

/// Whether `package.json` declares a non-empty `build` script.
pub fn node_has_build_script(package_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(package_json)
        .ok()
        .and_then(|v| {
            v.get("scripts")
                .and_then(|s| s.get("build"))
                .and_then(|b| b.as_str())
                .map(|s| !s.trim().is_empty())
        })
        .unwrap_or(false)
}

/// First console-script name from a pyproject.toml `[project.scripts]` table.
pub fn parse_pyproject_script(pyproject: &str) -> Option<String> {
    let val: toml::Value = toml::from_str(pyproject).ok()?;
    let scripts = val.get("project")?.get("scripts")?.as_table()?;
    scripts.keys().next().cloned()
}

/// Binary name produced by `cargo build`: first `[[bin]]` name, else `[package].name`.
pub fn parse_cargo_bin(cargo_toml: &str) -> Option<String> {
    let val: toml::Value = toml::from_str(cargo_toml).ok()?;

    if let Some(name) = val
        .get("bin")
        .and_then(|b| b.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("name"))
        .and_then(|n| n.as_str())
    {
        if !name.trim().is_empty() {
            return Some(name.trim().to_string());
        }
    }

    val.get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_member_matching() {
        let globs = vec!["src/*".to_string()];
        assert!(subdir_is_workspace_member("src/sequentialthinking", &globs));
        assert!(!subdir_is_workspace_member("src/a/b", &globs));
        assert!(!subdir_is_workspace_member("packages/x", &globs));
        assert!(subdir_is_workspace_member("packages/x", &["packages/**".into()]));
    }

    #[test]
    fn node_entry_priority() {
        assert_eq!(
            parse_node_entry(r#"{"scripts":{"start":"node x"}}"#, NodePm::Pnpm).as_deref(),
            Some("pnpm start")
        );
        assert_eq!(
            parse_node_entry(r#"{"main":"dist/index.js"}"#, NodePm::Npm).as_deref(),
            Some("node dist/index.js")
        );
        assert_eq!(
            parse_node_entry(r#"{"name":"p","bin":{"p":"build/cli.js"}}"#, NodePm::Npm).as_deref(),
            Some("node build/cli.js")
        );
    }

    #[test]
    fn tsconfig_extends_detection() {
        assert!(tsconfig_extends_parent(r#"{ "extends": "../../tsconfig.json" }"#));
        assert!(!tsconfig_extends_parent(r#"{ "extends": "./base.json" }"#));
    }

    #[test]
    fn prefix_entry() {
        assert_eq!(
            prefix_entry_with_subdir("node dist/index.js", "src/foo"),
            "node src/foo/dist/index.js"
        );
    }
}

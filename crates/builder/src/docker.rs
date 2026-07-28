use regex::Regex;

/// Parse a Node.js engines.node specifier and return the minimum required major version.
/// Handles OR branches by taking the minimum (compatible with any branch).
pub(crate) fn min_node_major_from_spec(spec: &str) -> Option<u32> {
    spec.split("||")
        .filter_map(|branch| {
            let b = branch.trim();
            // Match >=X, >X, ^X, ~X, or a bare number at the start
            let re = Regex::new(r"(?:>=|>|\^|~)\s*(\d+)|^(\d+)").ok()?;
            re.captures(b).and_then(|c| {
                c.get(1).or(c.get(2))
                    .and_then(|m| m.as_str().parse::<u32>().ok())
            })
        })
        .min()
}

/// Scan package-lock.json (lockfileVersion 2/3) for the highest minimum required Node.js version.
/// Returns (major_version, "package@version") or None if no engines.node constraints found.
pub(crate) fn max_node_requirement_from_lockfile(lockfile_json: &str) -> Option<(u32, String)> {
    let lockfile: serde_json::Value = serde_json::from_str(lockfile_json).ok()?;
    let packages = lockfile.get("packages")?.as_object()?;

    let mut max_major: u32 = 0;
    let mut culprit = String::new();

    for (pkg_path, pkg_info) in packages {
        if pkg_path.is_empty() {
            continue; // root package entry — skip
        }
        let Some(node_spec) = pkg_info
            .get("engines")
            .and_then(|e| e.get("node"))
            .and_then(|n| n.as_str())
        else {
            continue;
        };

        if let Some(major) = min_node_major_from_spec(node_spec) {
            if major > max_major {
                max_major = major;
                // pkg_path looks like "node_modules/undici" or nested "node_modules/a/node_modules/b"
                let name = pkg_path
                    .rsplit("node_modules/")
                    .next()
                    .unwrap_or(pkg_path)
                    .trim_matches('/');
                let version = pkg_info.get("version").and_then(|v| v.as_str()).unwrap_or("");
                culprit = if version.is_empty() {
                    name.to_string()
                } else {
                    format!("{}@{}", name, version)
                };
            }
        }
    }

    if max_major > 0 { Some((max_major, culprit)) } else { None }
}

/// Extract the Node.js major version from a Dockerfile's FROM line (first `FROM node:X` match).
pub(crate) fn node_major_in_dockerfile(dockerfile: &str) -> Option<u32> {
    let re = Regex::new(r"(?im)^\s*FROM\s+node:(\d+)").ok()?;
    re.captures(dockerfile)?.get(1)?.as_str().parse().ok()
}

/// Replace all `node:X` / `node:X.Y.Z` occurrences in a Dockerfile with `node:{target_major}`.
/// Preserves the image variant suffix (e.g. `-alpine`, `-slim`).
pub(crate) fn set_node_version_in_dockerfile(dockerfile: &str, target_major: u32) -> String {
    let re = Regex::new(r"\bnode:(\d+)(?:\.\d+)*").unwrap();
    re.replace_all(dockerfile, |_: &regex::Captures| format!("node:{}", target_major))
        .into_owned()
}

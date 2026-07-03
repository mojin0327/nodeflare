//! Heuristic source scanning — the weakest signal tier, used only when a manifest did
//! not already declare a field. Operates on concatenated source text (a handful of
//! probed entry files), so it is pure and dependency-free.

use crate::Transport;

/// Decide transport from source text. Explicit SDK transport classes win; otherwise an
/// HTTP server framework / port binding implies HTTP; the MCP-spec default is STDIO.
/// Returns `None` when there is no signal either way (caller applies the default).
pub fn scan_transport(src: &str) -> Option<Transport> {
    // Explicit, unambiguous transport instantiation (TS SDK).
    let http_markers = [
        "StreamableHTTPServerTransport",
        "SSEServerTransport",
        "streamable-http",
        "transport=\"http\"",
        "transport='http'",
        "transport=\"streamable-http\"",
        "transport='streamable-http'",
        "transport=\"sse\"",
        "transport='sse'",
        ".streamable_http_app(",
        ".sse_app(",
    ];
    let stdio_markers = [
        "StdioServerTransport",
        "stdio_server(",
        "mcp.server.stdio",
        "transport=\"stdio\"",
        "transport='stdio'",
    ];

    let has_http = http_markers.iter().any(|m| src.contains(m));
    let has_stdio = stdio_markers.iter().any(|m| src.contains(m));

    match (has_http, has_stdio) {
        (true, false) => Some(Transport::Sse),
        (false, true) => Some(Transport::Stdio),
        (true, true) => Some(Transport::Sse), // dual-transport: prefer deployable HTTP
        (false, false) => {
            // No explicit SDK transport — infer from an HTTP server / port binding.
            let http_hint = [
                "express(",
                "createServer(",
                "fastify(",
                "new Hono(",
                "uvicorn.run",
                "app.listen(",
                "FastAPI(",
                "starlette",
                "process.env.PORT",
                "os.environ[\"PORT\"]",
                "os.environ['PORT']",
            ]
            .iter()
            .any(|m| src.contains(m));
            http_hint.then_some(Transport::Sse)
        }
    }
}

/// Best-effort listening port from source: a hardcoded `.listen(NNNN`, `port=NNNN`,
/// `port: NNNN`, or `PORT || NNNN`. Returns None when only `$PORT` (dynamic) is used.
pub fn scan_port(src: &str) -> Option<u16> {
    for marker in [".listen(", "port=", "port:", "port =", "PORT || ", "PORT ||", "port', "] {
        if let Some(p) = find_number_after(src, marker) {
            if (1..=65535).contains(&p) {
                return Some(p as u16);
            }
        }
    }
    None
}

fn find_number_after(hay: &str, marker: &str) -> Option<u32> {
    let mut from = 0;
    while let Some(idx) = hay[from..].find(marker) {
        let start = from + idx + marker.len();
        let rest = hay[start..].trim_start_matches(|c: char| c == '\'' || c == '"' || c == ' ');
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<u32>() {
            return Some(n);
        }
        from = start;
    }
    None
}

/// Extract environment-variable keys read from source: `process.env.X`,
/// `process.env['X']`, `os.environ["X"]`, `os.environ.get("X")`, `os.getenv("X")`.
pub fn scan_env_keys(src: &str) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let mut push = |k: String| {
        if is_env_key(&k) && !keys.contains(&k) {
            keys.push(k);
        }
    };

    // process.env.NAME
    collect_after(src, "process.env.", |c| c.is_ascii_alphanumeric() || c == '_', &mut push);
    // bracketed / quoted forms
    for marker in [
        "process.env[",
        "os.environ[",
        "os.environ.get(",
        "os.getenv(",
        "env.get(",
    ] {
        collect_quoted_after(src, marker, &mut push);
    }
    keys.sort();
    keys
}

/// Keys from a `.env.example` file (lines of the form `KEY=...` or `KEY`).
pub fn env_keys_from_example(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in content.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let l = l.strip_prefix("export ").unwrap_or(l);
        let key = l.split('=').next().unwrap_or("").trim();
        if is_env_key(key) && !out.contains(&key.to_string()) {
            out.push(key.to_string());
        }
    }
    out
}

fn is_env_key(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 64
        && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && k.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
        // Skip a couple of platform-provided vars the user never sets.
        && !matches!(k, "NODE_ENV" | "PATH" | "HOME" | "PORT" | "PWD" | "USER")
}

fn collect_after(hay: &str, marker: &str, is_ident: impl Fn(char) -> bool, push: &mut impl FnMut(String)) {
    let mut from = 0;
    while let Some(idx) = hay[from..].find(marker) {
        let start = from + idx + marker.len();
        let ident: String = hay[start..].chars().take_while(|&c| is_ident(c)).collect();
        if !ident.is_empty() {
            push(ident);
        }
        from = start;
    }
}

fn collect_quoted_after(hay: &str, marker: &str, push: &mut impl FnMut(String)) {
    let mut from = 0;
    while let Some(idx) = hay[from..].find(marker) {
        let start = from + idx + marker.len();
        let rest = hay[start..].trim_start();
        if let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let inner: String = rest[1..].chars().take_while(|&c| c != q).collect();
            if !inner.is_empty() {
                push(inner);
            }
        }
        from = start;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_explicit() {
        assert_eq!(scan_transport("new StdioServerTransport()"), Some(Transport::Stdio));
        assert_eq!(
            scan_transport("new StreamableHTTPServerTransport()"),
            Some(Transport::Sse)
        );
        assert_eq!(scan_transport("mcp.run(transport=\"http\", port=8000)"), Some(Transport::Sse));
    }

    #[test]
    fn transport_inferred_and_none() {
        assert_eq!(scan_transport("const app = express(); app.listen(3000)"), Some(Transport::Sse));
        assert_eq!(scan_transport("const x = 1;"), None);
    }

    #[test]
    fn port_scan() {
        assert_eq!(scan_port("app.listen(3000)"), Some(3000));
        assert_eq!(scan_port("uvicorn.run(app, port=8000)"), Some(8000));
        assert_eq!(scan_port("app.listen(process.env.PORT)"), None);
    }

    #[test]
    fn env_scan() {
        let src = r#"const k = process.env.API_KEY; const u = process.env["DATABASE_URL"];
            token = os.environ.get("GITHUB_TOKEN")"#;
        let keys = scan_env_keys(src);
        assert!(keys.contains(&"API_KEY".to_string()));
        assert!(keys.contains(&"DATABASE_URL".to_string()));
        assert!(keys.contains(&"GITHUB_TOKEN".to_string()));
    }
}

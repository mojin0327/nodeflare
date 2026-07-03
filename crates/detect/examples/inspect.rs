//! Manual verification helper: run the real detection algorithm over a local repo.
//!   cargo run -p mcp-detect --example inspect -- <repo_root> [subdir]
use mcp_detect::{inspect, LocalFs};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let root = args.next().expect("usage: inspect <repo_root> [subdir]");
    let subdir = args.next();
    let files = LocalFs::new(&root);
    let d = inspect(&files, subdir.as_deref()).await;
    println!("{}", serde_json::to_string_pretty(&d).unwrap());
}

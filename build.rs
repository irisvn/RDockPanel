//! What the binary says of itself (Settings → Panel, the Dashboard):
//! `RDP_VERSION` (a release's `X.Y.Z`, else the crate's version), its
//! commit (`RDP_COMMIT`, set by the image's build; else Git's, from
//! source), and the RustCMS Core it was built with (`Cargo.lock`).

fn main() {
    println!("cargo:rerun-if-env-changed=RDP_VERSION");
    println!("cargo:rerun-if-env-changed=RDP_COMMIT");
    println!("cargo:rerun-if-changed=Cargo.lock");
    // From source, the commit is Git's: its branch's ref (not only HEAD,
    // which stays `ref: refs/heads/main` as commits advance). Only paths
    // that exist (an image's build has no `.git`: never rerun for it).
    if std::env::var("RDP_COMMIT").is_err() {
        let head = std::path::Path::new(".git/HEAD");
        if head.is_file() {
            println!("cargo:rerun-if-changed=.git/HEAD");
            if let Some(reference) = std::fs::read_to_string(head)
                .ok()
                .and_then(|h| h.trim().strip_prefix("ref: ").map(str::to_owned))
            {
                let path = format!(".git/{reference}");
                if std::path::Path::new(&path).is_file() {
                    println!("cargo:rerun-if-changed={path}");
                }
            }
            if std::path::Path::new(".git/packed-refs").is_file() {
                println!("cargo:rerun-if-changed=.git/packed-refs");
            }
        }
    }
    let version = std::env::var("RDP_VERSION")
        .ok()
        .map(|v| v.trim().trim_start_matches('v').to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap_or_default());
    let commit = std::env::var("RDP_COMMIT")
        .ok()
        .filter(|c| !c.trim().is_empty())
        .or_else(|| {
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        })
        .map(|c| c.trim().chars().take(12).collect::<String>())
        .unwrap_or_default();
    let lock = std::fs::read_to_string("Cargo.lock").unwrap_or_default();
    let core = lock
        .split("[[package]]")
        .find(|p| p.lines().any(|l| l.trim() == "name = \"rustcms\""))
        .and_then(|p| {
            p.lines()
                .find_map(|l| l.trim().strip_prefix("version = \""))
                .map(|v| v.trim_end_matches('"').to_owned())
        })
        .unwrap_or_default();
    println!("cargo:rustc-env=RDP_VERSION={version}");
    println!("cargo:rustc-env=RDP_COMMIT={commit}");
    println!("cargo:rustc-env=RDP_CORE_VERSION={core}");
}

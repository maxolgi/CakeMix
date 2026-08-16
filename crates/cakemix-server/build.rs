/// Ensure the rust-embed folders exist so `#[derive(Embed)]` doesn't fail at
/// compile time when the build outputs haven't been generated yet. The
/// directories may be empty (all embedded assets return `None`) — the server
/// still compiles and runs; it just 404s the web UI until `make build-ui`
/// (and friends) populate them. Same pattern as websrt-gateway's build.rs.
fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = std::path::Path::new(&manifest_dir).join("../..");

    // (path relative to the workspace root, make target that builds it)
    for (rel, hint) in [
        ("web", "make build-ui + make build-worklet"),
        ("crates/mixer-wasm/pkg", "make build-web"),
        ("vendor/WebSRT/web/dist", "make build-websrt-web"),
    ] {
        let dir = root.join(rel);
        if !dir.exists() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join(".gitkeep"), "");
            println!("cargo:warning={rel}/ was empty; run '{hint}' to build it");
        }
        // rerun-if-changed is resolved relative to CARGO_MANIFEST_DIR.
        let crate_rel = format!("../../{rel}");
        println!("cargo:rerun-if-changed={crate_rel}");
    }
}

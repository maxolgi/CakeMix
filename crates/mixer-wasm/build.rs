/// Compile the gxfx guitarix faust C++ DSP sources — every `.cpp` directly
/// under `gxfx/` — into one static library with plain `clang++` + `ar`
/// invocations (no `cc` crate, no build-dependencies). The DSP sources are
/// consumed from the pinned `vendor/guitarix` submodule; the wasm32 build is
/// freestanding (`-nostdinc`/`-nostdinc++`) with `gxfx/shim/` first on the
/// include path so the C++ standard-name headers resolve to our shims.
use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run(command: &mut Command) {
    let program = command.get_program().to_string_lossy().into_owned();
    match command.output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => panic!(
            "`{program}` failed ({}): {command:?}\n--- stderr ---\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) if e.kind() == io::ErrorKind::NotFound => panic!(
            "`{program}` not found on PATH — building mixer-wasm requires clang++ and ar \
             (install the system clang/LLVM toolchain); {e}"
        ),
        Err(e) => panic!("failed to run `{program}`: {e}"),
    }
}

fn rerun_if_changed_recursive(dir: &Path) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
        Err(e) => panic!("cannot read {}: {e}", dir.display()),
    };
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rerun_if_changed_recursive(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap();

    let gxfx = manifest_dir.join("gxfx");
    let root = manifest_dir.join("../.."); // crates/mixer-wasm -> workspace root
    let vendor_dsp = root.join("vendor/guitarix/trunk/src/LV2/DSP");
    let vendor_faust = root.join("vendor/guitarix/trunk/src/LV2/faust-generated");

    if !vendor_dsp.is_dir() || !vendor_faust.is_dir() {
        panic!(
            "vendor/guitarix is missing or not initialized (expected {}): run \
             `git submodule update --init vendor/guitarix` and rebuild",
            root.join("vendor/guitarix").display()
        );
    }

    // Rebuild whenever anything under gxfx/ changes (headers, shims, TUs);
    // the directory lines also catch files added or removed later.
    rerun_if_changed_recursive(&gxfx);

    // Every .cpp directly under gxfx/ is a translation unit (shim/ holds
    // only headers). Sorted for a deterministic archive member order.
    let mut sources: Vec<PathBuf> = match std::fs::read_dir(&gxfx) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "cpp"))
            .collect(),
        Err(e) => panic!("cannot read {}: {e}", gxfx.display()),
    };
    sources.sort();

    let is_wasm = target.starts_with("wasm32");
    let mut includes: Vec<String> = Vec::new();
    if is_wasm {
        // The freestanding shims must shadow the C++ standard-name headers,
        // so gxfx/shim goes FIRST — before the gxfx and vendor includes.
        includes.push(gxfx.join("shim").display().to_string());
    }
    includes.push(gxfx.display().to_string());
    includes.push(vendor_dsp.display().to_string());
    includes.push(vendor_faust.display().to_string());

    let mut flags: Vec<String> = [
        "-std=c++20",
        "-O2",
        "-fno-exceptions",
        "-fno-rtti",
        "-Wno-vla-cxx-extension",
        "-Wall",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if is_wasm {
        flags.extend(
            [
                "--target=wasm32-unknown-unknown",
                "-nostdinc",
                "-nostdinc++",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    } else {
        flags.push("-fPIC".to_string());
    }

    let mut objects: Vec<PathBuf> = Vec::new();
    for src in &sources {
        let obj = out_dir.join(src.file_name().unwrap()).with_extension("o");
        let mut cmd = Command::new("clang++");
        cmd.args(&flags)
            .args(includes.iter().map(|i| format!("-I{i}")))
            .arg("-c")
            .arg(src)
            .arg("-o")
            .arg(&obj);
        run(&mut cmd);
        objects.push(obj);
    }

    // Start from a clean archive so a removed TU cannot linger as a member.
    let lib = out_dir.join("libgxfx_dsp.a");
    let _ = std::fs::remove_file(&lib);
    let mut ar = Command::new("ar");
    ar.arg("rcs").arg(&lib).args(&objects);
    run(&mut ar);

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=gxfx_dsp");
}

/// Compile the gxfx guitarix faust C++ DSP sources — every `.cpp` directly
/// under `gxfx/` — into one static library with a discovered clang++ plus
/// a discovered archiver (no `cc` crate, no build-dependencies). Both are
/// resolved by probing candidates with the exact invocations this build
/// uses: the compiler (`$CXX`, PATH `clang++`, then emsdk/homebrew LLVM —
/// Apple's clang++ ships without the WebAssembly backend) and the archiver
/// (`$AR`, `ar`, `llvm-ar` — see `resolve_archiver`). The DSP sources are
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
            "`{program}` not found on PATH — building mixer-wasm requires \
             clang++ and an archiver (ar or llvm-ar from the clang/LLVM \
             toolchain); {e}"
        ),
        Err(e) => panic!("failed to run `{program}`: {e}"),
    }
}

/// C++ compiler candidates in priority order: an explicit `$CXX` override,
/// `clang++` on PATH, then wasm-capable toolchains that live off PATH —
/// emsdk (`$EMSDK`, else the `~/emsdk` CI installs) and homebrew LLVM.
/// Apple's clang++ ships without the WebAssembly backend, so on macOS the
/// PATH hit fails the probe and emsdk's clang (already a prerequisite of
/// the web build) takes over; Linux and Windows keep their PATH clang++.
fn cxx_candidates() -> Vec<String> {
    let mut candidates = Vec::new();
    if let Ok(cxx) = env::var("CXX") {
        if !cxx.trim().is_empty() {
            candidates.push(cxx);
        }
    }
    candidates.push("clang++".to_string());
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(emsdk) = env::var("EMSDK") {
        dirs.push(Path::new(&emsdk).join("upstream/bin"));
    }
    if let Ok(home) = env::var("HOME") {
        dirs.push(Path::new(&home).join("emsdk/upstream/bin"));
    }
    dirs.push(PathBuf::from("/opt/homebrew/opt/llvm/bin"));
    // Explicit dirs bypass the OS search, so only offer file names that exist
    // (.exe included for Windows).
    for dir in dirs {
        for name in ["clang++", "clang++.exe"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                candidates.push(candidate.display().to_string());
            }
        }
    }
    candidates
}

/// Archiver candidates in priority order: an explicit `$AR` override, LLVM's
/// `llvm-ar` beside the resolved C++ compiler when that compiler came from
/// an off-PATH toolchain dir (a matched pair — a system `ar` may not index
/// that toolchain's objects), the classic `ar`, `llvm-ar` on PATH (windows-
/// latest ships LLVM's llvm-ar without binutils `ar`), and finally
/// `llvm-ar` sitting in the same bin dir as the `clang++` found on PATH.
fn archiver_candidates(cxx: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    if let Ok(ar) = env::var("AR") {
        if !ar.trim().is_empty() {
            candidates.push(ar);
        }
    }
    let cxx_path = Path::new(cxx);
    if cxx_path
        .parent()
        .is_some_and(|dir| !dir.as_os_str().is_empty())
    {
        for name in ["llvm-ar", "llvm-ar.exe"] {
            let candidate = cxx_path.parent().unwrap().join(name);
            if candidate.is_file() {
                candidates.push(candidate.display().to_string());
            }
        }
    }
    candidates.push("ar".to_string());
    candidates.push("llvm-ar".to_string());
    if let Some(dir) = clang_bin_dir() {
        for name in ["llvm-ar", "llvm-ar.exe"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                candidates.push(candidate.display().to_string());
            }
        }
    }
    candidates
}

/// Directory holding the `clang++` this build will drive (first hit on
/// PATH), so the LLVM `llvm-ar` shipped beside it can serve as a fallback.
fn clang_bin_dir() -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .find(|dir| dir.join("clang++").is_file() || dir.join("clang++.exe").is_file())
}

/// Probe each C++ compiler candidate by compiling a scratch TU with the
/// exact flags this build passes — a compiler that cannot create the target
/// (Apple clang++ asked for wasm32 has no WebAssembly backend) fails here
/// with a clear message instead of mid-build. Returns the winner plus the
/// probe object, which `resolve_archiver` reuses so archivers are probed on
/// a real object of the kind this build archives.
fn resolve_cxx(target: &str, flags: &[String], out_dir: &Path) -> (String, PathBuf) {
    let src = out_dir.join(".cxx-probe.cpp");
    let obj = out_dir.join(".cxx-probe.o");
    if let Err(e) = std::fs::write(
        &src,
        b"extern \"C\" int gxfx_build_probe(void) { return 1; }\n",
    ) {
        panic!("cannot write {}: {e}", src.display());
    }
    let mut failures = Vec::new();
    for candidate in cxx_candidates() {
        // A failed compile can leave a stale object; never let one masquerade
        // as the next candidate's success.
        let _ = std::fs::remove_file(&obj);
        match Command::new(&candidate)
            .args(flags)
            .arg("-c")
            .arg(&src)
            .arg("-o")
            .arg(&obj)
            .output()
        {
            Ok(out) if out.status.success() && obj.is_file() => return (candidate, obj),
            Ok(out) => failures.push(format!(
                "`{candidate}` probe compile exited {}{}",
                out.status,
                stderr_note(&out.stderr)
            )),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                failures.push(format!("`{candidate}` not found"))
            }
            Err(e) => failures.push(format!("`{candidate}` failed to run: {e}")),
        }
    }
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&obj);
    panic!(
        "no C++ compiler able to target {target} for the gxfx static library — \
         tried:\n  {}\n  set CXX to a clang++ that can (Apple's clang++ cannot: \
         it ships without the WebAssembly backend; emsdk's or LLVM's can)",
        failures.join("\n  ")
    );
}

/// Probe each archiver candidate by archiving the compiler-probe object — a
/// real object of the kind this build archives — with the exact `rcs`
/// invocation used for libgxfx_dsp.a. (`--version` is not a portable
/// capability test: macOS's BSD `ar` answers no version flag and exits
/// nonzero despite archiving fine.) Panics with everything tried if none do.
fn resolve_archiver(cxx: &str, probe_obj: &Path, out_dir: &Path) -> String {
    let archive = out_dir.join(".ar-probe.a");
    let mut failures = Vec::new();
    for candidate in archiver_candidates(cxx) {
        let _ = std::fs::remove_file(&archive);
        match Command::new(&candidate)
            .arg("rcs")
            .arg(&archive)
            .arg(probe_obj)
            .output()
        {
            Ok(out) if out.status.success() && archive.is_file() => {
                let _ = std::fs::remove_file(&archive);
                return candidate;
            }
            Ok(out) => failures.push(format!(
                "`{candidate}` `rcs` probe exited {}{}",
                out.status,
                stderr_note(&out.stderr)
            )),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                failures.push(format!("`{candidate}` not found"))
            }
            Err(e) => failures.push(format!("`{candidate}` failed to run: {e}")),
        }
        let _ = std::fs::remove_file(&archive);
    }
    panic!(
        "no working archiver for the gxfx static library — tried:\n  {}\n  \
         set AR to an archiver, or install ar (binutils) or llvm-ar (ships \
         beside clang++)",
        failures.join("\n  ")
    );
}

/// Compact `: <first lines of stderr>` suffix for probe failure messages.
fn stderr_note(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr).trim().to_string();
    if text.is_empty() {
        String::new()
    } else {
        format!(": {text}")
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

    // A set AR or CXX changes which tools run, so either must trigger a
    // rebuild.
    println!("cargo:rerun-if-env-changed=AR");
    println!("cargo:rerun-if-env-changed=CXX");

    let gxfx = manifest_dir.join("gxfx");
    let root = manifest_dir.join("../.."); // crates/mixer-wasm -> workspace root
    let vendor_dsp = root.join("vendor/guitarix/trunk/src/LV2/DSP");
    let vendor_faust = root.join("vendor/guitarix/trunk/src/LV2/faust-generated");
    // zita-resampler: <zita-resampler/resampler.h> pulled in by
    // gx_resampler.h (distortion's oversampling chain). LV2/faust:
    // bossds1.cc includes "clipping.h", which lives in the faust (dsp
    // source) dir, one level up from faust-generated.
    let vendor_zita = root.join("vendor/guitarix/trunk/src/zita-resampler-1.1.0");
    let vendor_clipping = root.join("vendor/guitarix/trunk/src/LV2/faust");
    // circuit_tables: the guard-less tube/copicat transfer-table headers
    // the time family's gxtape/gxtape_st/gxechocat/gxtubedelay .cc files
    // include by bare name — exactly the include dir OctOBX's documented
    // compile line for gxfx_dsp_time.cpp passes. One push on the shared
    // include list covers BOTH targets (native and wasm32).
    let vendor_circuit_tables = root.join("vendor/guitarix/trunk/src/LV2/DSP/circuit_tables");

    if !vendor_dsp.is_dir()
        || !vendor_faust.is_dir()
        || !vendor_zita.is_dir()
        || !vendor_clipping.is_dir()
        || !vendor_circuit_tables.is_dir()
    {
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
    includes.push(vendor_zita.display().to_string());
    includes.push(vendor_clipping.display().to_string());
    includes.push(vendor_circuit_tables.display().to_string());

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
    // Resolve the compiler and archiver before compiling, probing each with
    // the exact invocations used below, so a machine without a usable
    // toolchain fails fast instead of after a partial C++ compile.
    let (cxx, probe_obj) = resolve_cxx(&target, &flags, &out_dir);
    let ar_program = resolve_archiver(&cxx, &probe_obj, &out_dir);
    let _ = std::fs::remove_file(&probe_obj);
    let _ = std::fs::remove_file(out_dir.join(".cxx-probe.cpp"));
    for src in &sources {
        let obj = out_dir.join(src.file_name().unwrap()).with_extension("o");
        let mut cmd = Command::new(&cxx);
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
    let mut ar = Command::new(&ar_program);
    ar.arg("rcs").arg(&lib).args(&objects);
    run(&mut ar);

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=gxfx_dsp");
}

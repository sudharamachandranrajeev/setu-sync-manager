//! Builds the Go engine in `go/` into a static library (`go build -buildmode=c-archive`) and
//! links it into whatever uses this crate.
//!
//! Needs Go and a C compiler for cgo. They are found, in this order, through:
//! - `SETU_GO` (the `go` binary) and `SETU_CGO_CC` (the C compiler);
//! - a `.tools` folder next to the repository (local builds keep their toolchains there):
//!   `.tools/go*/go/bin/go` and, on Windows, `.tools/llvm-mingw-*/bin/x86_64-w64-mingw32-clang`;
//! - `go` and `cc` (Windows: `x86_64-w64-mingw32-gcc`) on the PATH.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Syncthing's own build tags for a build without the web GUI and without self-upgrade.
const TAGS: &str = "noupgrade,noassets";
/// The Syncthing version pinned in go/go.mod, stamped into the engine so peers see it.
const SYNCTHING_VERSION: &str = "v2.1.7";

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let go_dir = manifest.join("go");
    for f in ["engine.go", "calls.go", "exports.go", "go.mod", "go.sum"] {
        println!("cargo:rerun-if-changed={}", go_dir.join(f).display());
    }
    for v in ["SETU_GO", "SETU_CGO_CC"] {
        println!("cargo:rerun-if-env-changed={v}");
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let tools = manifest.parent().and_then(Path::parent).map(|p| p.join(".tools"));
    let go = env::var_os("SETU_GO").map(PathBuf::from).or_else(|| tools.as_deref().and_then(tool_go)).unwrap_or_else(|| "go".into());
    let cc = env::var_os("SETU_CGO_CC").map(PathBuf::from).or_else(|| {
        if target_os == "windows" {
            tools.as_deref().and_then(tool_mingw).or_else(|| Some("x86_64-w64-mingw32-gcc".into()))
        } else {
            None
        }
    });

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let archive = out.join("libsetuengine.a");
    let mut cmd = Command::new(&go);
    cmd.current_dir(&go_dir)
        .args(["build", "-buildmode=c-archive", "-trimpath", "-tags", TAGS])
        .arg(format!("-ldflags=-s -w -X github.com/syncthing/syncthing/lib/build.Version={SYNCTHING_VERSION}"))
        .arg("-o")
        .arg(&archive)
        .arg(".")
        .env("CGO_ENABLED", "1")
        .env("GOTOOLCHAIN", "local");
    if let Some(cc) = &cc {
        cmd.env("CC", cc);
    }
    if let Some(t) = tools.as_deref().filter(|t| t.is_dir()) {
        // Keep downloaded modules and the build cache with the toolchain, not in the profile.
        cmd.env("GOPATH", t.join("gopath")).env("GOCACHE", t.join("gocache"));
    }
    let status = cmd.status().unwrap_or_else(|e| {
        panic!("setu-engine needs Go ({}): {e}. Install Go, or set SETU_GO to the go binary.", go.display())
    });
    assert!(status.success(), "building the Go engine failed ({status})");

    println!("cargo:rustc-link-search=native={}", out.display());
    if target_env == "msvc" {
        // The MSVC linker looks for `<name>.lib`; the archive format is the same.
        std::fs::copy(&archive, out.join("setuengine.lib")).unwrap();
    }
    println!("cargo:rustc-link-lib=static=setuengine");
    match target_os.as_str() {
        "windows" => {
            for lib in ["ws2_32", "winmm", "ntdll", "userenv", "iphlpapi", "bcrypt", "advapi32", "crypt32", "secur32", "ole32", "shell32"] {
                println!("cargo:rustc-link-lib=dylib={lib}");
            }
        }
        _ => {
            for lib in ["pthread", "dl", "m", "resolv"] {
                println!("cargo:rustc-link-lib=dylib={lib}");
            }
        }
    }
}

/// `.tools/go*/go/bin/go(.exe)`, newest first by name.
fn tool_go(tools: &Path) -> Option<PathBuf> {
    // "go1." and not "go": gopath and gocache live there too.
    newest(tools, "go1.").map(|d| d.join("go").join("bin").join(if cfg!(windows) { "go.exe" } else { "go" })).filter(|p| p.is_file())
}

/// `.tools/llvm-mingw-*/bin/x86_64-w64-mingw32-clang(.exe)`.
fn tool_mingw(tools: &Path) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "x86_64-w64-mingw32-clang.exe" } else { "x86_64-w64-mingw32-clang" };
    newest(tools, "llvm-mingw-").map(|d| d.join("bin").join(exe)).filter(|p| p.is_file())
}

fn newest(dir: &Path, prefix: &str) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix) && e.path().is_dir())
        .map(|e| e.path())
        .collect();
    found.sort();
    found.pop()
}

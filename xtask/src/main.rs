//! Project automation, run as `cargo xtask <command>` (alias in
//! `.cargo/config.toml`). CI, the pre-commit hook and the docs all call these
//! commands, so the checks cannot drift apart.

mod demo;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

  lint           cargo fmt --check and clippy with warnings denied
  check          lint, test, and import the demo board headlessly (what CI runs)
  check-windows  clippy for the Windows target from here, warnings denied: the
                 Windows-only code type-checked without a Windows machine
                 (needs `rustup target add x86_64-pc-windows-msvc`, clang and
                 llvm-ar; nothing is linked or run)
  demo [DIR]     write a demo sound pack and board to DIR (default target/demo)
                 and import it into DIR/library
  previews [DIR] render 30 s of each Generator style to a WAV in DIR
                 (default target/previews), to listen to without the app
  instruments fetch|check|build [--sources DIR] [--out DIR]
                 the Generator's instrument pack: fetch the recordings it is
                 made from (default target/instrument-sources), check their
                 pitches, build the pack and its zip (default
                 target/instruments)
  run [args...]  build the app and run a copy of it, passing args through:
                   cargo xtask run --library target/demo/library
                   cargo xtask run --screenshot out.png --library target/demo/library
";

type Result<T = ()> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("lint") => lint(),
        Some("check") => check(),
        Some("check-windows") => check_windows(),
        Some("demo") => {
            let dir = args
                .get(1)
                .map_or_else(|| target_dir().join("demo"), PathBuf::from);
            build_app_copy().and_then(|app| demo_library(&app, &dir))
        }
        Some("previews") => {
            let dir = args
                .get(1)
                .map_or_else(|| target_dir().join("previews"), PathBuf::from);
            build_app_copy().and_then(|app| {
                run(
                    &app,
                    &[
                        "generate".to_owned(),
                        "--out".to_owned(),
                        dir.display().to_string(),
                    ],
                )
            })
        }
        Some("run") => build_app_copy().and_then(|app| run(&app, &args[1..])),
        Some("instruments") => {
            let mut command = vec!["run", "--release", "--quiet", "-p", "olio-packer", "--"];
            command.extend(args[1..].iter().map(String::as_str));
            cargo(&command)
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask: {err}");
            ExitCode::FAILURE
        }
    }
}

fn lint() -> Result {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])
}

fn check() -> Result {
    lint()?;
    cargo(&["test", "--workspace"])?;
    // The import path end to end, through the real binary, without a window.
    let app = build_app_copy()?;
    demo_library(&app, &target_dir().join("xtask-check"))
}

const WINDOWS_TARGET: &str = "x86_64-pc-windows-msvc";

/// Clippy for the Windows target, from any host: the Windows-only code
/// (the music model's process flags, the installer's zip path, WASAPI)
/// type-checked and linted without linking or running anything. CI's
/// Windows run is the real one; this catches what it would, sooner.
///
/// Two dependencies compile C in their build scripts. blake3 is told to use
/// its pure-Rust path; ring's C is compiled with clang aimed at Windows
/// against a few stub headers written to `target/windows-check/`, since
/// there is no Windows SDK here. Neither object is ever linked.
fn check_windows() -> Result {
    let clippy = [
        "clippy",
        "--workspace",
        "--all-targets",
        "--target",
        WINDOWS_TARGET,
        "--",
        "-D",
        "warnings",
    ];
    if cfg!(windows) {
        return cargo(&clippy);
    }
    let installed = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map_err(|err| format!("could not run rustup: {err}"))?;
    let installed = String::from_utf8_lossy(&installed.stdout);
    if !installed.lines().any(|line| line.trim() == WINDOWS_TARGET) {
        return Err(format!(
            "the {WINDOWS_TARGET} target is not installed; run `rustup target add {WINDOWS_TARGET}`"
        ));
    }
    let ar = llvm_ar().ok_or("llvm-ar not found; install LLVM (brew install llvm)")?;
    let include = write_stub_headers()?;
    if include.to_string_lossy().contains(' ') {
        return Err(format!(
            "{} has a space in its path, which the C compiler's arguments cannot carry; \
             set CARGO_TARGET_DIR to a path without one",
            include.display()
        ));
    }
    let cc = format!(
        "clang --target={WINDOWS_TARGET} -nostdlibinc -isystem {}",
        include.display()
    );
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    run_with_env(
        Path::new(&cargo),
        &clippy,
        &[
            ("CARGO_FEATURE_PURE", "1".to_owned()),
            ("CC_x86_64_pc_windows_msvc", cc),
            ("AR_x86_64_pc_windows_msvc", ar.display().to_string()),
        ],
    )
}

/// LLVM's archiver, which can make a Windows static library on any host:
/// Homebrew's LLVM, or one on the PATH.
fn llvm_ar() -> Option<PathBuf> {
    let homebrew = [
        "/opt/homebrew/opt/llvm/bin/llvm-ar",
        "/usr/local/opt/llvm/bin/llvm-ar",
    ];
    homebrew
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .or_else(|| {
            Command::new("llvm-ar")
                .arg("--version")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|_| PathBuf::from("llvm-ar"))
        })
}

/// The few C headers ring's sources include, as far as compiling (not
/// linking) needs them, in `target/windows-check/include`.
fn write_stub_headers() -> Result<PathBuf> {
    const HEADERS: [(&str, &str); 6] = [
        ("assert.h", "#pragma once\n#define assert(x) ((void)0)\n"),
        (
            "string.h",
            "#pragma once\n#include <stddef.h>\n\
             void *memcpy(void *, const void *, size_t);\n\
             void *memset(void *, int, size_t);\n\
             int memcmp(const void *, const void *, size_t);\n\
             void *memmove(void *, const void *, size_t);\n\
             size_t strlen(const char *);\n",
        ),
        (
            "stdlib.h",
            "#pragma once\n#include <stddef.h>\n\
             void abort(void);\n\
             void *malloc(size_t);\n\
             void free(void *);\n\
             unsigned short _byteswap_ushort(unsigned short);\n\
             unsigned long _byteswap_ulong(unsigned long);\n\
             unsigned long long _byteswap_uint64(unsigned long long);\n",
        ),
        ("stdio.h", "#pragma once\n"),
        (
            "limits.h",
            "#pragma once\n\
             #define INT_MAX 2147483647\n\
             #define UINT_MAX 4294967295U\n\
             #define SIZE_MAX 18446744073709551615ULL\n",
        ),
        ("intrin.h", "#pragma once\n#include <stdlib.h>\n"),
    ];
    let dir = target_dir().join("windows-check").join("include");
    fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    for (name, text) in HEADERS {
        let path = dir.join(name);
        fs::write(&path, text).map_err(|err| format!("{}: {err}", path.display()))?;
    }
    Ok(dir)
}

/// Writes the demo pack to `dir` and imports its board into a fresh
/// `dir/library` with `olio import`.
fn demo_library(app: &Path, dir: &Path) -> Result {
    let sounds = dir.join("sounds");
    let library = dir.join("library");
    if library.exists() {
        fs::remove_dir_all(&library).map_err(|err| format!("{}: {err}", library.display()))?;
    }
    let board = demo::write(&sounds)?;
    run(
        app,
        &[
            "import".to_owned(),
            "--library".to_owned(),
            library.display().to_string(),
            board.display().to_string(),
        ],
    )?;
    eprintln!(
        "demo library ready; try:\n  cargo xtask run --library {}",
        library.display()
    );
    Ok(())
}

/// Builds the app and copies the binary to `target/run/`, returning the copy.
///
/// On Windows a running executable is locked, so running the build output
/// directly makes the next build fail to link while the window is open.
/// Running a copy leaves `target/debug` free to rebuild.
fn build_app_copy() -> Result<PathBuf> {
    cargo(&["build", "--package", "olio-app"])?;
    let exe = format!("olio{}", env::consts::EXE_SUFFIX);
    let built = target_dir().join("debug").join(&exe);
    let run_dir = target_dir().join("run");
    fs::create_dir_all(&run_dir).map_err(|err| format!("{}: {err}", run_dir.display()))?;
    // Copies that are still running cannot be removed; leave them and take
    // the first free name.
    if let Ok(entries) = fs::read_dir(&run_dir) {
        for entry in entries.flatten() {
            let _ = fs::remove_file(entry.path());
        }
    }
    let copy = (0..)
        .map(|n| match n {
            0 => run_dir.join(&exe),
            n => run_dir.join(format!("olio-{n}{}", env::consts::EXE_SUFFIX)),
        })
        .find(|path| !path.exists())
        .expect("an unbounded range always has a free name");
    fs::copy(&built, &copy).map_err(|err| format!("copying {}: {err}", built.display()))?;
    Ok(copy)
}

fn cargo(args: &[&str]) -> Result {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    run(Path::new(&cargo), args)
}

fn run(program: &Path, args: &[impl AsRef<str>]) -> Result {
    run_with_env(program, args, &[])
}

fn run_with_env(program: &Path, args: &[impl AsRef<str>], env: &[(&str, String)]) -> Result {
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let name = program.file_stem().unwrap_or_default().to_string_lossy();
    eprintln!("$ {name} {}", args.join(" "));
    let status = Command::new(program)
        .args(&args)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .current_dir(root())
        .status()
        .map_err(|err| format!("could not start {}: {err}", program.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{name} {}` failed ({status})", args.join(" ")))
    }
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
}

fn target_dir() -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target"))
}

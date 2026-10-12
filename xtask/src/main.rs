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
  demo [DIR]     write a demo sound pack and board to DIR (default target/demo)
                 and import it into DIR/library
  previews [DIR] render 30 s of each Generator style to a WAV in DIR
                 (default target/previews), to listen to without the app
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
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let name = program.file_stem().unwrap_or_default().to_string_lossy();
    eprintln!("$ {name} {}", args.join(" "));
    let status = Command::new(program)
        .args(&args)
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

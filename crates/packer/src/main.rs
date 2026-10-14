//! Builds Olio's instrument pack: recorded instruments for the Generator
//! from the Versilian Studios CC0 libraries (`sources.json`).
//!
//! ```text
//! olio-packer fetch --sources DIR          download the listed recordings
//! olio-packer check --sources DIR          measure their pitches
//! olio-packer build --sources DIR --out DIR
//! ```
//!
//! `cargo xtask instruments` runs it with the defaults under `target/`.

mod build;
mod fetch;
mod pitch;
mod recipes;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: olio-packer fetch|check|build [--sources DIR] [--out DIR]\n";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let mut sources = PathBuf::from("target/instrument-sources");
    let mut out = PathBuf::from("target/instruments");
    while let Some(arg) = args.next() {
        match (arg.as_str(), args.next()) {
            ("--sources", Some(dir)) => sources = PathBuf::from(dir),
            ("--out", Some(dir)) => out = PathBuf::from(dir),
            _ => {
                eprint!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let result = match command.as_deref() {
        Some("fetch") => fetch::fetch(&sources),
        Some("check") => build::check(&sources),
        Some("build") => build::build(&sources, &out),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("olio-packer: {err}");
            ExitCode::FAILURE
        }
    }
}

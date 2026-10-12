//! Olio desktop application: the window, the importer and the command line.
//! `src/main.rs` is only the entry point.
//!
//! `olio [--library DIR] [--screenshot out.png] [PATH...]` opens the
//! soundboard on the library in `DIR` (default: `$OLIO_LIBRARY`, or the
//! per-user data folder), importing any sounds, folders or boards given.
//! `--screenshot` saves a PNG of the window once it has drawn and exits.
//!
//! `olio import [--library DIR] [--set NAME] [--force] PATH...` imports
//! without opening a window, for scripts (see `docs/BOARD-JSON.md`).
//!
//! `olio generate [--style NAME|all] [--seconds N] [--seed N] [--out DIR]`
//! renders the Generator's styles to WAV files, without a window.

pub mod app;
pub mod cli;
mod clips;
mod confirm;
mod emoji;
mod fonts;
mod format;
mod generator;
mod grid;
pub mod import;
mod inspector;
mod keys;
mod menu;
mod meter;
mod palette;
mod polish;
pub mod prefs;
mod sidebar;
mod undo;

use std::path::PathBuf;

use olio_library::Store;

pub use app::OlioApp;

pub const USAGE: &str = "\
usage: olio [--library DIR] [--set NAME] [--screenshot OUT.png] [PATH...]
       olio import [--library DIR] [--set NAME] [--force] PATH...
       olio generate [--style NAME|all] [--seconds N] [--seed N] [--out DIR]

  --library DIR       use the library in DIR (default: $OLIO_LIBRARY, or the
                      per-user data folder)
  --set NAME          open on the set with this name
  --screenshot FILE   save a PNG of the window once it has drawn, then quit
  PATH...             sounds to add to the front set, folders to add as sets
                      (sounds in alphabetical order), and boards (.json)

  import              import without opening a window, then exit; 1 if
                      anything could not be imported
  --set NAME          put loose sounds in this set, creating it if needed
                      (default: the first set)
  --force             import even though Olio seems to have the library open

  generate            render the Generator's styles to WAV files, then exit
  --style NAME        one style, such as \"Rain Study\" (default: all of them)
  --seconds N         how long each track is, 3 to 600 (default 30)
  --seed N            another variation of the same style
  --out DIR           where the files go (default: previews)
";

/// What the command line asked for.
pub struct Options {
    pub library: PathBuf,
    /// Open no output device: the mix is rendered and thrown away in step
    /// with the window's clock, so playback, progress and meters behave as
    /// with a device but nothing is heard. For tests.
    pub silent: bool,
    pub set: Option<String>,
    pub screenshot: Option<PathBuf>,
    pub import: Vec<PathBuf>,
}

/// The library directory unless `--library` named one.
pub fn default_library() -> Result<PathBuf, String> {
    std::env::var_os("OLIO_LIBRARY")
        .map(PathBuf::from)
        .or_else(Store::default_root)
        .ok_or_else(|| "no per-user data folder on this system; pass --library DIR".to_owned())
}

/// `Ok(None)` means help was asked for.
pub fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut library = None;
    let mut set = None;
    let mut screenshot = None;
    let mut import = Vec::new();
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--library" => {
                library = Some(PathBuf::from(
                    args.next().ok_or("--library needs a directory")?,
                ));
            }
            "--set" => set = Some(args.next().ok_or("--set needs a set name")?),
            "--screenshot" => {
                screenshot = Some(PathBuf::from(
                    args.next().ok_or("--screenshot needs a file name")?,
                ));
            }
            option if option.starts_with('-') => return Err(format!("unknown option {option}")),
            file => import.push(PathBuf::from(file)),
        }
    }
    let library = match library {
        Some(library) => library,
        None => default_library()?,
    };
    Ok(Some(Options {
        library,
        silent: false,
        set,
        screenshot,
        import,
    }))
}

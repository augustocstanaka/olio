//! `olio import`: importing from a script, without a window.

use std::path::PathBuf;

use olio_library::{Set, Store};

use crate::import::Importer;
use crate::{USAGE, default_library};

/// Runs `olio import` with the arguments after `import`; returns the exit
/// code.
pub fn import(args: impl Iterator<Item = String>) -> i32 {
    match run(args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("olio import: {message}");
            2
        }
    }
}

fn run(mut args: impl Iterator<Item = String>) -> Result<i32, String> {
    let mut library = None;
    let mut set_name = None;
    let mut force = false;
    let mut paths = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(0);
            }
            "--library" => {
                library = Some(PathBuf::from(
                    args.next().ok_or("--library needs a directory")?,
                ));
            }
            "--set" => set_name = Some(args.next().ok_or("--set needs a name")?),
            "--force" => force = true,
            option if option.starts_with('-') => return Err(format!("unknown option {option}")),
            path => paths.push(PathBuf::from(path)),
        }
    }
    if paths.is_empty() {
        return Err("nothing to import; give sounds, folders or .json boards".to_owned());
    }
    let library_dir = match library {
        Some(library) => library,
        None => default_library()?,
    };
    let store = Store::open(&library_dir).map_err(|err| err.to_string())?;
    if let Some(lock) = store.lock_holder()
        && !force
    {
        return Err(format!(
            "Olio has this library open, and would overwrite the import when it next saves.\n\
             Quit Olio and run this again, or pass --force if Olio is not running \
             (then {} is left over from a crash).",
            lock.display()
        ));
    }
    let mut library = store.load().map_err(|err| err.to_string())?;
    if library.sets.is_empty() {
        library.sets.push(Set::new("My Sounds"));
    }
    let target = match &set_name {
        Some(name) => match library.sets.iter().find(|set| &set.name == name) {
            Some(set) => set.id,
            None => {
                let set = Set::new(name.clone());
                let id = set.id;
                library.sets.push(set);
                id
            }
        },
        None => library.sets[0].id,
    };

    let report = Importer::new(&store, &mut library).paths(&paths, target, None);
    store.save(&library).map_err(|err| err.to_string())?;

    let sounds = report.tiles;
    let sets = report.sets.len();
    println!(
        "imported {sounds} sound{} ({sets} set{} added or replaced) into {}",
        if sounds == 1 { "" } else { "s" },
        if sets == 1 { "" } else { "s" },
        library_dir.display()
    );
    for problem in &report.problems {
        eprintln!("problem: {problem}");
    }
    Ok(if report.problems.is_empty() { 0 } else { 1 })
}

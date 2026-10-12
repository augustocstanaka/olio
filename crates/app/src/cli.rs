//! The commands that run without a window: `olio import`, importing from
//! a script, and `olio generate`, rendering the Generator's styles to WAV
//! files.

use std::path::PathBuf;

use olio_audio::ambience::Style;
use olio_audio::{WavFormat, render_track};
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

/// Runs `olio generate` with the arguments after `generate`; returns the
/// exit code.
pub fn generate(args: impl Iterator<Item = String>) -> i32 {
    match run_generate(args) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("olio generate: {message}");
            2
        }
    }
}

fn run_generate(mut args: impl Iterator<Item = String>) -> Result<(), String> {
    let mut styles: Vec<Style> = Style::ALL.to_vec();
    let mut seconds = 30.0f32;
    let mut seed = None;
    let mut out = PathBuf::from("previews");
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "--style" => {
                let name = args.next().ok_or("--style needs a style name, or all")?;
                if !name.eq_ignore_ascii_case("all") {
                    let style = Style::ALL
                        .into_iter()
                        .find(|style| style.name().eq_ignore_ascii_case(&name))
                        .ok_or_else(|| {
                            let names: Vec<&str> = Style::ALL.iter().map(|s| s.name()).collect();
                            format!("no style called “{name}”; try {}", names.join(", "))
                        })?;
                    styles = vec![style];
                }
            }
            "--seconds" => {
                seconds = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|value: &f32| (3.0..=600.0).contains(value))
                    .ok_or("--seconds needs a number from 3 to 600")?;
            }
            "--seed" => {
                seed = Some(
                    args.next()
                        .and_then(|value| value.parse::<u64>().ok())
                        .ok_or("--seed needs a whole number")?,
                );
            }
            "--out" => out = PathBuf::from(args.next().ok_or("--out needs a directory")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    std::fs::create_dir_all(&out).map_err(|err| format!("{}: {err}", out.display()))?;
    for style in styles {
        let mut settings = style.settings();
        if let Some(seed) = seed {
            settings.seed = seed;
        }
        let clip = render_track(&settings, seconds, |_| true).ok_or("the render stopped")?;
        let path = out.join(format!("{}.wav", style.name()));
        clip.write_wav(&path, WavFormat::Int16)
            .map_err(|err| format!("{}: {err}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_writes_a_wav_for_a_style() {
        let dir = tempfile::tempdir().unwrap();
        let args = [
            "--style",
            "campfire",
            "--seconds",
            "4",
            "--out",
            dir.path().to_str().unwrap(),
        ];
        assert_eq!(generate(args.iter().map(|arg| (*arg).to_owned())), 0);
        let clip = olio_audio::decode_file(&dir.path().join("Campfire.wav")).unwrap();
        assert_eq!(clip.channels(), 2);
        assert_eq!(clip.frames(), 4 * olio_audio::TRACK_RATE as usize);

        let unknown = ["--style", "Polka"].iter().map(|arg| (*arg).to_owned());
        assert_eq!(generate(unknown), 2);
    }
}

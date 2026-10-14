//! The commands that run without a window: `olio import`, importing from
//! a script, and `olio generate`, rendering the Generator's styles to WAV
//! files.

use std::path::PathBuf;
use std::sync::Arc;

use olio_audio::ambience::{Journey, Style};
use olio_audio::instruments::Recorded;

use crate::instruments;
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

/// Runs `olio instruments` with the arguments after `instruments`:
/// downloads the Generator's recorded instruments and installs them, or
/// installs a pack built here (`--from ZIP`). Returns the exit code.
pub fn install_instruments(args: impl Iterator<Item = String>) -> i32 {
    match run_install_instruments(args) {
        Ok(dir) => {
            println!("recorded instruments installed in {}", dir.display());
            0
        }
        Err(message) => {
            eprintln!("olio instruments: {message}");
            2
        }
    }
}

fn run_install_instruments(mut args: impl Iterator<Item = String>) -> Result<PathBuf, String> {
    let mut from = None;
    let mut dir = instruments::default_dir();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--from" => from = Some(PathBuf::from(args.next().ok_or("--from needs a zip")?)),
            "--dir" => dir = Some(PathBuf::from(args.next().ok_or("--dir needs a directory")?)),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let dir = dir.ok_or("no per-user data folder on this system; pass --dir DIR")?;
    match from {
        Some(zip) => instruments::install_zip(&zip, None, &dir)?,
        None => {
            let mut shown = 0;
            let finished = instruments::download_and_install(&dir, |done| {
                let tenth = (done * 10.0) as u32;
                if tenth > shown {
                    shown = tenth;
                    println!("{} %", tenth * 10);
                }
                true
            })?;
            if !finished {
                return Err("the download stopped".to_owned());
            }
        }
    }
    Ok(dir)
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
    // Variety, concentration, chaos and energy, in percent, when asked for.
    let mut character: [Option<f32>; 4] = [None; 4];
    let mut journey = None;
    let mut synthesised = false;
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
            "--variety" | "--concentration" | "--chaos" | "--energy" => {
                let index = match arg.as_str() {
                    "--variety" => 0,
                    "--concentration" => 1,
                    "--chaos" => 2,
                    _ => 3,
                };
                character[index] = Some(
                    args.next()
                        .and_then(|value| value.parse().ok())
                        .filter(|value: &f32| (0.0..=100.0).contains(value))
                        .ok_or_else(|| format!("{arg} needs a percentage from 0 to 100"))?,
                );
            }
            "--journey" => {
                let name = args.next().unwrap_or_default().to_lowercase();
                journey = Some(match name.as_str() {
                    "steady" => Journey::Steady,
                    "build" | "builds up" => Journey::Build,
                    "evolve" | "evolves" => Journey::Evolve,
                    _ => return Err("--journey needs steady, build or evolve".to_owned()),
                });
            }
            "--synthesised" => synthesised = true,
            "--out" => out = PathBuf::from(args.next().ok_or("--out needs a directory")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    std::fs::create_dir_all(&out).map_err(|err| format!("{}: {err}", out.display()))?;
    // The recorded instruments where installed, loaded as styles need them.
    let pack = if synthesised {
        None
    } else {
        instruments::default_dir().and_then(|dir| instruments::installed(&dir))
    };
    let mut recorded = Recorded::default();
    for style in styles {
        let mut settings = style.settings();
        if let Some(seed) = seed {
            settings.seed = seed;
        }
        // Named for what was asked: "Campfire (chaos 90).wav".
        let mut asked = Vec::new();
        let fields = [
            ("variety", &mut settings.variety),
            ("concentration", &mut settings.concentration),
            ("chaos", &mut settings.chaos),
            ("energy", &mut settings.energy),
        ];
        for ((name, field), percent) in fields.into_iter().zip(character) {
            if let Some(percent) = percent {
                *field = percent / 100.0;
                asked.push(format!("{name} {percent}"));
            }
        }
        if let Some(journey) = journey {
            settings.journey = journey;
            asked.push(journey.name().to_lowercase());
        }
        if synthesised {
            asked.push("synthesised".to_owned());
        }
        let name = match asked.is_empty() {
            true => style.name().to_owned(),
            false => format!("{} ({})", style.name(), asked.join(", ")),
        };
        if let Some(pack) = &pack {
            for instrument in settings.playing() {
                if recorded.get(instrument).is_none()
                    && pack.instruments().any(|i| i == instrument)
                    && let Some(samples) = pack.load(instrument)?
                {
                    recorded.set(instrument, Some(Arc::new(samples)));
                }
            }
        }
        let clip =
            render_track(&settings, seconds, &recorded, |_| true).ok_or("the render stopped")?;
        let path = out.join(format!("{name}.wav"));
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

        let args = [
            "--style",
            "campfire",
            "--seconds",
            "3",
            "--variety",
            "20",
            "--chaos",
            "90",
            "--energy",
            "90",
            "--journey",
            "evolve",
            "--out",
            dir.path().to_str().unwrap(),
        ];
        assert_eq!(generate(args.iter().map(|arg| (*arg).to_owned())), 0);
        assert!(
            dir.path()
                .join("Campfire (variety 20, chaos 90, energy 90, evolves).wav")
                .exists()
        );

        let unknown = ["--style", "Polka"].iter().map(|arg| (*arg).to_owned());
        assert_eq!(generate(unknown), 2);
        let wild = ["--chaos", "150"].iter().map(|arg| (*arg).to_owned());
        assert_eq!(generate(wild), 2);
        let wild = ["--energy", "200"].iter().map(|arg| (*arg).to_owned());
        assert_eq!(generate(wild), 2);
        let wandering = ["--journey", "wander"].iter().map(|arg| (*arg).to_owned());
        assert_eq!(generate(wandering), 2);
    }
}

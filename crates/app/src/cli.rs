//! The commands that run without a window: `olio import`, importing from
//! a script, `olio generate`, rendering the Generator's styles (or a
//! prompt, with the music model) to WAV files, `olio instruments` and
//! `olio music-model`, installing what the Generator can download.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use olio_audio::ambience::{Journey, Style};
use olio_audio::instruments::Recorded;

use crate::instruments;
use crate::music_model::client::{Client, Request};
use crate::music_model::install::{Sources, Sync};
use crate::music_model::server::{START_TIMEOUT, Server};
use crate::{generator, music_model};
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

/// Runs `olio music-model` with the arguments after `music-model`:
/// `install` or `remove`, each with `--dir DIR`. Returns the exit code.
pub fn music_model(args: impl Iterator<Item = String>) -> i32 {
    match run_music_model(args) {
        Ok(message) => {
            println!("{message}");
            0
        }
        Err(message) => {
            eprintln!("olio music-model: {message}");
            2
        }
    }
}

fn run_music_model(mut args: impl Iterator<Item = String>) -> Result<String, String> {
    let mut verb = None;
    let mut dir = music_model::default_dir();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--dir" => dir = Some(PathBuf::from(args.next().ok_or("--dir needs a directory")?)),
            "install" | "remove" if verb.is_none() => verb = Some(arg),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let dir = dir.ok_or("no per-user data folder on this system; pass --dir DIR")?;
    match verb.as_deref() {
        Some("install") => {
            install_music_model(&dir)?;
            Ok(format!("music model installed in {}", dir.display()))
        }
        Some("remove") => {
            if music_model::installed(&dir).is_none() && !dir.exists() {
                return Err(format!("nothing is installed in {}", dir.display()));
            }
            music_model::remove(&dir)?;
            Ok(format!("music model removed from {}", dir.display()))
        }
        _ => Err("say install or remove".to_owned()),
    }
}

/// Installs the music model into `dir`, printing each phase and every
/// tenth of the way through it.
fn install_music_model(dir: &Path) -> Result<(), String> {
    let mut last = None;
    let mut shown = 0;
    let finished =
        music_model::install::run(dir, &Sources::published(), Sync::Uv, |phase, done| {
            if last != Some(phase) {
                last = Some(phase);
                shown = 0;
                println!("{}…", phase.words());
            }
            if let Some(done) = done {
                let tenth = (done * 10.0) as u32;
                if tenth > shown {
                    shown = tenth;
                    println!("  {} %", tenth * 10);
                }
            }
            true
        })
        .map_err(|err| {
            format!(
                "{err}\n(the log is {})",
                music_model::Layout::new(dir).install_log().display()
            )
        })?;
    if !finished {
        return Err("the install stopped".to_owned());
    }
    Ok(())
}

/// Where `olio generate --prompt` finds the music model.
enum Model {
    /// One already answering at this address (`OLIO_MUSIC_MODEL_URL`).
    At(String),
    /// Installed in this folder: started for the track and stopped after.
    In(PathBuf),
}

impl Model {
    fn from_env() -> Result<Self, String> {
        if let Some(url) = music_model::url_override() {
            return Ok(Self::At(url));
        }
        music_model::default_dir()
            .map(Self::In)
            .ok_or_else(|| "no per-user data folder on this system".to_owned())
    }
}

/// Makes `seconds` of music from `prompt` with the music model and writes
/// it under `out`, named after the prompt.
fn generate_prompted(
    prompt: &str,
    seconds: u32,
    seed: Option<u64>,
    out: &Path,
    model: Model,
) -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    let (client, mut server) = match model {
        Model::At(url) => (Client::new(url, None), None),
        Model::In(dir) => {
            if music_model::installed(&dir).is_none() {
                return Err(
                    "the music model is not installed; run `olio music-model install`".to_owned(),
                );
            }
            let server = Server::start(&dir)?;
            println!("starting the music model…");
            (server.client(), Some(server))
        }
    };
    if !client.wait_healthy(START_TIMEOUT, &cancel)? {
        return Err("stopped".to_owned());
    }
    let title = generator::prompt_track_title(prompt, seconds);
    let path = out.join(format!("{title}.wav"));
    let request = Request {
        prompt: prompt.to_owned(),
        seconds,
        seed,
    };
    print!("generating “{title}”");
    let _ = std::io::stdout().flush();
    // A dot every five seconds while the model works, and the first time it
    // loads too.
    let done = Arc::new(AtomicBool::new(false));
    let ticking = Arc::clone(&done);
    let ticker = std::thread::spawn(move || {
        let mut waited = 0;
        while !ticking.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
            waited += 1;
            if waited % 50 == 0 {
                print!(".");
                let _ = std::io::stdout().flush();
            }
        }
    });
    let made = client.generate(&request, &path, &cancel);
    done.store(true, Ordering::Relaxed);
    let _ = ticker.join();
    println!();
    if let Some(server) = &mut server {
        server.stop();
    }
    made?.ok_or("stopped")?;
    println!("wrote {}", path.display());
    Ok(())
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
    let mut prompt: Option<String> = None;
    // Whether any of the style options was given: none applies to a prompt.
    let mut styled = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "--prompt" => {
                let text = args
                    .next()
                    .ok_or("--prompt needs the words to make music from")?;
                if text.trim().is_empty() {
                    return Err("--prompt needs some words".to_owned());
                }
                prompt = Some(text.trim().to_owned());
            }
            "--style" => {
                styled = true;
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
                styled = true;
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
                styled = true;
                let name = args.next().unwrap_or_default().to_lowercase();
                journey = Some(match name.as_str() {
                    "steady" => Journey::Steady,
                    "build" | "builds up" => Journey::Build,
                    "evolve" | "evolves" => Journey::Evolve,
                    _ => return Err("--journey needs steady, build or evolve".to_owned()),
                });
            }
            "--synthesised" => {
                styled = true;
                synthesised = true;
            }
            "--out" => out = PathBuf::from(args.next().ok_or("--out needs a directory")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    std::fs::create_dir_all(&out).map_err(|err| format!("{}: {err}", out.display()))?;
    if let Some(prompt) = prompt {
        if styled {
            return Err(
                "--prompt makes a track with the music model; the style options do not apply"
                    .to_owned(),
            );
        }
        if seconds < 10.0 {
            return Err("--seconds needs a number from 10 to 600 for a prompt".to_owned());
        }
        return generate_prompted(&prompt, seconds as u32, seed, &out, Model::from_env()?);
    }
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
    fn a_prompt_writes_a_wav_named_after_it_from_the_music_model() {
        use crate::music_model::client::fake::{Fake, Settings};
        let fake = Fake::start(Settings {
            audio: crate::music_model::client::tests::wav(),
            ..Settings::default()
        });
        let dir = tempfile::tempdir().unwrap();
        generate_prompted(
            "soft piano and rain",
            10,
            Some(7),
            dir.path(),
            Model::At(fake.base.clone()),
        )
        .unwrap();
        let clip =
            olio_audio::decode_file(&dir.path().join("Soft piano and rain 10 s.wav")).unwrap();
        assert_eq!(clip.channels(), 2);
        let bodies = fake.bodies();
        assert_eq!(bodies[0]["prompt"], "soft piano and rain");
        assert_eq!(bodies[0]["audio_duration"], 10);
        assert_eq!(bodies[0]["seed"], 7);
    }

    #[test]
    fn a_prompt_needs_the_music_model_installed() {
        let dir = tempfile::tempdir().unwrap();
        let err = generate_prompted(
            "soft piano",
            10,
            None,
            dir.path(),
            Model::In(dir.path().join("MusicModel")),
        )
        .unwrap_err();
        assert!(err.contains("olio music-model install"), "{err}");
    }

    #[test]
    fn a_prompt_takes_no_style_options_and_at_least_ten_seconds() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap();
        let args = [
            "--prompt",
            "soft piano",
            "--style",
            "campfire",
            "--out",
            out,
        ];
        assert_eq!(generate(args.iter().map(|arg| (*arg).to_owned())), 2);
        let args = ["--prompt", "soft piano", "--chaos", "50", "--out", out];
        assert_eq!(generate(args.iter().map(|arg| (*arg).to_owned())), 2);
        let args = ["--prompt", "soft piano", "--seconds", "5", "--out", out];
        assert_eq!(generate(args.iter().map(|arg| (*arg).to_owned())), 2);
        let args = ["--prompt", "  ", "--out", out];
        assert_eq!(generate(args.iter().map(|arg| (*arg).to_owned())), 2);
    }

    #[test]
    fn music_model_wants_install_or_remove() {
        let dir = tempfile::tempdir().unwrap();
        let args = ["--dir", dir.path().to_str().unwrap()];
        assert_eq!(music_model(args.iter().map(|arg| (*arg).to_owned())), 2);
        let none = dir.path().join("none");
        let args = ["remove", "--dir", none.to_str().unwrap()];
        assert_eq!(music_model(args.iter().map(|arg| (*arg).to_owned())), 2);
        // Removing what is there, even half installed, is fine.
        let half = dir.path().join("half");
        std::fs::create_dir_all(half.join("uv")).unwrap();
        let args = ["remove", "--dir", half.to_str().unwrap()];
        assert_eq!(music_model(args.iter().map(|arg| (*arg).to_owned())), 0);
        assert!(!half.exists());
    }

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

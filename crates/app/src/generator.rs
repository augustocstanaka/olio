//! The Generator: a panel in place of the inspector where the ambience is
//! shaped, played live under the sounds, and rendered to tracks that land
//! on tiles. It shows the essentials (style, key, mood, energy, how it
//! changes over time, background and its balance with the instruments,
//! instruments), each in a section that opens and closes; a section's
//! More adds what refines it (the character's amounts, the form, each
//! instrument's level, density and register). Which sections are open is
//! kept in the preferences. Every control explains itself on hover.
//! Custom settings can be saved as a style of one's own, listed after the
//! built-in styles.
//!
//! The settings live in the preferences and go to the engine when they
//! change. A track is rendered on a thread of its own ([`Generation`]),
//! written as a WAV into the library and placed on a tile by the app. With
//! the music model installed, a track can be made from a prompt instead
//! (From a prompt): the same thread asks the model for it, and can render
//! a bed, the Generator's background alone, quietly, to loop on a second
//! tile under it.

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use egui::{Align, Color32, Key, Layout, Ui};
use olio_audio::ambience::{
    AmbienceSettings, Background, Density, Form, Instrument, Journey, Mood, Now, Presence,
    ROOT_NAMES, Register, ScaleKind, Style,
};
use olio_audio::instruments::{Pack, Recorded as Recordings};
use olio_audio::{Clip, WavFormat, render_track};
use olio_library::{Sound, Store};

use crate::emoji::EmojiImages;
use crate::instruments::{PACK_BYTES, megabytes};
use crate::music_model::client::{Client, Request};
use crate::music_model::server::{START_TIMEOUT, Server};
use crate::music_model::{DISK_BYTES, about_gigabytes};
use crate::prefs::{Disclosure, UserStyle, style_name_problem, style_named};
use crate::skin;

/// The lengths a track can be, in seconds, and how they read.
pub const TRACK_LENGTHS: [(u32, &str); 5] = [
    (30, "30 s"),
    (60, "1 min"),
    (120, "2 min"),
    (300, "5 min"),
    (600, "10 min"),
];

/// The length a track is unless another was picked.
pub const DEFAULT_TRACK_SECONDS: u32 = 120;

/// How a length reads: "2 min".
pub fn length_name(seconds: u32) -> String {
    match TRACK_LENGTHS.iter().find(|(length, _)| *length == seconds) {
        Some((_, name)) => (*name).to_owned(),
        None if seconds.is_multiple_of(60) => format!("{} min", seconds / 60),
        None => format!("{seconds} s"),
    }
}

/// The emoji for a background, for the toolbar bubble and a track's tile.
pub fn emoji(background: Background) -> &'static str {
    match background {
        Background::None => "🎵",
        Background::Rain => "🌧️",
        Background::Thunder => "⛈️",
        Background::Wind => "🌬️",
        Background::Winter => "❄️",
        Background::Waves => "🌊",
        Background::Forest => "🌲",
        Background::Desert => "🏜️",
        Background::Fire => "🔥",
    }
}

/// The character in words: "Comes back to a motif now and then, around
/// the chord, in the key."
pub fn character_words(settings: &AmbienceSettings) -> String {
    let variety = match settings.variety {
        v if v < 0.2 => "Repeats a short motif",
        v if v < 0.45 => "Comes back to a motif often",
        v if v < 0.7 => "Comes back to a motif now and then",
        _ => "Always finding new melodies",
    };
    let concentration = match settings.concentration {
        c if c < 0.3 => "wandering wide",
        c if c < 0.7 => "around the chord",
        _ => "close to the home note",
    };
    let chaos = match settings.chaos {
        c if c <= 0.0 => "in the key",
        c if c < 0.35 => "a note outside the key now and then",
        c if c < 0.7 => "with clashes and notes outside the key",
        _ => "dissonant and unsettled",
    };
    format!("{variety}, {concentration}, {chaos}.")
}

/// `text` as a sentence: capitalised, with a full stop.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let first = chars.next().map(|c| c.to_uppercase().to_string());
    format!("{}{}.", first.unwrap_or_default(), chars.as_str())
}

/// Where a building or evolving ambience has got to: "Now: E Dorian,
/// thunder; the harp resting."
pub fn now_words(now: &Now) -> String {
    let mut words = format!("Now: {}", now.scale.name());
    if now.background != Background::None {
        words.push_str(&format!(", {}", now.background.name().to_lowercase()));
    }
    if now.building {
        words.push_str("; building up");
    } else if let Some(instrument) = now.resting {
        words.push_str(&format!(
            "; the {} resting",
            instrument.name().to_lowercase()
        ));
    }
    words.push('.');
    words
}

/// A track's title: its name (its style, saved or built-in, or its key
/// when Custom) and its length: "Rain Study 2 min", "D Dorian 30 s".
pub fn track_title(name: &str, seconds: u32) -> String {
    format!("{name} {}", length_name(seconds))
}

/// The emoji on a prompted track's tile.
pub const PROMPT_EMOJI: &str = "🎼";

/// How much of a prompt a track's title keeps.
const TITLE_CHARS: usize = 40;

/// A prompted track's title: the prompt, tidied and cut at a word to
/// about forty characters, and its length: "Warm piano over soft rain 2
/// min". Characters a file name cannot hold become dashes.
pub fn prompt_track_title(prompt: &str, seconds: u32) -> String {
    let words: Vec<&str> = prompt.split_whitespace().collect();
    let mut title = String::new();
    for word in words {
        let next = if title.is_empty() {
            word.to_owned()
        } else {
            format!("{title} {word}")
        };
        if !title.is_empty() && next.chars().count() > TITLE_CHARS {
            break;
        }
        title = next;
    }
    if title.chars().count() > TITLE_CHARS {
        title = title.chars().take(TITLE_CHARS).collect();
    }
    let mut title: String = title
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) { '-' } else { c })
        .collect();
    if let Some(first) = title.chars().next()
        && first.is_lowercase()
    {
        title = first.to_uppercase().chain(title.chars().skip(1)).collect();
    }
    if title.is_empty() {
        title = "Prompt".to_owned();
    }
    track_title(title.trim_end_matches(['.', ',', ' ']), seconds)
}

/// What a prompted track is asked to sound like when Keep it ambient is
/// on: added to the prompt, so the track stays behind the sounds.
pub const PROMPT_HINTS: &str = "ambient, sparse, slow, spacious, soft, no drums";

/// The prompt with the hints after it; the title keeps the words alone.
pub fn hinted(prompt: &str) -> String {
    let prompt = prompt.trim().trim_end_matches([',', '.', ' ']);
    format!("{prompt}, {PROMPT_HINTS}")
}

/// A bed sits this far under the track: -12 dB.
pub const BED_GAIN: f32 = 0.25;

/// A bed's title: "Rain bed 30 s".
pub fn bed_title(background: Background, seconds: u32) -> String {
    track_title(&format!("{} bed", background.name()), seconds)
}

/// The Generator's settings that render a bed: the background alone.
pub fn bed_settings(background: Background, seed: u64) -> AmbienceSettings {
    AmbienceSettings {
        style: None,
        background,
        balance: 0.0,
        seed,
        ..AmbienceSettings::default()
    }
}

/// Renders `seconds` of a bed of `background`, [`BED_GAIN`] under a
/// track; `None` when `progress` stopped it (see [`render_track`]).
pub fn render_bed(
    background: Background,
    seconds: u32,
    seed: u64,
    progress: impl FnMut(f32) -> bool,
) -> Option<Clip> {
    let settings = bed_settings(background, seed);
    let clip = render_track(&settings, seconds as f32, &Recordings::default(), progress)?;
    let samples = clip.samples().iter().map(|s| s * BED_GAIN).collect();
    Some(Clip::new(clip.rate(), clip.channels(), samples))
}

/// A seed for a bed no one picked: a new one each time.
pub fn fresh_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |since| since.as_nanos() as u64 | 1)
}

/// The settings in a line: "G Aeolian, rain; harp and bass."
pub fn summary(settings: &AmbienceSettings) -> String {
    let instruments: Vec<String> = settings
        .playing()
        .map(|instrument| instrument.name().to_lowercase())
        .collect();
    let instruments = match instruments.as_slice() {
        [] => "no instruments".to_owned(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    let background = match settings.background {
        Background::None => "no background".to_owned(),
        background => background.name().to_lowercase(),
    };
    format!("{}, {background}; {instruments}.", settings.scale.name())
}

// ----- generating tracks ----------------------------------------------------

/// One track to render: its title (which becomes its tile's) and settings.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackPlan {
    pub title: String,
    pub settings: AmbienceSettings,
}

/// What a new set of tracks holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetOf {
    /// The settings, several times over, each with other notes.
    Variations,
    /// The settings in each mood.
    Moods,
    /// Every style, built-in and one's own.
    Styles,
}

impl SetOf {
    pub const ALL: [Self; 3] = [Self::Variations, Self::Moods, Self::Styles];

    pub fn name(self) -> &'static str {
        match self {
            Self::Variations => "Variations",
            Self::Moods => "Moods",
            Self::Styles => "Styles",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Variations => "Four tracks of these settings, each with other notes",
            Self::Moods => "These settings in each mood, from Still to Unsettled",
            Self::Styles => "A track of every style, the built-in ones and your own",
        }
    }
}

/// How many tracks a set of variations has.
pub const VARIATIONS: u64 = 4;

/// The name of a new set of tracks and the tracks in it, for `settings`
/// called `name` (its style, or its key when Custom). Each keeps how the
/// music changes over time.
pub fn set_plan(
    of: SetOf,
    settings: &AmbienceSettings,
    name: &str,
    styles: &[UserStyle],
) -> (String, Vec<TrackPlan>) {
    let plan = |title: String, settings: AmbienceSettings| TrackPlan { title, settings };
    match of {
        SetOf::Variations => {
            let tracks = (0..VARIATIONS)
                .map(|n| {
                    let seed = settings.seed.wrapping_add(n);
                    plan(
                        format!("{name} {}", n + 1),
                        AmbienceSettings { seed, ..*settings },
                    )
                })
                .collect();
            (format!("{name} Variations"), tracks)
        }
        SetOf::Moods => {
            let tracks = Mood::ALL
                .into_iter()
                .map(|mood| {
                    let mut settings = *settings;
                    settings.set_mood(mood);
                    plan(format!("{name} {}", mood.name()), settings)
                })
                .collect();
            (format!("{name} Moods"), tracks)
        }
        SetOf::Styles => {
            let journey = settings.journey;
            let built_in = Style::ALL
                .into_iter()
                .map(|style| (style.name().to_owned(), style.settings()));
            let own = styles
                .iter()
                .map(|style| (style.name.clone(), style.settings));
            let tracks = built_in
                .chain(own)
                .map(|(title, settings)| {
                    plan(
                        title,
                        AmbienceSettings {
                            journey,
                            ..settings
                        },
                    )
                })
                .collect();
            ("Styles".to_owned(), tracks)
        }
    }
}

/// Where generated tracks go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Destination {
    /// A tile in this set.
    Tile(olio_library::SetId),
    /// A new set of their own.
    NewSet,
}

/// Why generating stopped before the end.
#[derive(Clone, Debug, PartialEq)]
pub enum Stop {
    Cancelled,
    Failed(String),
}

/// What a generation made: the sounds imported, in the order of its
/// tracks, and why it stopped early, if it did.
#[derive(Debug)]
pub struct Made {
    pub sounds: Vec<Sound>,
    pub stopped: Option<Stop>,
}

/// How far something has got: a generation, or the music model's install.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    /// 0 to 1.
    Fraction(f32),
    /// Unknown: what is going on, and for how long.
    Indeterminate {
        words: &'static str,
        elapsed: Duration,
    },
}

/// Tracks being rendered, one after another, or a prompted track being
/// made by the music model, on a thread of their own.
pub struct Generation {
    /// A track's title, or the new set's name.
    pub title: String,
    pub destination: Destination,
    /// Each track's emoji, for its tile.
    pub emojis: Vec<&'static str>,
    /// How far, as `f32` bits, 0 to 1, over every track; NaN while
    /// unknown (the music model says nothing of how far it is).
    progress: Arc<AtomicU32>,
    /// The track being rendered.
    current: Arc<AtomicUsize>,
    /// For a prompted track: `STARTING` while the music model starts,
    /// `BEDDING` while its bed is rendered.
    phase: Arc<AtomicU8>,
    cancel: Arc<AtomicBool>,
    done: mpsc::Receiver<Made>,
    started: Instant,
    /// The music model a prompted track comes from: stopped on Cancel, so
    /// a request it is still loading for ends at once.
    server: Option<Arc<Mutex<Server>>>,
    prompted: bool,
}

const STARTING: u8 = 1;
const BEDDING: u8 = 2;

impl Generation {
    /// Starts rendering `seconds` of each track into the library in
    /// `store`, with the `recorded` instruments already loaded and, from
    /// `pack`, any others a track plays.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        ctx: egui::Context,
        store: Store,
        title: String,
        tracks: Vec<TrackPlan>,
        seconds: u32,
        recorded: Recordings,
        pack: Option<Pack>,
        destination: Destination,
    ) -> Self {
        let emojis = tracks
            .iter()
            .map(|track| emoji(track.settings.background))
            .collect();
        let work = Work::Ambience {
            tracks,
            seconds,
            recorded,
            pack,
        };
        Self::spawn(ctx, store, title, emojis, destination, work, None)
    }

    /// Starts making a track from `request` with the music model `client`
    /// talks to (running as `server`, if the app started it).
    #[allow(clippy::too_many_arguments)]
    pub fn start_prompted(
        ctx: egui::Context,
        store: Store,
        title: String,
        request: Request,
        client: Client,
        server: Option<Arc<Mutex<Server>>>,
        destination: Destination,
        bed: Option<Background>,
    ) -> Self {
        let mut emojis = vec![PROMPT_EMOJI];
        if let Some(background) = bed {
            emojis.push(emoji(background));
        }
        let work = Work::Prompt {
            title: title.clone(),
            request,
            client,
            bed,
        };
        Self::spawn(ctx, store, title, emojis, destination, work, server)
    }

    fn spawn(
        ctx: egui::Context,
        store: Store,
        title: String,
        emojis: Vec<&'static str>,
        destination: Destination,
        work: Work,
        server: Option<Arc<Mutex<Server>>>,
    ) -> Self {
        let prompted = matches!(work, Work::Prompt { .. });
        let progress = Arc::new(AtomicU32::new(if prompted {
            f32::NAN.to_bits()
        } else {
            0
        }));
        let current = Arc::new(AtomicUsize::new(0));
        let phase = Arc::new(AtomicU8::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, done) = mpsc::channel();
        let job = Job {
            ctx: ctx.clone(),
            store,
            work,
            progress: Arc::clone(&progress),
            current: Arc::clone(&current),
            phase: Arc::clone(&phase),
            cancel: Arc::clone(&cancel),
        };
        let spawned = thread::Builder::new()
            .name("olio-generator".to_owned())
            .spawn(move || {
                let made = job.run();
                let _ = sender.send(made);
                ctx.request_repaint();
            });
        if let Err(err) = spawned {
            // The receiver reports the thread as gone, with this reason.
            eprintln!("olio: could not start the generator: {err}");
        }
        Self {
            title,
            destination,
            emojis,
            progress,
            current,
            phase,
            cancel,
            done,
            started: Instant::now(),
            server,
            prompted,
        }
    }

    /// How far: 0 to 1 over every track, or, for a prompted track, what is
    /// going on and for how long.
    pub fn progress(&self) -> Progress {
        let done = f32::from_bits(self.progress.load(Ordering::Relaxed));
        if done.is_finite() {
            return Progress::Fraction(done);
        }
        let words = match self.phase.load(Ordering::Relaxed) {
            STARTING => "Starting the music model for",
            BEDDING => "Adding the bed to",
            _ => "Generating",
        };
        Progress::Indeterminate {
            words,
            elapsed: self.started.elapsed(),
        }
    }

    /// The track being rendered, from 1, and how many there are.
    pub fn track(&self) -> (usize, usize) {
        let count = self.emojis.len();
        let current = self.current.load(Ordering::Relaxed) + 1;
        (current.min(count), count)
    }

    /// True for a track made by the music model.
    pub fn is_prompted(&self) -> bool {
        self.prompted
    }

    /// Stops: a prompted track's music model too, so a request it is still
    /// loading for ends at once (the next prompt starts it again).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(server) = &self.server
            && let Ok(mut server) = server.lock()
        {
            server.stop();
        }
    }

    /// What was made, once it is done.
    pub fn poll(&self) -> Option<Made> {
        match self.done.try_recv() {
            Ok(made) => Some(made),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Made {
                sounds: Vec::new(),
                stopped: Some(Stop::Failed(
                    "the generator stopped unexpectedly".to_owned(),
                )),
            }),
        }
    }
}

/// What a generation makes its tracks from.
enum Work {
    /// The ambience rendered, track after track.
    Ambience {
        tracks: Vec<TrackPlan>,
        seconds: u32,
        recorded: Recordings,
        pack: Option<Pack>,
    },
    /// The music model asked for one track, with a bed of this background
    /// rendered after it.
    Prompt {
        title: String,
        request: Request,
        client: Client,
        bed: Option<Background>,
    },
}

struct Job {
    ctx: egui::Context,
    store: Store,
    work: Work,
    progress: Arc<AtomicU32>,
    current: Arc<AtomicUsize>,
    phase: Arc<AtomicU8>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    fn run(mut self) -> Made {
        let work = std::mem::replace(
            &mut self.work,
            Work::Ambience {
                tracks: Vec::new(),
                seconds: 0,
                recorded: Recordings::default(),
                pack: None,
            },
        );
        match work {
            Work::Ambience {
                tracks,
                seconds,
                mut recorded,
                pack,
            } => self.render_all(&tracks, seconds, &mut recorded, pack.as_ref()),
            Work::Prompt {
                title,
                request,
                client,
                bed,
            } => self.prompt(&title, &request, &client, bed),
        }
    }

    fn render_all(
        &self,
        tracks: &[TrackPlan],
        seconds: u32,
        recorded: &mut Recordings,
        pack: Option<&Pack>,
    ) -> Made {
        let mut sounds = Vec::new();
        let count = tracks.len().max(1) as f32;
        for (index, track) in tracks.iter().enumerate() {
            self.current.store(index, Ordering::Relaxed);
            load_for(track, recorded, pack);
            match self.render(track, seconds, recorded, |done| {
                (index as f32 + done) / count
            }) {
                Ok(Some(sound)) => sounds.push(sound),
                Ok(None) => {
                    return Made {
                        sounds,
                        stopped: Some(Stop::Cancelled),
                    };
                }
                Err(err) => {
                    return Made {
                        sounds,
                        stopped: Some(Stop::Failed(err)),
                    };
                }
            }
        }
        Made {
            sounds,
            stopped: None,
        }
    }

    /// Renders one track and imports it: `None` if cancelled. `overall`
    /// turns its own progress into the whole generation's.
    fn render(
        &self,
        track: &TrackPlan,
        seconds: u32,
        recorded: &Recordings,
        overall: impl Fn(f32) -> f32,
    ) -> Result<Option<Sound>, String> {
        let mut shown = Instant::now();
        let clip = render_track(&track.settings, seconds as f32, recorded, |done| {
            self.progress
                .store(overall(done).to_bits(), Ordering::Relaxed);
            if shown.elapsed() > Duration::from_millis(100) {
                shown = Instant::now();
                self.ctx.request_repaint();
            }
            !self.cancel.load(Ordering::Relaxed)
        });
        let Some(clip) = clip else {
            return Ok(None);
        };
        import_track(&self.store, &track.title, |path| {
            clip.write_wav(path, WavFormat::Int16)
                .map(|()| true)
                .map_err(|err| err.to_string())
        })
    }

    /// Waits for the music model, asks it for the track and imports it;
    /// then renders and imports its bed, if it has one.
    fn prompt(
        &self,
        title: &str,
        request: &Request,
        client: &Client,
        bed: Option<Background>,
    ) -> Made {
        let stopped = |stop| Made {
            sounds: Vec::new(),
            stopped: Some(stop),
        };
        self.phase.store(STARTING, Ordering::Relaxed);
        self.ctx.request_repaint();
        match client.wait_healthy(START_TIMEOUT, &self.cancel) {
            Ok(true) => {}
            Ok(false) => return stopped(Stop::Cancelled),
            Err(err) => return stopped(Stop::Failed(err)),
        }
        self.phase.store(0, Ordering::Relaxed);
        self.ctx.request_repaint();
        let made = import_track(&self.store, title, |path| {
            client
                .generate(request, path, &self.cancel)
                .map(|made| made.is_some())
        });
        let sound = match made {
            Ok(Some(sound)) => sound,
            Ok(None) => return stopped(Stop::Cancelled),
            // A request the model was stopped under fails: that was the
            // cancel.
            Err(_) if self.cancel.load(Ordering::Relaxed) => return stopped(Stop::Cancelled),
            Err(err) => return stopped(Stop::Failed(err)),
        };
        let Some(background) = bed else {
            return Made {
                sounds: vec![sound],
                stopped: None,
            };
        };
        // The track is made; what goes wrong from here loses only the bed.
        let stopped = |stop| Made {
            sounds: vec![sound.clone()],
            stopped: Some(stop),
        };
        self.phase.store(BEDDING, Ordering::Relaxed);
        self.ctx.request_repaint();
        let mut shown = Instant::now();
        let clip = render_bed(background, request.seconds, fresh_seed(), |_| {
            if shown.elapsed() > Duration::from_millis(100) {
                shown = Instant::now();
                self.ctx.request_repaint();
            }
            !self.cancel.load(Ordering::Relaxed)
        });
        let Some(clip) = clip else {
            return stopped(Stop::Cancelled);
        };
        let title = bed_title(background, request.seconds);
        match import_track(&self.store, &title, |path| {
            clip.write_wav(path, WavFormat::Int16)
                .map(|()| true)
                .map_err(|err| err.to_string())
        }) {
            Ok(Some(bed)) => Made {
                sounds: vec![sound, bed],
                stopped: None,
            },
            Ok(None) => stopped(Stop::Cancelled),
            Err(err) => stopped(Stop::Failed(err)),
        }
    }
}

/// Loads from the pack any recorded instrument the track plays that is
/// not loaded yet; one that will not load stays synthesised.
fn load_for(track: &TrackPlan, recorded: &mut Recordings, pack: Option<&Pack>) {
    let Some(pack) = pack else {
        return;
    };
    for instrument in track.settings.playing() {
        if recorded.get(instrument).is_none()
            && pack.instruments().any(|i| i == instrument)
            && let Ok(Some(samples)) = pack.load(instrument)
        {
            recorded.set(instrument, Some(Arc::new(samples)));
        }
    }
}

/// Imports a track `write` puts at the path it is given, beside the
/// library: the file is named after the title, which so becomes the
/// tile's. `write` gives false when it stopped short (`None` here).
fn import_track(
    store: &Store,
    title: &str,
    write: impl FnOnce(&Path) -> Result<bool, String>,
) -> Result<Option<Sound>, String> {
    let folder = store.root().join("generating");
    fs::create_dir_all(&folder).map_err(|err| err.to_string())?;
    let name: String = title
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) { '-' } else { c })
        .collect();
    let path = folder.join(format!("{name}.wav"));
    let written = write(&path);
    let sound = match written {
        Ok(true) => store
            .import_sound(&path)
            .map(Some)
            .map_err(|err| err.to_string()),
        Ok(false) => Ok(None),
        Err(err) => Err(err),
    };
    let _ = fs::remove_file(&path);
    let _ = fs::remove_dir(&folder);
    sound
}

// ----- the panel -----------------------------------------------------------

/// A style's name being typed: for a new style (`of` is `None`), or for a
/// saved one being renamed.
pub struct Naming {
    pub text: String,
    pub of: Option<usize>,
    focused: bool,
}

impl Naming {
    pub fn new(text: String, of: Option<usize>) -> Self {
        Self {
            text,
            of,
            focused: false,
        }
    }
}

/// Tracks being generated.
pub struct Busy<'a> {
    /// A track's title, or a new set's name.
    pub title: &'a str,
    pub progress: Progress,
    /// For a set: the track being rendered, from 1, and how many.
    pub track: Option<(usize, usize)>,
}

/// What the panel shows besides the settings.
pub struct View<'a> {
    pub images: &'a EmojiImages,
    /// What is being generated, if anything.
    pub busy: Option<Busy<'a>>,
    pub track_seconds: u32,
    /// The set a track would go to.
    pub set_name: &'a str,
    /// Styles of one's own, and the one the settings are, if any.
    pub styles: &'a [UserStyle],
    pub saved: Option<usize>,
    /// Where a building or evolving ambience playing live has got to.
    pub now: Option<Now>,
    /// After a surprise: what Back returns to.
    pub before_surprise: Option<&'a str>,
    /// Whether the recorded instruments are installed, downloading or on.
    pub recorded: Recorded,
    /// Whether the music model is installed, installing or running.
    pub music_model: MusicModel,
}

/// Where the music model is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MusicModel {
    /// Nowhere to install it.
    Unavailable,
    /// Not installed: the panel offers the install.
    Missing,
    /// Installing: what is going on, and how far where known.
    Installing {
        words: &'static str,
        done: Option<f32>,
    },
    /// Installed, its process running or not.
    Installed { running: bool },
    /// Being removed.
    Removing,
}

/// Where the recorded instruments are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Recorded {
    /// Nowhere to install them.
    Unavailable,
    /// Not installed: the panel offers the download.
    Missing,
    /// Downloading, this far.
    Downloading(f32),
    /// Installed, played or not, some still loading.
    Installed { on: bool, loading: bool },
}

/// What the person did in the panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// The settings changed; send them to the engine.
    Changed,
    /// Picked a track length.
    Length(u32),
    /// Generate a track of the picked length.
    Generate,
    /// Generate a new set of tracks of the picked length.
    GenerateSet(SetOf),
    Cancel,
    /// Start naming the settings as a new style of one's own.
    SaveStyle,
    /// Start renaming the saved style.
    RenameStyle(usize),
    /// Save or rename with the name typed.
    NameStyle,
    CancelNaming,
    /// Remove the saved style, once confirmed.
    RemoveStyle(usize),
    /// Something new, picked at random, playing now.
    Surprise,
    /// Back to what played before the surprise.
    BackFromSurprise,
    /// Download and install the recorded instruments, or stop.
    DownloadInstruments,
    CancelDownload,
    /// Play the recorded instruments (or the synthesised ones).
    RecordedInstruments(bool),
    /// Install the music model, or stop installing it.
    InstallMusicModel,
    CancelInstall,
    /// Generate a track of the picked length from the prompt.
    GeneratePrompt,
    /// Remove the music model, once confirmed.
    RemoveMusicModel,
}

/// The prompt and what goes with it, edited in place: preferences all.
pub struct Prompted<'a> {
    pub prompt: &'a mut String,
    /// The bed under a prompted track; `None` for the track alone.
    pub bed: &'a mut Background,
    /// Keep it ambient: the [`PROMPT_HINTS`] go with the prompt.
    pub hints: &'a mut bool,
}

pub fn show(
    ui: &mut Ui,
    settings: &mut AmbienceSettings,
    view: &View,
    naming: &mut Option<Naming>,
    prompted: Prompted,
    disclosure: &mut Disclosure,
) -> Vec<Action> {
    let before = *settings;
    let mut actions = Vec::new();

    // The track: pinned to the bottom, always in reach.
    egui::Panel::bottom("olio-generator-track")
        .frame(egui::Frame::NONE)
        .show(ui, |ui| track(ui, view, &mut actions));

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Clear of the scroll bar.
            ui.set_max_width(ui.available_width() - 12.0);
            ui.add_space(10.0);
            live(ui, settings);
            section_with(ui, disclosure, "style", "Style", |ui, header| {
                // One scope whatever it holds, so the widgets below keep
                // their ids (and a focused slider its focus) when the
                // settings turn Custom and Save Style… appears.
                ui.scope(|ui| {
                    style(ui, settings, header, view);
                    match naming {
                        Some(naming) => naming_row(ui, naming, view.styles, &mut actions),
                        None => style_actions(ui, settings, view, &mut actions),
                    }
                });
            });
            let more = disclosure.shows_more("key_and_mood");
            let shown = section(ui, disclosure, "key_and_mood", "Key and mood", |ui| {
                key(ui, settings);
                ui.add_space(8.0);
                mood(ui, settings);
                let open = more_toggle(ui, "Key and mood", more);
                ui.scope(|ui| {
                    if open {
                        character(ui, settings);
                    }
                });
                open
            });
            if let Some(open) = shown.inner {
                disclosure.set_more("key_and_mood", open);
            }
            let more = disclosure.shows_more("energy_and_time");
            let shown = section(ui, disclosure, "energy_and_time", "Energy and time", |ui| {
                // One scope: the pulse's row comes and goes with the energy.
                ui.scope(|ui| energy(ui, settings));
                ui.add_space(8.0);
                // One scope, so the line saying where it has got to never
                // moves the ids of the widgets below.
                ui.scope(|ui| over_time(ui, settings, view.now.as_ref()));
                let open = more_toggle(ui, "Energy and time", more);
                ui.scope(|ui| {
                    if open {
                        form(ui, settings);
                    }
                });
                open
            });
            if let Some(open) = shown.inner {
                disclosure.set_more("energy_and_time", open);
            }
            section(ui, disclosure, "background", "Background", |ui| {
                background(ui, settings, view.images);
            });
            let more = disclosure.shows_more("instruments");
            let shown = section(ui, disclosure, "instruments", "Instruments", |ui| {
                // In one scope: it changes as the pack downloads and installs.
                ui.scope(|ui| recorded_row(ui, view.recorded, &mut actions));
                ui.add_space(4.0);
                ui.scope(|ui| {
                    if more {
                        instrument_rows(ui, settings);
                    } else {
                        instrument_chips(ui, settings);
                    }
                    if settings.layers.iter().all(|layer| !layer.enabled) {
                        ui.weak("No instruments: the background only.");
                    }
                });
                more_toggle(ui, "Instruments", more)
            });
            if let Some(open) = shown.inner {
                disclosure.set_more("instruments", open);
            }
            if view.music_model != MusicModel::Unavailable {
                let more = disclosure.shows_more("from_a_prompt");
                let shown = section(ui, disclosure, "from_a_prompt", "From a prompt", |ui| {
                    // In one scope: it changes as the model installs.
                    ui.scope(|ui| prompt_section(ui, view, prompted, more, &mut actions))
                        .inner
                });
                let _ = shown.header.on_hover_text(
                    "A track the music model makes from words, not from the settings above",
                );
                if let Some(Some(open)) = shown.inner {
                    disclosure.set_more("from_a_prompt", open);
                }
            }
            ui.add_space(10.0);
        });

    // Anything but picking a style, switching on or a new variation makes
    // the settings Custom.
    let touched = !settings.same_music(&before);
    if touched && settings.style == before.style {
        settings.style = None;
    }
    if *settings != before {
        actions.push(Action::Changed);
    }
    actions
}

/// A section of the Generator, open or closed as the preferences say.
fn section<R>(
    ui: &mut Ui,
    disclosure: &mut Disclosure,
    key: &str,
    title: &str,
    body: impl FnOnce(&mut Ui) -> R,
) -> skin::Disclosed<R> {
    section_with(ui, disclosure, key, title, |ui, _| body(ui))
}

fn section_with<R>(
    ui: &mut Ui,
    disclosure: &mut Disclosure,
    key: &str,
    title: &str,
    body: impl FnOnce(&mut Ui, &egui::Response) -> R,
) -> skin::Disclosed<R> {
    let mut open = disclosure.is_open(key);
    let shown = skin::section_with(ui, title, &mut open, body);
    disclosure.set_open(key, open);
    shown
}

/// A section's More, after a little room: whether it is open now.
fn more_toggle(ui: &mut Ui, section: &str, open: bool) -> bool {
    let mut open = open;
    ui.add_space(4.0);
    skin::more(ui, section, &mut open);
    open
}

/// A 0-to-1 `value` on a slider row reading in percent: named `name`,
/// known as `label`; true when it moved.
fn percent_row(ui: &mut Ui, name: &str, label: &str, value: &mut f32, hint: &str) -> bool {
    let mut percent = *value * 100.0;
    let readout = format!("{percent:.0} %");
    let moved = skin::slider_row(ui, name, label, &mut percent, 0.0..=100.0, &readout)
        .on_hover_text(hint)
        .changed();
    if moved {
        *value = percent / 100.0;
    }
    moved
}

/// Play live, its level, and whether it ducks under the sounds.
fn live(ui: &mut Ui, settings: &mut AmbienceSettings) {
    skin::switch(ui, "Play live", &mut settings.enabled)
        .on_hover_text("Plays the ambience now, under your sounds");
    percent_row(
        ui,
        "Level",
        "Ambience level",
        &mut settings.level,
        "How loud the ambience plays. Even at full level it stays well under the sounds",
    );
    skin::switch(ui, "Quieter while sounds play", &mut settings.duck)
        .on_hover_text("Sinks to about a third while any sound plays, and comes back after");
}

/// The style to start from, built-in or one's own, a new variation of
/// it, and what it is.
fn style(ui: &mut Ui, settings: &mut AmbienceSettings, title: &egui::Response, view: &View) {
    let saved = view.saved.and_then(|index| view.styles.get(index));
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 128.0).max(120.0);
        let chosen_name = match (settings.style, saved) {
            (Some(style), _) => style.name(),
            (None, Some(style)) => style.name.as_str(),
            (None, None) => "Custom",
        };
        let combo = egui::ComboBox::from_id_salt("olio-generator-style")
            .selected_text(chosen_name)
            .width(width)
            .height(560.0)
            .show_ui(ui, |ui| {
                // Whether it plays, and how it changes over time, stay as
                // they were, whichever is picked.
                let (enabled, journey) = (settings.enabled, settings.journey);
                for style in Style::ALL {
                    let chosen = settings.style == Some(style);
                    if ui
                        .selectable_label(chosen, style.name())
                        .on_hover_text(style.describe())
                        .clicked()
                        && !chosen
                    {
                        *settings = AmbienceSettings {
                            enabled,
                            journey,
                            ..style.settings()
                        };
                    }
                }
                if !view.styles.is_empty() {
                    ui.separator();
                }
                for (index, style) in view.styles.iter().enumerate() {
                    let chosen = settings.style.is_none() && view.saved == Some(index);
                    if ui
                        .selectable_label(chosen, &style.name)
                        .on_hover_text(summary(&style.settings))
                        .clicked()
                        && !chosen
                    {
                        *settings = AmbienceSettings {
                            enabled,
                            journey,
                            ..style.settings
                        };
                    }
                }
            });
        let _ = combo
            .response
            .labelled_by(title.id)
            .on_hover_text("Start from a style; touch anything and the settings are yours");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if skin::button(ui, "New variation", false)
                .on_hover_text("The same settings, different notes")
                .clicked()
            {
                settings.seed = settings.seed.wrapping_add(1);
            }
        });
    });
    ui.weak(match (settings.style, saved) {
        (Some(style), _) => sentence(style.describe()),
        (None, Some(_)) => format!("Your style: {}", summary(settings)),
        (None, None) => sentence(summary(settings).trim_end_matches('.')),
    });
}

/// Surprise Me, and Back after one; Custom settings can be saved as a
/// style, a saved one renamed or removed.
fn style_actions(ui: &mut Ui, settings: &AmbienceSettings, view: &View, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let dice = view.images.texture(ui.ctx(), "🎲");
        if skin::picture_button(ui, "Surprise Me", dice)
            .on_hover_text(
                "Something new, playing now: a key, a background, a few instruments and a \
                 mood picked at random",
            )
            .clicked()
        {
            actions.push(Action::Surprise);
        }
        if let Some(name) = view.before_surprise
            && skin::button(ui, "Back", false)
                .on_hover_text(format!("Back to {name}"))
                .clicked()
        {
            actions.push(Action::BackFromSurprise);
        }
        if settings.style.is_some() {
            return;
        }
        match view.saved {
            Some(index) => {
                if skin::button(ui, "Rename…", false)
                    .on_hover_text("Give this style another name")
                    .clicked()
                {
                    actions.push(Action::RenameStyle(index));
                }
                if skin::button(ui, "Remove Style", false)
                    .on_hover_text("Takes the style out of the list; tracks made from it stay")
                    .clicked()
                {
                    actions.push(Action::RemoveStyle(index));
                }
            }
            None => {
                if skin::button(ui, "Save Style…", false)
                    .on_hover_text("Keep these settings as a style of your own, in the Style list")
                    .clicked()
                {
                    actions.push(Action::SaveStyle);
                }
            }
        }
    });
}

/// The name being typed: Return or Save keeps it, Escape or Cancel
/// doesn't.
fn naming_row(ui: &mut Ui, naming: &mut Naming, styles: &[UserStyle], actions: &mut Vec<Action>) {
    let problem = style_name_problem(styles, &naming.text, naming.of);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 150.0).max(100.0);
        let response = skin::field(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut naming.text)
                    .hint_text("Style name")
                    .desired_width(width),
            )
        });
        if !naming.focused {
            response.request_focus();
            naming.focused = true;
        }
        let entered = response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        let save = ui
            .add_enabled_ui(problem.is_none(), |ui| skin::button(ui, "Save", false))
            .inner
            .on_hover_text("Keep the style under this name (Return)")
            .clicked();
        let cancel = skin::button(ui, "Cancel", false)
            .on_hover_text("Leave the styles as they were (Escape)")
            .clicked();
        if cancel || ui.input(|input| input.key_pressed(Key::Escape)) {
            actions.push(Action::CancelNaming);
        } else if (entered || save) && problem.is_none() {
            actions.push(Action::NameStyle);
        } else if entered {
            // Not a name it can have: keep typing.
            response.request_focus();
        }
    });
    if let Some(problem) = problem {
        ui.weak(problem);
    } else if naming.of.is_none()
        && let Some(style) = style_named(styles, &naming.text).and_then(|index| styles.get(index))
    {
        ui.weak(format!("Replaces the style “{}”", style.name));
    }
}

/// Steady, building up or evolving, and where it has got to.
fn over_time(ui: &mut Ui, settings: &mut AmbienceSettings, now: Option<&Now>) {
    skin::legend_label(ui, "Over time")
        .on_hover_text("Whether the music stays as it is or changes as it plays");
    choose(
        ui,
        &mut settings.journey,
        &Journey::ALL,
        Journey::name,
        Journey::describe,
    );
    ui.weak(sentence(settings.journey.describe()));
    if let Some(now) = now {
        ui.label(now_words(now));
    }
}

/// The moods, and what the one chosen sounds like.
fn mood(ui: &mut Ui, settings: &mut AmbienceSettings) {
    let now = settings.mood();
    skin::legend_label(ui, "Mood")
        .on_hover_text("How the music moves: how often a motif comes back, how near the notes stay, how much they clash");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for mood in Mood::ALL {
            if skin::chip(ui, mood.name(), None, now == Some(mood))
                .on_hover_text(mood.describe())
                .clicked()
            {
                settings.set_mood(mood);
            }
        }
        if now.is_none() {
            let _ = skin::chip(ui, "Custom", None, true)
                .on_hover_text("Pick a mood to start over from one");
        }
    });
    ui.weak(character_words(settings));
}

/// A row of chips, wrapping, one of them chosen.
fn choose<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    all: &[T],
    name: impl Fn(T) -> &'static str,
    hint: impl Fn(T) -> &'static str,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for &choice in all {
            let response = skin::chip(ui, name(choice), None, *value == choice);
            let response = match hint(choice) {
                "" => response,
                text => response.on_hover_text(text),
            };
            if response.clicked() {
                *value = choice;
            }
        }
    });
}

fn key(ui: &mut Ui, settings: &mut AmbienceSettings) {
    ui.horizontal(|ui| {
        let root = ui.label("Root");
        let combo = egui::ComboBox::from_id_salt("olio-generator-root")
            .selected_text(ROOT_NAMES[usize::from(settings.scale.root % 12)])
            .width(56.0)
            .show_ui(ui, |ui| {
                for (index, name) in ROOT_NAMES.iter().enumerate() {
                    let index = index as u8;
                    ui.selectable_value(&mut settings.scale.root, index, *name);
                }
            });
        let _ = combo
            .response
            .labelled_by(root.id)
            .on_hover_text("The home note the music keeps coming back to");
        ui.add_space(8.0);
        let scale = ui.label("Scale");
        let width = (ui.available_width() - 8.0).max(120.0);
        let combo = egui::ComboBox::from_id_salt("olio-generator-scale")
            .selected_text(settings.scale.kind.name())
            .width(width)
            .show_ui(ui, |ui| {
                for kind in ScaleKind::all() {
                    let _ = ui
                        .selectable_value(&mut settings.scale.kind, kind, kind.name())
                        .on_hover_text(kind.character());
                }
            });
        let _ = combo
            .response
            .labelled_by(scale.id)
            .on_hover_text("Which notes the music uses, and so its colour");
    });
    ui.add_space(4.0);
    skin::switch(ui, "Suspended", &mut settings.suspended).on_hover_text(
        "Chords with a second or a fourth in place of their third, and no third in the \
         tunes: neither major nor minor, open and floating",
    );
    ui.weak(if settings.suspended {
        format!(
            "{}, suspended: neither major nor minor; open, floating.",
            settings.scale.name()
        )
    } else {
        format!(
            "{}: {}.",
            settings.scale.name(),
            settings.scale.kind.character()
        )
    });
}

/// The three amounts behind a mood, under Key and mood's More.
fn character(ui: &mut Ui, settings: &mut AmbienceSettings) {
    ui.add_space(4.0);
    percent_row(
        ui,
        "Variety",
        "Variety",
        &mut settings.variety,
        "Low: a short motif comes back again and again, on few chords, in an even rhythm. \
         High: new melodies, more chords, freer rhythm",
    );
    percent_row(
        ui,
        "Concentration",
        "Concentration",
        &mut settings.concentration,
        "High: the notes gather around the chord and the home note, in small steps and a \
         narrow range. Low: they wander wide",
    );
    percent_row(
        ui,
        "Chaos",
        "Chaos",
        &mut settings.chaos,
        "Dissonance: notes from outside the key, clashing seconds and tritones, chords from \
         anywhere and looser timing, more as it rises",
    );
}

fn background(ui: &mut Ui, settings: &mut AmbienceSettings, images: &EmojiImages) {
    background_chips(ui, &mut settings.background, images, false);
    ui.weak(settings.background.describe());
    // Its balance with the instruments: in one scope, so whether it shows
    // never moves the ids of the widgets below.
    ui.scope(|ui| {
        if settings.background != Background::None {
            ui.add_space(2.0);
            balance(ui, settings);
        }
    });
}

/// Each background as a chip with its picture, the chosen one lit. As a
/// `bed`, each is known as "Rain bed" (and None as "No bed"), so it is not
/// taken for the Background section's.
fn background_chips(ui: &mut Ui, value: &mut Background, images: &EmojiImages, bed: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for choice in Background::ALL {
            let picture = images.texture(ui.ctx(), emoji(choice));
            let label = match (bed, choice) {
                (false, _) => choice.name().to_owned(),
                (true, Background::None) => "No bed".to_owned(),
                (true, _) => format!("{} bed", choice.name()),
            };
            if skin::chip_labelled(ui, choice.name(), &label, picture, *value == choice)
                .on_hover_text(choice.describe())
                .clicked()
            {
                *value = choice;
            }
        }
    });
}

/// One slider between the background and the instruments, the middle as
/// the style was made.
fn balance(ui: &mut Ui, settings: &mut AmbienceSettings) {
    let background = settings.background.name().to_lowercase();
    let hint = format!(
        "Balance {:.0} %: the {background} alone at the left, the instruments alone at the \
         right; the middle as the style was made. Each is up to twice as loud towards its end",
        settings.balance * 100.0
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(settings.background.name())
            .on_hover_text(hint.clone());
        let instruments = ui
            .painter()
            .layout_no_wrap("Instruments".to_owned(), skin::body_font(), Color32::WHITE)
            .size()
            .x;
        let length = (ui.available_width() - instruments - 8.0).clamp(60.0, 260.0);
        let mut percent = settings.balance * 100.0;
        if skin::slider(ui, &mut percent, 0.0..=100.0, length, false, "Balance")
            .on_hover_text(hint.clone())
            .changed()
        {
            settings.balance = percent / 100.0;
        }
        ui.label("Instruments").on_hover_text(hint);
    });
}

/// The recorded instruments: offered for download, downloading, or,
/// installed, a switch between them and the synthesised ones.
fn recorded_row(ui: &mut Ui, recorded: Recorded, actions: &mut Vec<Action>) {
    const ABOUT: &str = "A grand piano, harp, hand chimes, violin and bass pizzicato, string \
                         sections, bowed vibraphone and hand drums, recorded by Versilian \
                         Studios (public domain)";
    match recorded {
        Recorded::Unavailable => {}
        Recorded::Missing => {
            let label = format!("Download Recorded Instruments ({})", megabytes(PACK_BYTES));
            if skin::button(ui, &label, false)
                .on_hover_text(format!(
                    "{ABOUT}. Downloaded once, then played in place of the synthesised ones"
                ))
                .clicked()
            {
                actions.push(Action::DownloadInstruments);
            }
        }
        Recorded::Downloading(progress) => {
            ui.horizontal(|ui| {
                if skin::button(ui, "Stop Download", false)
                    .on_hover_text("Stop; the synthesised instruments play on")
                    .clicked()
                {
                    actions.push(Action::CancelDownload);
                }
                let text = format!("Downloading recorded instruments {:.0} %", progress * 100.0);
                ui.add(egui::ProgressBar::new(progress).text(text));
            });
        }
        Recorded::Installed { on, loading } => {
            ui.horizontal(|ui| {
                let mut value = on;
                if skin::switch(ui, "Recorded instruments", &mut value)
                    .on_hover_text(format!("{ABOUT}. Off, every instrument is synthesised"))
                    .clicked()
                {
                    actions.push(Action::RecordedInstruments(value));
                }
                if loading {
                    ui.spinner();
                }
            });
        }
    }
}

/// The music model: offered for install, installing, or, installed, the
/// prompt and the button that makes a track of it. Installed, it ends
/// with its More (the model's state and Remove), and gives back whether
/// that is open.
fn prompt_section(
    ui: &mut Ui,
    view: &View,
    prompted: Prompted,
    more: bool,
    actions: &mut Vec<Action>,
) -> Option<bool> {
    let Prompted { prompt, bed, hints } = prompted;
    const ABOUT: &str = "ACE-Step 1.5, an open-source music model, installed once into the \
                         per-user data folder with the Python it needs. It runs on this \
                         computer; nothing you type leaves it. It takes about 8 GB of memory \
                         while it runs";
    match view.music_model {
        MusicModel::Unavailable => None,
        MusicModel::Missing => {
            let label = format!("Install Music Model ({})", about_gigabytes(DISK_BYTES));
            if skin::button(ui, &label, false)
                .on_hover_text(ABOUT)
                .clicked()
            {
                actions.push(Action::InstallMusicModel);
            }
            ui.weak("Tracks from a few words, once the music model is installed.");
            None
        }
        MusicModel::Installing { words, done } => {
            ui.horizontal(|ui| {
                if skin::button(ui, "Stop Install", false)
                    .on_hover_text("Stop; Install Music Model later goes on from here")
                    .clicked()
                {
                    actions.push(Action::CancelInstall);
                }
                let text = match done {
                    Some(done) => {
                        format!("Installing the music model: {words} {:.0} %", done * 100.0)
                    }
                    None => format!("Installing the music model: {words}…"),
                };
                let bar = egui::ProgressBar::new(done.unwrap_or(0.0))
                    .animate(done.is_none())
                    .text(text);
                ui.add(bar);
            });
            None
        }
        MusicModel::Removing => {
            ui.weak("Removing the music model…");
            None
        }
        MusicModel::Installed { running } => {
            skin::field(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(prompt)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY)
                        .hint_text("Instruments, mood, tempo: “warm piano over soft rain, slow”"),
                )
            })
            .on_hover_text("What the track is made from. Prompted tracks are instrumental");
            ui.add_space(4.0);
            skin::switch(ui, "Keep it ambient", hints).on_hover_text(format!(
                "Asks for “{PROMPT_HINTS}” along with your words, so the track stays \
                 behind the sounds; off, the words go as they are"
            ));
            ui.add_space(4.0);
            skin::legend_label(ui, "Bed").on_hover_text(
                "The Generator's background alone, rendered quietly onto a second tile \
                 that loops under the track: a world for the music to sit in",
            );
            background_chips(ui, bed, view.images, true);
            ui.weak(match *bed {
                Background::None => "No bed: the track alone.".to_owned(),
                background => format!(
                    "{}: on a tile of its own, looping, well under the track.",
                    background.describe()
                ),
            });
            ui.add_space(6.0);
            let ready = view.busy.is_none() && !prompt.trim().is_empty();
            if ui
                .add_enabled_ui(ready, |ui| {
                    skin::wide_button(ui, "Generate from Prompt", None, false)
                })
                .inner
                .on_hover_text(
                    "Asks the music model for this much music from these words, then puts \
                     it on a tile to play, loop and polish like any other; with a bed, the \
                     track comes polished and the bed loops on the next tile",
                )
                .clicked()
            {
                actions.push(Action::GeneratePrompt);
            }
            ui.weak(match *bed {
                Background::None => format!(
                    "{} onto a tile in “{}”",
                    length_name(view.track_seconds),
                    view.set_name
                ),
                _ => format!(
                    "{} onto two tiles in “{}”: the track and its bed",
                    length_name(view.track_seconds),
                    view.set_name
                ),
            });
            let open = more_toggle(ui, "From a prompt", more);
            ui.scope(|ui| {
                if open {
                    ui.weak(if running {
                        "The music model is running (about 6 GB); it stops after ten minutes \
                         unused."
                    } else {
                        "The music model is installed and stopped."
                    });
                    if skin::button(ui, "Remove Music Model…", false)
                        .on_hover_text("Its files (about 10 GB) go; tracks made from it stay")
                        .clicked()
                    {
                        actions.push(Action::RemoveMusicModel);
                    }
                }
            });
            Some(open)
        }
    }
}

/// Each instrument on or off, side by side.
fn instrument_chips(ui: &mut Ui, settings: &mut AmbienceSettings) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for instrument in Instrument::ALL {
            let layer = &mut settings.layers[instrument];
            if skin::chip(ui, instrument.name(), None, layer.enabled)
                .on_hover_text(instrument.describe())
                .clicked()
            {
                layer.enabled = !layer.enabled;
            }
        }
    });
}

/// The width of the instruments' switch column, so their sliders line up.
const INSTRUMENT_COLUMN: f32 = 132.0;

/// Each instrument on or off with its level; one that is on also shows
/// how many notes it plays (the drone holds one) and its register.
fn instrument_rows(ui: &mut Ui, settings: &mut AmbienceSettings) {
    for instrument in Instrument::ALL {
        let seconds = settings.spacing(instrument);
        let layer = &mut settings.layers[instrument];
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(INSTRUMENT_COLUMN, skin::SLIDER_BREADTH),
                Layout::left_to_right(Align::Center),
                |ui| {
                    // The whole column, however long the name, so the
                    // sliders line up.
                    ui.set_min_width(INSTRUMENT_COLUMN);
                    skin::switch(ui, instrument.name(), &mut layer.enabled)
                        .on_hover_text(instrument.describe());
                },
            );
            ui.add_enabled_ui(layer.enabled, |ui| {
                let hint = format!(
                    "{} at {:.0} %, among the instruments",
                    instrument.name(),
                    layer.level * 100.0
                );
                let label = format!("{} level", instrument.name());
                amount_slider(ui, &mut layer.level, &label, &hint);
            });
        });
        if !layer.enabled {
            continue;
        }
        if instrument != Instrument::Drone {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(INSTRUMENT_COLUMN, skin::SLIDER_BREADTH),
                    Layout::right_to_left(Align::Center),
                    |ui| {
                        ui.set_min_width(INSTRUMENT_COLUMN);
                        ui.add_space(8.0);
                        ui.weak("Notes");
                    },
                );
                let label = format!("{} notes", instrument.name());
                let hint = format!(
                    "Notes: {}, about one every {seconds:.0} s. Fewer to the left, more to the \
                     right",
                    layer.density.name()
                );
                let mut amount = layer.density.amount();
                if amount_slider(ui, &mut amount, &label, &hint) {
                    layer.density = Density::new(amount);
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add_space(INSTRUMENT_COLUMN + ui.spacing().item_spacing.x);
            row(
                ui,
                &mut layer.register,
                &Register::ALL,
                Register::name,
                Register::describe,
            );
        });
        ui.add_space(6.0);
    }
}

/// A slider for a 0-to-1 `value` filling the rest of the row, with its
/// percent in a readout after it, named `label`; true when it moved.
fn amount_slider(ui: &mut Ui, value: &mut f32, label: &str, hint: &str) -> bool {
    let mut percent = *value * 100.0;
    let spacing = ui.spacing().item_spacing.x;
    let length = (ui.available_width() - skin::ROW_READOUT_WIDTH - spacing).max(60.0);
    let moved = skin::slider(ui, &mut percent, 0.0..=100.0, length, false, label)
        .on_hover_text(hint)
        .changed();
    skin::readout_label(ui, skin::ROW_READOUT_WIDTH, &format!("{percent:.0} %"));
    if moved {
        *value = percent / 100.0;
    }
    moved
}

/// Small choices side by side, never wrapping.
fn row<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    all: &[T],
    name: impl Fn(T) -> &'static str,
    hint: impl Fn(T) -> &'static str,
) {
    ui.spacing_mut().item_spacing.x = 6.0;
    for &choice in all {
        if skin::chip(ui, name(choice), None, *value == choice)
            .on_hover_text(sentence(hint(choice)))
            .clicked()
        {
            *value = choice;
        }
    }
}

/// How much there is of the music: its four names, the amount, and the
/// pulse when there is one.
fn energy(ui: &mut Ui, settings: &mut AmbienceSettings) {
    let now = settings.presence();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for presence in Presence::ALL {
            if skin::chip(ui, presence.name(), None, now == Some(presence))
                .on_hover_text(presence.describe())
                .clicked()
            {
                settings.set_presence(presence);
            }
        }
    });
    let hint = format!(
        "Energy {:.0} %: how much there is of the music, not how loud. Softer and sparser to \
         the left; harder and busier to the right, on a pulse from the middle, with running \
         lines and fuller chords near the top",
        settings.energy * 100.0
    );
    percent_row(ui, "Energy", "Energy", &mut settings.energy, &hint);
    if settings.pulsed() {
        let hint = "Beats per minute of the pulse; the chords change every eight beats";
        let mut tempo = f32::from(settings.tempo);
        let readout = format!("{tempo:.0} BPM");
        if skin::slider_row(ui, "Pulse", "Pulse", &mut tempo, 30.0..=120.0, &readout)
            .on_hover_text(hint)
            .changed()
        {
            settings.tempo = tempo.round() as u16;
        }
    }
    ui.weak(sentence(Presence::nearest(settings.energy).describe()));
}

/// Endless or in pieces, and how long a piece and its pause are; under
/// Energy and time's More.
fn form(ui: &mut Ui, settings: &mut AmbienceSettings) {
    ui.add_space(4.0);
    skin::legend_label(ui, "Form")
        .on_hover_text("Whether the music plays on, or in pieces with a pause between them");
    let pieces = matches!(settings.form, Form::Piece { .. });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if skin::chip(ui, "Endless", None, !pieces)
            .on_hover_text("Plays on without a break")
            .clicked()
            && pieces
        {
            settings.form = Form::Endless;
        }
        if skin::chip(ui, "Pieces", None, pieces)
            .on_hover_text("Each piece opens, builds, closes on the root, then pauses")
            .clicked()
            && !pieces
        {
            settings.form = Form::Piece {
                minutes: 3.0,
                pause_seconds: 20.0,
            };
        }
    });
    if let Form::Piece {
        minutes,
        pause_seconds,
    } = &mut settings.form
    {
        let readout = format!("{minutes:.1} min");
        let _ = skin::slider_row(ui, "Piece", "Piece", minutes, 0.5..=30.0, &readout)
            .on_hover_text("How long each piece plays before it closes on the root");
        let readout = format!("{pause_seconds:.0} s");
        let _ = skin::slider_row(ui, "Pause", "Pause", pause_seconds, 0.0..=120.0, &readout)
            .on_hover_text("The background alone between one piece and the next");
    }
}

fn track(ui: &mut Ui, view: &View, actions: &mut Vec<Action>) {
    ui.add_space(12.0);
    skin::legend_label(ui, "Track")
        .on_hover_text("These settings rendered to a sound in the library, on a tile");
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let hint = "How long the generated track is";
        ui.label("Length").on_hover_text(hint);
        for (seconds, name) in TRACK_LENGTHS {
            if skin::chip(ui, name, None, view.track_seconds == seconds)
                .on_hover_text(hint)
                .clicked()
            {
                actions.push(Action::Length(seconds));
            }
        }
    });
    ui.add_space(6.0);
    match &view.busy {
        Some(busy) => {
            ui.horizontal(|ui| {
                if skin::button(ui, "Cancel", false)
                    .on_hover_text("Stop generating; tracks already made stay")
                    .clicked()
                {
                    actions.push(Action::Cancel);
                }
                let bar = match busy.progress {
                    Progress::Fraction(done) => {
                        let percent = done * 100.0;
                        let text = match busy.track {
                            Some((track, count)) => format!(
                                "Generating “{}”: {track} of {count}, {percent:.0} %",
                                busy.title
                            ),
                            None => format!("Generating “{}” {percent:.0} %", busy.title),
                        };
                        egui::ProgressBar::new(done).text(text)
                    }
                    Progress::Indeterminate { words, elapsed } => {
                        let seconds = elapsed.as_secs();
                        let text = format!(
                            "{words} “{}”… {}:{:02}",
                            busy.title,
                            seconds / 60,
                            seconds % 60
                        );
                        egui::ProgressBar::new(0.0).animate(true).text(text)
                    }
                };
                ui.add(bar);
            });
        }
        None => {
            if skin::wide_button(ui, "Generate Track", None, false)
                .on_hover_text(
                    "Renders this ambience to a sound in the library and puts it on a tile, \
                     to play, loop and polish like any other",
                )
                .clicked()
            {
                actions.push(Action::Generate);
            }
            ui.weak(format!("onto a tile in “{}”", view.set_name));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label("New set of")
                    .on_hover_text("Several tracks at once, in a set of their own");
                for of in SetOf::ALL {
                    if skin::button(ui, of.name(), false)
                        .on_hover_text(of.describe())
                        .clicked()
                    {
                        actions.push(Action::GenerateSet(of));
                    }
                }
            });
        }
    }
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_holds_variations_moods_or_every_style() {
        use olio_audio::ambience::Journey;
        let mut settings = Style::Campfire.settings();
        settings.journey = Journey::Evolve;
        let own = [UserStyle::new(
            "Night Harp".to_owned(),
            Style::Cathedral.settings(),
        )];

        let (name, tracks) = set_plan(SetOf::Variations, &settings, "Campfire", &own);
        assert_eq!(name, "Campfire Variations");
        let titles: Vec<&str> = tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Campfire 1", "Campfire 2", "Campfire 3", "Campfire 4"]
        );
        let seeds: std::collections::HashSet<u64> =
            tracks.iter().map(|t| t.settings.seed).collect();
        assert_eq!(seeds.len(), 4, "each with other notes");
        assert!(tracks.iter().all(|t| t.settings.same_music(&settings)));

        let (name, tracks) = set_plan(SetOf::Moods, &settings, "Campfire", &own);
        assert_eq!(name, "Campfire Moods");
        assert_eq!(tracks.len(), Mood::ALL.len());
        assert_eq!(tracks[0].title, "Campfire Still");
        for (track, mood) in tracks.iter().zip(Mood::ALL) {
            assert_eq!(track.settings.mood(), Some(mood));
            assert_eq!(track.settings.background, settings.background);
        }

        let (name, tracks) = set_plan(SetOf::Styles, &settings, "Campfire", &own);
        assert_eq!(name, "Styles");
        assert_eq!(tracks.len(), Style::ALL.len() + 1);
        assert_eq!(tracks[0].title, "Rain Study");
        assert_eq!(tracks.last().unwrap().title, "Night Harp");
        assert!(
            tracks.iter().all(|t| t.settings.journey == Journey::Evolve),
            "played the way chosen"
        );
    }

    #[test]
    fn prompted_tracks_are_titled_by_their_prompt_cut_at_a_word() {
        assert_eq!(
            prompt_track_title("  warm piano   over soft rain, slow ", 120),
            "Warm piano over soft rain, slow 2 min"
        );
        assert_eq!(
            prompt_track_title(
                "a gentle lo-fi beat with vinyl crackle and a warm upright bass under it all",
                30
            ),
            "A gentle lo-fi beat with vinyl crackle 30 s"
        );
        assert_eq!(
            prompt_track_title("Supercalifragilisticexpialidociousandthensomemorewords", 60),
            "Supercalifragilisticexpialidociousandthe 1 min"
        );
        assert_eq!(
            prompt_track_title("rain/thunder: 2 pm?", 60),
            "Rain-thunder- 2 pm- 1 min"
        );
        assert_eq!(prompt_track_title("   ", 60), "Prompt 1 min");
    }

    #[test]
    fn a_prompted_generation_imports_the_music_models_wav() {
        use crate::music_model::client::fake::{Fake, Settings};
        let fake = Fake::start(Settings {
            audio: crate::music_model::client::tests::wav(),
            ..Settings::default()
        });
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("library")).unwrap();
        let ctx = egui::Context::default();
        let request = Request {
            prompt: "soft piano".to_owned(),
            seconds: 30,
            seed: None,
        };
        let generation = Generation::start_prompted(
            ctx,
            store.clone(),
            prompt_track_title(&request.prompt, request.seconds),
            request,
            Client::new(fake.base.clone(), None),
            None,
            Destination::NewSet,
            None,
        );
        assert!(generation.is_prompted());
        assert_eq!(generation.emojis, [PROMPT_EMOJI]);
        assert!(matches!(
            generation.progress(),
            Progress::Indeterminate { .. }
        ));
        let deadline = Instant::now() + Duration::from_secs(30);
        let made = loop {
            if let Some(made) = generation.poll() {
                break made;
            }
            assert!(Instant::now() < deadline, "never finished");
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(made.stopped, None);
        assert_eq!(made.sounds.len(), 1);
        assert_eq!(made.sounds[0].original_name, "Soft piano 30 s.wav");
        assert!(store.sound_path(&made.sounds[0]).is_file());
        assert!(!store.root().join("generating").exists(), "tidied");
    }

    #[test]
    fn a_prompted_track_can_come_with_a_bed_of_the_background_alone() {
        use crate::music_model::client::fake::{Fake, Settings};
        let fake = Fake::start(Settings {
            audio: crate::music_model::client::tests::wav(),
            ..Settings::default()
        });
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("library")).unwrap();
        let request = Request {
            prompt: hinted("soft piano"),
            seconds: 10,
            seed: None,
        };
        let generation = Generation::start_prompted(
            egui::Context::default(),
            store.clone(),
            prompt_track_title("soft piano", 10),
            request,
            Client::new(fake.base.clone(), None),
            None,
            Destination::Tile(olio_library::SetId::random()),
            Some(Background::Waves),
        );
        assert_eq!(generation.emojis, [PROMPT_EMOJI, "🌊"]);
        let deadline = Instant::now() + Duration::from_secs(60);
        let made = loop {
            if let Some(made) = generation.poll() {
                break made;
            }
            assert!(Instant::now() < deadline, "never finished");
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(made.stopped, None);
        let names: Vec<_> = made
            .sounds
            .iter()
            .map(|s| s.original_name.as_str())
            .collect();
        assert_eq!(names, ["Soft piano 10 s.wav", "Waves bed 10 s.wav"]);
        let bed = olio_audio::decode_file(&store.sound_path(&made.sounds[1])).unwrap();
        assert_eq!(bed.channels(), 2);
        assert!((bed.duration().as_secs_f32() - 10.0).abs() < 0.1);
        // Well under a track: a track is about -20 dBFS, the bed 12 dB less.
        let rms = bed.rms(0..bed.frames());
        assert!(rms > 0.01 && rms < 0.04, "{rms}");
        assert_eq!(
            fake.bodies()[0]["prompt"],
            "soft piano, ambient, sparse, slow, spacious, soft, no drums"
        );
    }

    #[test]
    fn hints_follow_the_prompt_and_beds_are_titled_by_their_background() {
        assert_eq!(
            hinted("soft piano, "),
            "soft piano, ambient, sparse, slow, spacious, soft, no drums"
        );
        assert_eq!(
            hinted("  rain. "),
            "rain, ambient, sparse, slow, spacious, soft, no drums"
        );
        assert_eq!(bed_title(Background::Rain, 30), "Rain bed 30 s");
        assert_eq!(bed_title(Background::Forest, 120), "Forest bed 2 min");
        let settings = bed_settings(Background::Fire, 9);
        assert_eq!(settings.background, Background::Fire);
        assert_eq!(settings.balance, 0.0, "the background alone");
        assert_eq!(settings.seed, 9);
        assert!(fresh_seed() != 0);
    }

    #[test]
    fn tracks_are_titled_by_style_or_key_and_length() {
        assert_eq!(track_title("Rain Study", 120), "Rain Study 2 min");
        assert_eq!(track_title("D Dorian", 30), "D Dorian 30 s");
        let settings = Style::RainStudy.settings();
        assert_eq!(summary(&settings), "D Dorian, rain; bass and harp.");
        assert_eq!(length_name(180), "3 min");
        for background in Background::ALL {
            assert!(!emoji(background).is_empty());
        }
    }

    #[test]
    fn where_an_evolving_ambience_has_got_to_reads_in_words() {
        use olio_audio::ambience::{Mode, Scale};
        let mut now = Now {
            scale: Scale {
                root: 4,
                kind: ScaleKind::Mode(Mode::Dorian),
            },
            background: Background::Thunder,
            resting: Some(Instrument::Harp),
            building: false,
        };
        assert_eq!(now_words(&now), "Now: E Dorian, thunder; the harp resting.");
        now.building = true;
        now.background = Background::None;
        assert_eq!(now_words(&now), "Now: E Dorian; building up.");
    }

    #[test]
    fn the_character_reads_in_words() {
        let mut settings = Style::RainStudy.settings();
        assert_eq!(
            character_words(&settings),
            "Comes back to a motif now and then, around the chord, in the key."
        );
        settings.variety = 0.0;
        settings.concentration = 1.0;
        settings.chaos = 0.9;
        assert_eq!(
            character_words(&settings),
            "Repeats a short motif, close to the home note, dissonant and unsettled."
        );
    }
}

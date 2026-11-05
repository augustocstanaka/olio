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
//! written as a WAV into the library and placed on a tile by the app.

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use egui::{Align, Color32, Key, Layout, Ui};
use olio_audio::ambience::{
    AmbienceSettings, Background, Density, Form, Instrument, Journey, Layer, Mood, Now, Presence,
    ROOT_NAMES, Register, ScaleKind, Style,
};
use olio_audio::instruments::{Pack, Recorded as Recordings};
use olio_audio::{WavFormat, render_track};
use olio_library::{Sound, Store};

use crate::emoji::EmojiImages;
use crate::instruments::{PACK_BYTES, megabytes};
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
        _ => "Always finding new melodies, now and then a borrowed chord",
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

/// Tracks being rendered, one after another, on a thread of their own.
pub struct Generation {
    /// A track's title, or the new set's name.
    pub title: String,
    pub destination: Destination,
    /// Each track's emoji, for its tile.
    pub emojis: Vec<&'static str>,
    /// How far, as `f32` bits, 0 to 1, over every track.
    progress: Arc<AtomicU32>,
    /// The track being rendered.
    current: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    done: mpsc::Receiver<Made>,
}

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
        let progress = Arc::new(AtomicU32::new(0));
        let current = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, done) = mpsc::channel();
        let emojis = tracks
            .iter()
            .map(|track| emoji(track.settings.background))
            .collect();
        let job = Job {
            ctx: ctx.clone(),
            store,
            tracks,
            seconds,
            recorded,
            pack,
            progress: Arc::clone(&progress),
            current: Arc::clone(&current),
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
            cancel,
            done,
        }
    }

    /// 0 to 1, over every track.
    pub fn progress(&self) -> f32 {
        f32::from_bits(self.progress.load(Ordering::Relaxed))
    }

    /// The track being rendered, from 1, and how many there are.
    pub fn track(&self) -> (usize, usize) {
        let count = self.emojis.len();
        let current = self.current.load(Ordering::Relaxed) + 1;
        (current.min(count), count)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
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

struct Job {
    ctx: egui::Context,
    store: Store,
    tracks: Vec<TrackPlan>,
    seconds: u32,
    recorded: Recordings,
    pack: Option<Pack>,
    progress: Arc<AtomicU32>,
    current: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    fn run(mut self) -> Made {
        let mut sounds = Vec::new();
        let count = self.tracks.len().max(1) as f32;
        let tracks = std::mem::take(&mut self.tracks);
        for (index, track) in tracks.iter().enumerate() {
            self.current.store(index, Ordering::Relaxed);
            self.load_for(track);
            match self.render(track, |done| (index as f32 + done) / count) {
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

    /// Loads from the pack any recorded instrument the track plays that is
    /// not loaded yet; one that will not load stays synthesised.
    fn load_for(&mut self, track: &TrackPlan) {
        if let Some(pack) = &self.pack {
            load_voices(pack, &mut self.recorded, &track.settings);
        }
    }

    /// Renders one track and imports it: `None` if cancelled. `overall`
    /// turns its own progress into the whole generation's.
    fn render(
        &self,
        track: &TrackPlan,
        overall: impl Fn(f32) -> f32,
    ) -> Result<Option<Sound>, String> {
        let mut shown = Instant::now();
        let clip = render_track(
            &track.settings,
            self.seconds as f32,
            &self.recorded,
            |done| {
                self.progress
                    .store(overall(done).to_bits(), Ordering::Relaxed);
                if shown.elapsed() > Duration::from_millis(100) {
                    shown = Instant::now();
                    self.ctx.request_repaint();
                }
                !self.cancel.load(Ordering::Relaxed)
            },
        );
        let Some(clip) = clip else {
            return Ok(None);
        };
        // Written beside the library, then imported like any sound: the
        // file name gives the tile its title.
        let folder = self.store.root().join("generating");
        fs::create_dir_all(&folder).map_err(|err| err.to_string())?;
        let name: String = track
            .title
            .chars()
            .map(|c| if "/\\:*?\"<>|".contains(c) { '-' } else { c })
            .collect();
        let path = folder.join(format!("{name}.wav"));
        clip.write_wav(&path, WavFormat::Int16)
            .map_err(|err| err.to_string())?;
        let sound = self.store.import_sound(&path);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&folder);
        sound.map(Some).map_err(|err| err.to_string())
    }
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
    /// 0 to 1, over every track.
    pub progress: f32,
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
    /// The pack, while recorded instruments are on: what voices it has.
    pub pack: Option<&'a Pack>,
}

/// Loads into `recorded` the voice of each instrument the `settings` play
/// that is not loaded as such: the layer's, or the instrument's own where
/// the pack has not that one. A voice that will not load leaves the
/// instrument synthesised, not playing another voice's recordings.
fn load_voices(pack: &Pack, recorded: &mut Recordings, settings: &AmbienceSettings) {
    for instrument in settings.playing() {
        let layer = settings.layer(instrument);
        if let Some(voice) = pack.voice_for(instrument, layer.voice)
            && recorded.voice(instrument) != Some(voice)
        {
            let samples = match pack.load(instrument, voice) {
                Ok(Some(samples)) => Some(Arc::new(samples)),
                _ => None,
            };
            recorded.set(instrument, samples);
        }
    }
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
}

pub fn show(
    ui: &mut Ui,
    settings: &mut AmbienceSettings,
    view: &View,
    naming: &mut Option<Naming>,
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
                        instrument_rows(ui, settings, view.pack);
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
        "Low: a short motif loops again and again, its notes echoing, on few chords, in an \
         even rhythm. High: new melodies, more chords (now and then one borrowed from the \
         parallel key), freer rhythm",
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
    background_chips(ui, &mut settings.background, images);
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

/// Each background as a chip with its picture, the chosen one lit.
fn background_chips(ui: &mut Ui, value: &mut Background, images: &EmojiImages) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for choice in Background::ALL {
            let picture = images.texture(ui.ctx(), emoji(choice));
            if skin::chip(ui, choice.name(), picture, *value == choice)
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
                         sections, bowed vibraphone and hand drums, with other recordings of \
                         the harp, bells, strings and pad to choose from, recorded by \
                         Versilian Studios (public domain)";
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
fn instrument_rows(ui: &mut Ui, settings: &mut AmbienceSettings, pack: Option<&Pack>) {
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
        if let Some(pack) = pack {
            voice_chips(ui, instrument, layer, pack);
        }
        ui.add_space(6.0);
    }
}

/// The recordings an instrument can be played from, when the pack has more
/// than one: the instrument's own and the others of its role (a folk harp
/// for the harp), side by side under its register.
fn voice_chips(ui: &mut Ui, instrument: Instrument, layer: &mut Layer, pack: &Pack) {
    let voices: Vec<(u8, &str)> = pack.voices(instrument).collect();
    if voices.len() < 2 {
        return;
    }
    let chosen = pack.voice_for(instrument, layer.voice);
    ui.horizontal_wrapped(|ui| {
        ui.add_space(INSTRUMENT_COLUMN + ui.spacing().item_spacing.x);
        ui.spacing_mut().item_spacing.x = 6.0;
        for (voice, name) in voices {
            let name = if name.is_empty() {
                instrument.name()
            } else {
                name
            };
            if skin::chip(ui, name, None, chosen == Some(voice))
                .on_hover_text(format!(
                    "Plays the {} from the recording of the {}.",
                    instrument.name().to_lowercase(),
                    name.to_lowercase()
                ))
                .clicked()
            {
                layer.voice = voice;
            }
        }
    });
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
                let percent = busy.progress * 100.0;
                let text = match busy.track {
                    Some((track, count)) => format!(
                        "Generating “{}”: {track} of {count}, {percent:.0} %",
                        busy.title
                    ),
                    None => format!("Generating “{}” {percent:.0} %", busy.title),
                };
                let bar = egui::ProgressBar::new(busy.progress).text(text);
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

    /// A pack with a harp of two voices whose second has no recordings on
    /// disk, and the settings of a style that plays the harp with `voice`.
    fn harp_pack(dir: &std::path::Path, voice: u8) -> (Pack, AmbienceSettings) {
        use olio_audio::instruments::{MANIFEST, Manifest};
        crate::instruments::tests::write_pack(dir);
        let folder = dir.join("olio-instruments");
        let text = fs::read_to_string(folder.join(MANIFEST)).unwrap();
        let mut manifest: Manifest = serde_json::from_str(&text).unwrap();
        let mut broken = manifest.instruments[0].clone();
        broken.voice = 1;
        broken.name = "Broken harp".to_owned();
        for zone in &mut broken.zones {
            zone.file = "harp/missing.wav".to_owned();
        }
        manifest.instruments.push(broken);
        fs::write(
            folder.join(MANIFEST),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let mut settings = Style::RainStudy.settings();
        settings.layers[Instrument::Harp].voice = voice;
        (Pack::open(&folder).unwrap(), settings)
    }

    #[test]
    fn a_track_plays_the_voice_its_settings_ask_for_and_never_another_when_it_will_not_load() {
        let dir = tempfile::tempdir().unwrap();
        let mut recorded = Recordings::default();

        // The instrument's own, loaded for a track that asks for it.
        let (pack, settings) = harp_pack(dir.path(), 0);
        load_voices(&pack, &mut recorded, &settings);
        assert_eq!(recorded.voice(Instrument::Harp), Some(0));

        // A voice the pack has not: the own one stays.
        let (pack, settings) = harp_pack(dir.path(), 7);
        load_voices(&pack, &mut recorded, &settings);
        assert_eq!(recorded.voice(Instrument::Harp), Some(0));

        // A voice the pack lists but cannot read: synthesised, not voice 0.
        let (pack, settings) = harp_pack(dir.path(), 1);
        load_voices(&pack, &mut recorded, &settings);
        assert!(recorded.get(Instrument::Harp).is_none());
    }

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

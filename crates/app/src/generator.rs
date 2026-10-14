//! The Generator: a panel in place of the inspector where the ambience is
//! shaped, played live under the sounds, and rendered to tracks that land
//! on tiles. It shows the essentials (style, mood, how it changes over
//! time, background, instruments); Details adds every setting (the character's amounts,
//! each instrument's level, density and register, the key, presence and
//! form) under the essential it belongs to. Custom settings can be saved
//! as a style of one's own, listed after the built-in styles.
//!
//! The settings live in the preferences and go to the engine when they
//! change. A track is rendered on a thread of its own ([`Generation`]),
//! written as a WAV into the library and placed on a tile by the app.

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use egui::{Align, Key, Layout, RichText, Ui};
use olio_audio::ambience::{
    AmbienceSettings, Background, Density, Form, Instrument, Journey, Mood, Now, Presence,
    ROOT_NAMES, Register, ScaleKind, Style,
};
use olio_audio::{WavFormat, render_track};
use olio_library::{Sound, Store};

use crate::emoji::EmojiImages;
use crate::prefs::{UserStyle, style_name_problem, style_named};
use crate::{inspector, menu};

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
    /// `store`.
    pub fn start(
        ctx: egui::Context,
        store: Store,
        title: String,
        tracks: Vec<TrackPlan>,
        seconds: u32,
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
    progress: Arc<AtomicU32>,
    current: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    fn run(self) -> Made {
        let mut sounds = Vec::new();
        let count = self.tracks.len().max(1) as f32;
        for (index, track) in self.tracks.iter().enumerate() {
            self.current.store(index, Ordering::Relaxed);
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

    /// Renders one track and imports it: `None` if cancelled. `overall`
    /// turns its own progress into the whole generation's.
    fn render(
        &self,
        track: &TrackPlan,
        overall: impl Fn(f32) -> f32,
    ) -> Result<Option<Sound>, String> {
        let mut shown = Instant::now();
        let clip = render_track(&track.settings, self.seconds as f32, |done| {
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
    /// Every setting, not only the essentials.
    pub details: bool,
    /// Styles of one's own, and the one the settings are, if any.
    pub styles: &'a [UserStyle],
    pub saved: Option<usize>,
    /// Where a building or evolving ambience playing live has got to.
    pub now: Option<Now>,
    /// After a surprise: what Back returns to.
    pub before_surprise: Option<&'a str>,
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
    /// Show every setting, or only the essentials.
    Details(bool),
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
}

pub fn show(
    ui: &mut Ui,
    settings: &mut AmbienceSettings,
    view: &View,
    naming: &mut Option<Naming>,
) -> Vec<Action> {
    let before = *settings;
    let mut actions = Vec::new();
    let details = view.details;

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Generator").strong().size(16.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .selectable_label(details, "Details")
                .on_hover_text("Show every setting, not only the essentials")
                .clicked()
            {
                actions.push(Action::Details(!details));
            }
        });
    });

    // The track: pinned to the bottom, always in reach.
    egui::Panel::bottom("olio-generator-track")
        .frame(egui::Frame::NONE)
        .show(ui, |ui| track(ui, view, &mut actions));

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Clear of the scroll bar.
            ui.set_max_width(ui.available_width() - 12.0);
            ui.add_space(8.0);
            live(ui, settings, details);
            let title = section(ui, "Style");
            // One scope whatever it holds, so the widgets below keep
            // their ids (and a focused slider its focus) when the settings
            // turn Custom and Save Style… appears.
            ui.scope(|ui| {
                style(ui, settings, &title, view);
                match naming {
                    Some(naming) => naming_row(ui, naming, view.styles, &mut actions),
                    None => style_actions(ui, settings, view, &mut actions),
                }
            });
            section(ui, "Mood");
            mood(ui, settings, details);
            section(ui, "Over time");
            // One scope, so the line saying where it has got to never
            // moves the ids of the widgets below.
            ui.scope(|ui| over_time(ui, settings, view.now.as_ref()));
            section(ui, "Background");
            background(ui, settings, view.images);
            section(ui, "Instruments");
            part_level(
                ui,
                &mut settings.instruments_level,
                "Instruments level",
                "How loud the instruments are, all together, against the background".to_owned(),
            );
            ui.add_space(4.0);
            if details {
                instrument_rows(ui, settings);
            } else {
                instrument_chips(ui, settings);
            }
            if settings.layers.iter().all(|layer| !layer.enabled) {
                ui.weak("No instruments: the background only.");
            }
            if details {
                section(ui, "Key");
                key(ui, settings);
                section(ui, "Presence");
                presence(ui, settings);
                section(ui, "Form");
                form(ui, settings);
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

/// A section's title, spaced as in the inspector.
fn section(ui: &mut Ui, title: &str) -> egui::Response {
    ui.add_space(16.0);
    let response = ui.label(RichText::new(title).strong());
    ui.add_space(6.0);
    response
}

/// Play live and the level; with details, ducking.
fn live(ui: &mut Ui, settings: &mut AmbienceSettings, details: bool) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut settings.enabled, "Play live")
            .on_hover_text("Play the ambience now, under your sounds");
        ui.add_space(8.0);
        let mut percent = settings.level * 100.0;
        inspector::fill_with_slider(ui);
        let slider = egui::Slider::new(&mut percent, 0.0..=100.0)
            .suffix(" %")
            .max_decimals(0);
        if menu::keeps_arrows(ui.add(slider))
            .on_hover_text("Level. Even at full level the ambience stays well under the sounds")
            .changed()
        {
            settings.level = percent / 100.0;
        }
    });
    if details {
        ui.checkbox(&mut settings.duck, "Quieter while sounds play");
    }
}

/// The style to start from, built-in or one's own, a new variation of
/// it, and what it is.
fn style(ui: &mut Ui, settings: &mut AmbienceSettings, title: &egui::Response, view: &View) {
    let saved = view.saved.and_then(|index| view.styles.get(index));
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 112.0).max(120.0);
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
        let _ = combo.response.labelled_by(title.id);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .button("New variation")
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
        (None, None) => format!(
            "{}: {}.",
            settings.scale.name(),
            settings.scale.kind.character()
        ),
    });
}

/// Custom settings can be saved as a style; a saved one renamed or
/// removed.
/// Surprise Me, and Back after one; Custom settings can be saved as a
/// style, a saved one renamed or removed.
fn style_actions(ui: &mut Ui, settings: &AmbienceSettings, view: &View, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let dice = match view.images.texture(ui.ctx(), "🎲") {
            Some(texture) => {
                let picture = egui::Image::from_texture(egui::load::SizedTexture::new(
                    texture,
                    egui::vec2(16.0, 16.0),
                ));
                egui::Button::new((picture, "Surprise Me"))
            }
            None => egui::Button::new("Surprise Me"),
        };
        if ui
            .add(dice)
            .on_hover_text(
                "Something new, playing now: a key, a background, a few instruments and a \
                 mood picked at random",
            )
            .clicked()
        {
            actions.push(Action::Surprise);
        }
        if let Some(name) = view.before_surprise
            && ui
                .button("Back")
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
                if ui.button("Rename…").clicked() {
                    actions.push(Action::RenameStyle(index));
                }
                if ui
                    .button("Remove Style")
                    .on_hover_text("Tracks made from it stay")
                    .clicked()
                {
                    actions.push(Action::RemoveStyle(index));
                }
            }
            None => {
                if ui
                    .button("Save Style…")
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
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 130.0).max(100.0);
        let edit = egui::TextEdit::singleline(&mut naming.text)
            .hint_text("Style name")
            .desired_width(width);
        let response = ui.add(edit);
        if !naming.focused {
            response.request_focus();
            naming.focused = true;
        }
        let entered = response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        let save = ui
            .add_enabled(problem.is_none(), egui::Button::new("Save"))
            .clicked();
        let cancel = ui.button("Cancel").clicked();
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
    choose(
        ui,
        &mut settings.journey,
        &Journey::ALL,
        Journey::name,
        Journey::describe,
    );
    ui.add_space(2.0);
    ui.weak(sentence(settings.journey.describe()));
    if let Some(now) = now {
        ui.label(now_words(now));
    }
}

/// The moods; with details, the three amounts behind them.
fn mood(ui: &mut Ui, settings: &mut AmbienceSettings, details: bool) {
    let now = settings.mood();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.spacing_mut().button_padding.x = 6.0;
        for mood in Mood::ALL {
            if ui
                .selectable_label(now == Some(mood), mood.name())
                .on_hover_text(mood.describe())
                .clicked()
            {
                settings.set_mood(mood);
            }
        }
        if now.is_none() {
            let _ = ui
                .selectable_label(true, "Custom")
                .on_hover_text("Pick a mood to start over from one");
        }
    });
    if details {
        ui.add_space(4.0);
        character(ui, settings);
    }
    ui.add_space(2.0);
    ui.weak(character_words(settings));
}

/// A row of choices, wrapping; true when one was picked.
fn choose<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    all: &[T],
    name: impl Fn(T) -> &'static str,
    hint: impl Fn(T) -> &'static str,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for &choice in all {
            let response = ui.selectable_label(*value == choice, name(choice));
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
        let _ = combo.response.labelled_by(root.id);
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
        let _ = combo.response.labelled_by(scale.id);
    });
}

fn character(ui: &mut Ui, settings: &mut AmbienceSettings) {
    egui::Grid::new("olio-generator-character")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 170.0).max(100.0);
            amount(
                ui,
                "Variety",
                &mut settings.variety,
                "Low: a short motif comes back again and again, on few chords, in an even \
                 rhythm. High: new melodies, more chords, freer rhythm",
            );
            amount(
                ui,
                "Concentration",
                &mut settings.concentration,
                "High: the notes gather around the chord and the home note, in small steps \
                 and a narrow range. Low: they wander wide",
            );
            amount(
                ui,
                "Chaos",
                &mut settings.chaos,
                "Dissonance: notes from outside the key, clashing seconds and tritones, \
                 chords from anywhere and looser timing, more as it rises",
            );
        });
}

/// A grid row: `name` and a slider showing the 0-to-1 `value` in percent.
fn amount(ui: &mut Ui, name: &str, value: &mut f32, hint: &str) {
    let label = ui.label(name).on_hover_text(hint);
    let mut percent = *value * 100.0;
    let slider = egui::Slider::new(&mut percent, 0.0..=100.0)
        .suffix(" %")
        .max_decimals(0);
    if menu::keeps_arrows(ui.add(slider))
        .labelled_by(label.id)
        .on_hover_text(hint)
        .changed()
    {
        *value = percent / 100.0;
    }
    ui.end_row();
}

fn background(ui: &mut Ui, settings: &mut AmbienceSettings, images: &EmojiImages) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for choice in Background::ALL {
            let chosen = settings.background == choice;
            let button = match images.texture(ui.ctx(), emoji(choice)) {
                Some(texture) => {
                    let picture = egui::Image::from_texture(egui::load::SizedTexture::new(
                        texture,
                        egui::vec2(16.0, 16.0),
                    ));
                    egui::Button::selectable(chosen, (picture, choice.name()))
                }
                None => egui::Button::selectable(chosen, choice.name()),
            };
            if ui.add(button).on_hover_text(choice.describe()).clicked() {
                settings.background = choice;
            }
        }
    });
    ui.weak(settings.background.describe());
    // Its own level, against the instruments: in one scope, so whether it
    // shows never moves the ids of the widgets below.
    ui.scope(|ui| {
        if settings.background == Background::None {
            return;
        }
        ui.add_space(2.0);
        let hint = format!(
            "How loud the {} is against the instruments",
            settings.background.name().to_lowercase()
        );
        part_level(ui, &mut settings.background_level, "Background level", hint);
    });
}

/// A part's level (the background's, or the instruments' together):
/// "Level" and a slider in percent filling the row, named `label` for
/// screen readers.
fn part_level(ui: &mut Ui, value: &mut f32, label: &str, hint: String) {
    ui.horizontal(|ui| {
        ui.label("Level");
        let mut percent = *value * 100.0;
        inspector::fill_with_slider(ui);
        let slider = egui::Slider::new(&mut percent, 0.0..=100.0)
            .suffix(" %")
            .max_decimals(0);
        let response = menu::keeps_arrows(ui.add(slider));
        response.widget_info(|| egui::WidgetInfo::slider(true, f64::from(percent), label));
        if response.on_hover_text(hint).changed() {
            *value = percent / 100.0;
        }
    });
}

/// Each instrument on or off, side by side.
fn instrument_chips(ui: &mut Ui, settings: &mut AmbienceSettings) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for instrument in Instrument::ALL {
            let layer = &mut settings.layers[instrument];
            if ui
                .selectable_label(layer.enabled, instrument.name())
                .on_hover_text(instrument.describe())
                .clicked()
            {
                layer.enabled = !layer.enabled;
            }
        }
    });
}

/// The width of the instruments' name column, so their sliders line up.
const NAME_WIDTH: f32 = 84.0;

/// Each instrument on or off with its level; one that is on also shows
/// how many notes it plays (the drone holds one) and its register.
fn instrument_rows(ui: &mut Ui, settings: &mut AmbienceSettings) {
    for instrument in Instrument::ALL {
        let seconds = settings.spacing(instrument);
        let layer = &mut settings.layers[instrument];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let name = ui.add_sized(
                [NAME_WIDTH, 20.0],
                egui::Checkbox::new(&mut layer.enabled, instrument.name()),
            );
            name.on_hover_text(instrument.describe());
            ui.add_enabled_ui(layer.enabled, |ui| {
                let hint = format!("Level {:.0} %", layer.level * 100.0);
                let label = format!("{} level", instrument.name());
                amount_slider(ui, &mut layer.level, &label, hint);
            });
        });
        if !layer.enabled {
            continue;
        }
        if instrument != Instrument::Drone {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.allocate_ui_with_layout(
                    egui::vec2(NAME_WIDTH, 18.0),
                    Layout::right_to_left(Align::Center),
                    |ui| ui.weak("Notes"),
                );
                let label = format!("{} notes", instrument.name());
                let hint = format!(
                    "Notes: {}, about one every {seconds:.0} s. Fewer to the left, more to the \
                     right",
                    layer.density.name()
                );
                let mut amount = layer.density.amount();
                if amount_slider(ui, &mut amount, &label, hint) {
                    layer.density = Density::new(amount);
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add_space(NAME_WIDTH + 6.0);
            ui.spacing_mut().button_padding = egui::vec2(6.0, 2.0);
            row(ui, &mut layer.register, &Register::ALL, Register::name);
        });
        ui.add_space(4.0);
    }
}

/// A slider without a value box for a 0-to-1 `value`, as wide as the
/// row allows, named `label` for screen readers and reading in percent;
/// true when it moved.
fn amount_slider(ui: &mut Ui, value: &mut f32, label: &str, hint: String) -> bool {
    let mut percent = *value * 100.0;
    ui.spacing_mut().slider_width = (ui.available_width() - 8.0).clamp(60.0, 200.0);
    let slider = egui::Slider::new(&mut percent, 0.0..=100.0)
        .show_value(false)
        .trailing_fill(true);
    let response = menu::keeps_arrows(ui.add(slider));
    let enabled = ui.is_enabled();
    response.widget_info(|| egui::WidgetInfo::slider(enabled, f64::from(percent), label));
    let moved = response.on_hover_text(hint).changed();
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
) {
    ui.spacing_mut().item_spacing.x = 2.0;
    for &choice in all {
        if ui
            .selectable_label(*value == choice, name(choice))
            .clicked()
        {
            *value = choice;
        }
    }
}

fn presence(ui: &mut Ui, settings: &mut AmbienceSettings) {
    choose(
        ui,
        &mut settings.presence,
        &Presence::ALL,
        Presence::name,
        Presence::describe,
    );
    if settings.presence.pulsed() {
        ui.horizontal(|ui| {
            ui.label("Pulse");
            let mut tempo = f32::from(settings.tempo);
            ui.spacing_mut().slider_width = 160.0;
            let slider = egui::Slider::new(&mut tempo, 30.0..=120.0)
                .suffix(" BPM")
                .max_decimals(0);
            if menu::keeps_arrows(ui.add(slider)).changed() {
                settings.tempo = tempo.round() as u16;
            }
        });
    }
    ui.weak(settings.presence.describe());
}

fn form(ui: &mut Ui, settings: &mut AmbienceSettings) {
    let pieces = matches!(settings.form, Form::Piece { .. });
    ui.horizontal(|ui| {
        if ui.selectable_label(!pieces, "Endless").clicked() && pieces {
            settings.form = Form::Endless;
        }
        if ui
            .selectable_label(pieces, "Pieces")
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
        egui::Grid::new("olio-generator-form")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.spacing_mut().slider_width = 160.0;
                ui.label("Length");
                let length = egui::Slider::new(minutes, 0.5..=30.0)
                    .suffix(" min")
                    .max_decimals(1);
                menu::keeps_arrows(ui.add(length));
                ui.end_row();
                ui.label("Pause");
                let pause = egui::Slider::new(pause_seconds, 0.0..=120.0)
                    .suffix(" s")
                    .max_decimals(0);
                menu::keeps_arrows(ui.add(pause));
                ui.end_row();
            });
    }
}

fn track(ui: &mut Ui, view: &View, actions: &mut Vec<Action>) {
    ui.add_space(4.0);
    ui.separator();
    ui.label(RichText::new("Track").strong());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label("Length");
        for (seconds, name) in TRACK_LENGTHS {
            if ui
                .selectable_label(view.track_seconds == seconds, name)
                .clicked()
            {
                actions.push(Action::Length(seconds));
            }
        }
    });
    ui.add_space(2.0);
    match &view.busy {
        Some(busy) => {
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
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
                ui.add(egui::ProgressBar::new(busy.progress).text(text));
            });
        }
        None => {
            let generate = egui::Button::new("Generate Track")
                .min_size(egui::vec2(ui.available_width(), 28.0));
            if ui
                .add(generate)
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
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label("New set of");
                for of in SetOf::ALL {
                    if ui.button(of.name()).on_hover_text(of.describe()).clicked() {
                        actions.push(Action::GenerateSet(of));
                    }
                }
            });
        }
    }
    ui.add_space(6.0);
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

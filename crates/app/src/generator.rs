//! The Generator: a panel in place of the inspector where the ambience is
//! shaped (style, key, background, instruments, presence, form), played
//! live under the sounds, and rendered to tracks that land on tiles.
//!
//! The settings live in the preferences and go to the engine when they
//! change. A track is rendered on a thread of its own ([`Generation`]),
//! written as a WAV into the library and placed on a tile by the app.

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use egui::{Align, Layout, RichText, Ui};
use olio_audio::ambience::{
    AmbienceSettings, Background, Density, Form, Instrument, Presence, ROOT_NAMES, Register,
    ScaleKind, Style,
};
use olio_audio::{WavFormat, render_track};
use olio_library::{Sound, Store};

use crate::emoji::EmojiImages;

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

/// A track's title: its style, or its key when Custom, and its length:
/// "Rain Study 2 min", "D Dorian 30 s".
pub fn track_title(settings: &AmbienceSettings, seconds: u32) -> String {
    let name = match settings.style {
        Some(style) => style.name().to_owned(),
        None => settings.scale.name(),
    };
    format!("{name} {}", length_name(seconds))
}

// ----- generating a track -------------------------------------------------

/// A track being rendered on its own thread.
pub struct Generation {
    pub title: String,
    pub emoji: &'static str,
    /// The set it goes to.
    pub set: olio_library::SetId,
    /// How far, as `f32` bits, 0 to 1.
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    done: mpsc::Receiver<Result<Option<Sound>, String>>,
}

impl Generation {
    /// Starts rendering `seconds` of the ambience with `settings` into the
    /// library in `store`, for the set `set`.
    pub fn start(
        ctx: egui::Context,
        store: Store,
        settings: AmbienceSettings,
        seconds: u32,
        set: olio_library::SetId,
    ) -> Self {
        let title = track_title(&settings, seconds);
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, done) = mpsc::channel();
        let job = Job {
            ctx: ctx.clone(),
            store,
            settings,
            seconds,
            title: title.clone(),
            progress: Arc::clone(&progress),
            cancel: Arc::clone(&cancel),
        };
        let spawned = thread::Builder::new()
            .name("olio-generator".to_owned())
            .spawn(move || {
                let result = job.run();
                let _ = sender.send(result);
                ctx.request_repaint();
            });
        if let Err(err) = spawned {
            // The receiver reports the thread as gone, with this reason.
            eprintln!("olio: could not start the generator: {err}");
        }
        Self {
            title,
            emoji: emoji(settings.background),
            set,
            progress,
            cancel,
            done,
        }
    }

    /// 0 to 1.
    pub fn progress(&self) -> f32 {
        f32::from_bits(self.progress.load(Ordering::Relaxed))
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The result once there is one: the imported sound, `None` if it was
    /// cancelled, or what went wrong.
    pub fn poll(&self) -> Option<Result<Option<Sound>, String>> {
        match self.done.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the generator stopped unexpectedly".to_owned()))
            }
        }
    }
}

struct Job {
    ctx: egui::Context,
    store: Store,
    settings: AmbienceSettings,
    seconds: u32,
    title: String,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    fn run(self) -> Result<Option<Sound>, String> {
        let mut shown = Instant::now();
        let clip = render_track(&self.settings, self.seconds as f32, |done| {
            self.progress.store(done.to_bits(), Ordering::Relaxed);
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
        let name: String = self
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

/// What the panel shows besides the settings.
pub struct View<'a> {
    pub images: &'a EmojiImages,
    /// The track being generated, if any: its title and progress.
    pub busy: Option<(&'a str, f32)>,
    pub track_seconds: u32,
    /// The set a track would go to.
    pub set_name: &'a str,
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
    Cancel,
}

pub fn show(ui: &mut Ui, settings: &mut AmbienceSettings, view: &View) -> Vec<Action> {
    let before = *settings;
    let mut actions = Vec::new();

    ui.add_space(6.0);
    ui.label(RichText::new("Generator").strong().size(15.0));
    ui.label(
        RichText::new(
            "Gentle music and backgrounds, made up as they go. Play them live under \
             your sounds, or generate a track onto a tile.",
        )
        .weak(),
    );
    ui.add_space(6.0);

    // The track: pinned to the bottom, always in reach.
    egui::Panel::bottom("olio-generator-track")
        .frame(egui::Frame::NONE)
        .show(ui, |ui| track(ui, view, &mut actions));

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Clear of the scroll bar.
            ui.set_max_width(ui.available_width() - 12.0);
            live(ui, settings);
            ui.add_space(6.0);
            styles(ui, settings);
            ui.add_space(2.0);
            ui.label(
                RichText::new(format!(
                    "{}: {}.",
                    settings.scale.name(),
                    settings.scale.kind.character()
                ))
                .weak(),
            );
            ui.add_space(4.0);
            section(ui, "Key", |ui| key(ui, settings));
            section(ui, "Background", |ui| background(ui, settings, view.images));
            section(ui, "Instruments", |ui| instruments(ui, settings));
            section(ui, "Presence", |ui| presence(ui, settings));
            section(ui, "Form", |ui| form(ui, settings));
            ui.add_space(8.0);
        });

    // Anything but picking a style, switching on or a new variation makes
    // the settings Custom.
    let touched = AmbienceSettings {
        enabled: before.enabled,
        seed: before.seed,
        style: before.style,
        ..*settings
    } != before;
    if touched && settings.style == before.style {
        settings.style = None;
    }
    if *settings != before {
        actions.push(Action::Changed);
    }
    actions
}

fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .id_salt(("olio-generator", title))
        .default_open(true)
        .show(ui, |ui| {
            ui.add_space(2.0);
            body(ui);
            ui.add_space(4.0);
        });
}

fn live(ui: &mut Ui, settings: &mut AmbienceSettings) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut settings.enabled, "Play live")
            .on_hover_text("Play the ambience now, under your sounds");
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
    ui.horizontal_wrapped(|ui| {
        ui.label("Level");
        let mut percent = settings.level * 100.0;
        ui.spacing_mut().slider_width = 100.0;
        if ui
            .add(
                egui::Slider::new(&mut percent, 0.0..=100.0)
                    .suffix(" %")
                    .max_decimals(0),
            )
            .on_hover_text("Even at full level the ambience stays well under the sounds")
            .changed()
        {
            settings.level = percent / 100.0;
        }
        ui.checkbox(&mut settings.duck, "Quieter while sounds play");
    });
}

fn styles(ui: &mut Ui, settings: &mut AmbienceSettings) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label("Style");
        for style in Style::ALL {
            let chosen = settings.style == Some(style);
            if ui
                .selectable_label(chosen, style.name())
                .on_hover_text(style.describe())
                .clicked()
                && !chosen
            {
                // Everything from the style; whether it plays stays.
                let enabled = settings.enabled;
                *settings = style.settings();
                settings.enabled = enabled;
            }
        }
        if settings.style.is_none() {
            let _ = ui
                .selectable_label(true, "Custom")
                .on_hover_text("Pick a style to start over from one");
        }
    });
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
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.spacing_mut().button_padding.x = 5.0;
        ui.label("Root");
        for (index, name) in ROOT_NAMES.iter().enumerate() {
            let root = index as u8;
            if ui
                .selectable_label(settings.scale.root == root, *name)
                .clicked()
            {
                settings.scale.root = root;
            }
        }
    });
    ui.add_space(4.0);
    choose(
        ui,
        &mut settings.scale.kind,
        &ScaleKind::all(),
        ScaleKind::name,
        ScaleKind::character,
    );
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
}

fn instruments(ui: &mut Ui, settings: &mut AmbienceSettings) {
    for instrument in Instrument::ALL {
        let layer = &mut settings.layers[instrument];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let name = ui.add_sized(
                [84.0, 20.0],
                egui::Checkbox::new(&mut layer.enabled, instrument.name()),
            );
            name.on_hover_text(instrument.describe());
            ui.add_enabled_ui(layer.enabled, |ui| {
                let mut percent = layer.level * 100.0;
                ui.spacing_mut().slider_width = (ui.available_width() - 8.0).clamp(60.0, 200.0);
                if ui
                    .add(
                        egui::Slider::new(&mut percent, 0.0..=100.0)
                            .show_value(false)
                            .trailing_fill(true),
                    )
                    .on_hover_text(format!("Level {percent:.0} %"))
                    .changed()
                {
                    layer.level = percent / 100.0;
                }
            });
        });
        if layer.enabled {
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.spacing_mut().button_padding = egui::vec2(6.0, 2.0);
                if instrument != Instrument::Drone {
                    row(ui, &mut layer.density, &Density::ALL, Density::name);
                    ui.add_space(8.0);
                }
                row(ui, &mut layer.register, &Register::ALL, Register::name);
            });
            ui.add_space(2.0);
        }
    }
    if settings.layers.iter().all(|layer| !layer.enabled) {
        ui.weak("No instruments: the background only.");
    }
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
            if ui
                .add(
                    egui::Slider::new(&mut tempo, 30.0..=120.0)
                        .suffix(" BPM")
                        .max_decimals(0),
                )
                .changed()
            {
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
                ui.add(
                    egui::Slider::new(minutes, 0.5..=30.0)
                        .suffix(" min")
                        .max_decimals(1),
                );
                ui.end_row();
                ui.label("Pause");
                ui.add(
                    egui::Slider::new(pause_seconds, 0.0..=120.0)
                        .suffix(" s")
                        .max_decimals(0),
                );
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
    match view.busy {
        Some((title, progress)) => {
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    actions.push(Action::Cancel);
                }
                ui.add(
                    egui::ProgressBar::new(progress)
                        .text(format!("Generating “{title}” {:.0} %", progress * 100.0)),
                );
            });
        }
        None => {
            ui.horizontal(|ui| {
                if ui
                    .button("Generate Track")
                    .on_hover_text(
                        "Renders this ambience to a sound in the library and puts it on a \
                         tile, to play, loop and polish like any other",
                    )
                    .clicked()
                {
                    actions.push(Action::Generate);
                }
                ui.weak(format!("onto a tile in “{}”", view.set_name));
            });
        }
    }
    ui.add_space(6.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_are_titled_by_style_or_key_and_length() {
        let mut settings = Style::RainStudy.settings();
        assert_eq!(track_title(&settings, 120), "Rain Study 2 min");
        settings.style = None;
        assert_eq!(track_title(&settings, 30), "D Dorian 30 s");
        assert_eq!(length_name(180), "3 min");
        for background in Background::ALL {
            assert!(!emoji(background).is_empty());
        }
    }
}

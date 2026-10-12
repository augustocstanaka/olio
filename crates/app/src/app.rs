//! The main window: what it holds and how a frame runs.
//!
//! Every playback request, from a key, a click, a menu or (later) a
//! controller, is an `olio_control::Command` carried out by
//! [`OlioApp::execute`]. Requests that only concern this window (show a
//! panel, open a dialog) are `MenuAction`s, carried out by `act`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{
    Align, CornerRadius, Event, Key, Layout, RichText, Ui, UserData, ViewportCommand, WidgetInfo,
    WidgetType,
};
use olio_audio::{Engine, Mixer, PlayOptions, VoiceId};
use olio_control::{Command, TileKey, TileSetting};
use olio_library::{
    Library, Lock, MAX_FADE_SECONDS, Set, SetId, Slot, Sound, Store, Tile, TileColor, TileId,
};

use crate::Options;
use crate::clips::{ClipState, Clips};
use crate::confirm::{self, Confirm, Pending, Reply};
use crate::emoji::{self, EmojiImages};
use crate::generator::{self, Generation};
use crate::grid::{self, GridLayout, GridView, Progress};
use crate::import::{Importer, Report};
use crate::inspector::{self, InspectorAction};
use crate::menu::{self, Direction, MenuAction, MenuState};
use crate::meter::{self, Meter};
use crate::prefs::{Polish, Preferences, TileStyle};
use crate::sidebar::{self, Rename, SidebarAction};
use crate::undo::{Change, History};
use crate::{keys, palette, polish};

/// Escape must be held this long to stop everything, so a stray tap does
/// not.
const STOP_ALL_HOLD: Duration = Duration::from_millis(300);
/// How long Fade All takes.
const FADE_ALL: Duration = Duration::from_secs(2);
/// How long a status message stays up; mistakes stay longer.
const STATUS_TIME: Duration = Duration::from_secs(6);
const ERROR_TIME: Duration = Duration::from_secs(12);
/// How often to try opening the audio output again after it failed.
const REOPEN_INTERVAL: Duration = Duration::from_secs(2);
/// Redraw interval while sounds play, for smooth progress.
const PLAYING_REPAINT: Duration = Duration::from_millis(33);
/// Edits are saved this long after the last one, so dragging a slider does
/// not write the library every frame.
const SAVE_DELAY: Duration = Duration::from_millis(500);
const WEBSITE: &str = "https://github.com/augustocstanaka/olio";

/// A tile that is sounding.
struct Playing {
    voice: VoiceId,
    set: SetId,
    /// The region being played, in clip frames.
    start: u64,
    length: u64,
    rate: u32,
}

struct Status {
    text: String,
    error: bool,
    /// Show an Undo button beside it: the status reports a removal.
    undo: bool,
    since: Instant,
}

impl Status {
    fn remaining(&self) -> Option<Duration> {
        let time = if self.error { ERROR_TIME } else { STATUS_TIME };
        time.checked_sub(self.since.elapsed())
    }
}

enum Audio {
    // Boxed: with WASAPI's stream inside, the engine is far larger than the
    // other variant.
    Running(Box<Engine>),
    Unavailable { reason: String, retry_at: Instant },
}

fn open_audio() -> Audio {
    match Engine::open_default_output() {
        Ok(engine) => Audio::Running(Box::new(engine)),
        Err(err) => Audio::Unavailable {
            reason: err.to_string(),
            retry_at: Instant::now() + REOPEN_INTERVAL,
        },
    }
}

/// An output with no device: the window renders the mixer itself, as much
/// as the clock has moved on each frame, and throws the sound away.
struct SilentOutput {
    mixer: Mixer,
    last: Option<f64>,
    buffer: Vec<f32>,
}

impl SilentOutput {
    const RATE: u32 = 48_000;

    fn render(&mut self, now: f64) {
        let elapsed = self.last.map_or(0.0, |last| (now - last).clamp(0.0, 1.0));
        self.last = Some(now);
        let frames = (elapsed * f64::from(Self::RATE)).round() as usize;
        self.buffer.resize(frames * 2, 0.0);
        self.mixer.render(&mut self.buffer, 2);
    }
}

/// A pending `--screenshot`: capture the window once it has settled, write
/// a PNG, and quit.
struct ScreenshotJob {
    path: PathBuf,
    frames: u32,
    requested: bool,
}

pub struct OlioApp {
    store: Store,
    _lock: Option<Lock>,
    library: Library,
    /// Why the library on disk could not be read. While set nothing is
    /// saved, so the file is never replaced by an empty library.
    load_error: Option<String>,
    prefs: Preferences,
    current: SetId,
    selected: Option<TileId>,
    /// Scroll the grid to the selected tile this frame (it was chosen
    /// from the keyboard, so it may be out of view).
    reveal_selected: bool,
    audio: Audio,
    clips: Clips,
    playing: HashMap<TileId, Playing>,
    /// When the library was first changed since it was last saved.
    unsaved_since: Option<Instant>,
    status: Option<Status>,
    escape_down_since: Option<Instant>,
    escape_fired: bool,
    rename: Option<Rename>,
    /// The inspector's title field while it is being edited.
    title_draft: Option<(TileId, String)>,
    /// Give the inspector's title field the focus this frame.
    focus_title: bool,
    confirm: Option<Confirm>,
    show_shortcuts: bool,
    show_polish: bool,
    /// A track the Generator is rendering.
    generation: Option<Generation>,
    history: History,
    /// The set in front when a dragged tile first opened another set by
    /// being held over it; the window goes back to it if the drag ends
    /// without a drop.
    spring_origin: Option<SetId>,
    emoji_picker: Option<emoji::Picker>,
    images: EmojiImages,
    /// The output's left and right levels.
    output_meters: [Meter; 2],
    /// Levels of tiles playing or just stopped.
    tile_meters: HashMap<TileId, Meter>,
    /// When the meters were last fed, in egui's seconds.
    meter_time: f64,
    /// With `--silent` (tests), the mixer the window renders itself.
    silent: Option<SilentOutput>,
    grid_layout: Option<GridLayout>,
    screenshot: Option<ScreenshotJob>,
}

impl OlioApp {
    pub fn new(cc: &eframe::CreationContext<'_>, store: Store, options: Options) -> Self {
        crate::fonts::install(&cc.egui_ctx);
        cc.egui_ctx.all_styles_mut(|style| {
            style.spacing.button_padding = egui::vec2(12.0, 6.0);
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.interact_size.y = 28.0;
            style.visuals.slider_trailing_fill = true;
            let widgets = &mut style.visuals.widgets;
            for visuals in [
                &mut widgets.noninteractive,
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
                &mut widgets.open,
            ] {
                visuals.corner_radius = CornerRadius::same(6);
            }
            style.visuals.menu_corner_radius = CornerRadius::same(8);
            style.visuals.window_corner_radius = CornerRadius::same(10);
        });
        let prefs = Preferences::load(cc.storage);
        cc.egui_ctx.set_theme(prefs.theme.preference());

        let (mut library, load_error) = match store.load() {
            Ok(library) => (library, None),
            Err(err) => (Library::default(), Some(err.to_string())),
        };
        if library.sets.is_empty() {
            library.sets.push(Set::new("My Sounds"));
        }
        let lock = store.lock().ok();
        let mut app = Self {
            current: library.sets[0].id,
            library,
            load_error,
            _lock: lock,
            store,
            prefs,
            selected: None,
            reveal_selected: false,
            audio: Audio::Unavailable {
                reason: String::new(),
                retry_at: Instant::now(),
            },
            silent: None,
            clips: Clips::new(cc.egui_ctx.clone()),
            playing: HashMap::new(),
            unsaved_since: None,
            status: None,
            escape_down_since: None,
            escape_fired: false,
            rename: None,
            title_draft: None,
            focus_title: false,
            confirm: None,
            show_shortcuts: false,
            show_polish: false,
            generation: None,
            history: History::default(),
            spring_origin: None,
            emoji_picker: None,
            images: EmojiImages::default(),
            output_meters: [Meter::default(); 2],
            tile_meters: HashMap::new(),
            meter_time: 0.0,
            grid_layout: None,
            screenshot: options.screenshot.map(|path| ScreenshotJob {
                path,
                frames: 0,
                requested: false,
            }),
        };
        if options.silent {
            let (engine, mixer) = Engine::offline(SilentOutput::RATE);
            app.audio = Audio::Running(Box::new(engine));
            app.silent = Some(SilentOutput {
                mixer,
                last: None,
                buffer: Vec::new(),
            });
        } else {
            app.audio = open_audio();
        }
        app.apply_master_gain();
        app.apply_ambience();
        if let Some(name) = &options.set {
            match app.library.sets.iter().find(|set| &set.name == name) {
                Some(set) => app.current = set.id,
                None => app.set_status(format!("There is no set called “{name}”"), true),
            }
        }
        if !options.import.is_empty() {
            app.import(&options.import, None);
        }
        app
    }

    /// True while any sound of the front set is still being decoded.
    pub fn is_loading(&self) -> bool {
        self.current_set()
            .tiles
            .iter()
            .any(|tile| self.clips.is_loading(&tile.sound.hash))
    }

    /// Titles of the tiles playing now, sorted.
    pub fn playing_titles(&self) -> Vec<String> {
        let mut titles: Vec<String> = self
            .playing
            .keys()
            .filter_map(|&id| self.library.tile(id))
            .map(|(_, tile)| tile.title.clone())
            .collect();
        titles.sort();
        titles
    }

    pub fn current_set_name(&self) -> &str {
        &self.current_set().name
    }

    /// The selected tile's title.
    pub fn selected_title(&self) -> Option<&str> {
        self.selected
            .and_then(|id| self.library.tile(id))
            .map(|(_, tile)| tile.title.as_str())
    }

    pub fn preferences(&self) -> &Preferences {
        &self.prefs
    }

    /// The louder of the output meter's two channels, in dBFS.
    pub fn output_level_db(&self) -> f32 {
        self.output_meters
            .iter()
            .map(Meter::level_db)
            .fold(f32::NEG_INFINITY, f32::max)
    }

    /// The level meter of the tile with this title, in dBFS, while it has
    /// one.
    pub fn tile_level_db(&self, title: &str) -> Option<f32> {
        self.tile_meters
            .iter()
            .find(|(id, _)| {
                self.library
                    .tile(**id)
                    .is_some_and(|(_, tile)| tile.title == title)
            })
            .map(|(_, meter)| meter.level_db())
    }

    /// The selected tile, as it is now.
    pub fn selected_tile(&self) -> Option<Tile> {
        self.selected
            .and_then(|id| self.library.tile(id))
            .map(|(_, tile)| tile.clone())
    }

    fn current_set(&self) -> &Set {
        self.library
            .set(self.current)
            .unwrap_or(&self.library.sets[0])
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some(Status {
            text: text.into(),
            error,
            undo: false,
            since: Instant::now(),
        });
    }

    /// A status for something that was removed, with an Undo button.
    fn set_undo_status(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            error: false,
            undo: true,
            since: Instant::now(),
        });
    }

    fn changed(&mut self) {
        self.unsaved_since.get_or_insert_with(Instant::now);
    }

    fn apply_ambience(&mut self) {
        if let Audio::Running(engine) = &mut self.audio {
            engine.set_ambience(self.prefs.ambience);
        }
    }

    fn apply_master_gain(&mut self) {
        if let Audio::Running(engine) = &mut self.audio {
            engine.set_master_gain(self.prefs.master_gain());
        }
    }

    // ----- commands -------------------------------------------------------

    /// Carries out a command from any control surface.
    fn execute(&mut self, command: Command) {
        match command {
            Command::ToggleTile(id) => {
                if self.playing.contains_key(&id) {
                    self.stop_tile(id);
                } else {
                    self.play_tile(id);
                }
            }
            Command::PlayTile(id) => {
                self.stop_tile(id);
                self.play_tile(id);
            }
            Command::StopTile(id) => self.stop_tile(id),
            Command::StopAll => self.stop_all(Duration::ZERO),
            Command::FadeAll => self.stop_all(FADE_ALL),
            Command::SetAmbience(on) => {
                self.prefs.ambience.enabled = on;
                self.apply_ambience();
            }
            Command::SetMasterVolume(volume) => {
                self.prefs.volume = volume.clamp(0.0, 1.0);
                self.apply_master_gain();
            }
            Command::SelectSet(id) => {
                if self.library.set(id).is_some() && id != self.current {
                    self.current = id;
                    self.selected = None;
                    self.rename = None;
                }
            }
            Command::SetTile(id, setting) => self.set_tile(id, setting),
        }
    }

    fn play_tile(&mut self, id: TileId) {
        let Some((set, tile)) = self.library.tile(id) else {
            return;
        };
        let set_id = set.id;
        let title = tile.title.clone();
        let Audio::Running(engine) = &mut self.audio else {
            self.set_status(format!("Can't play “{title}”: no audio output"), true);
            return;
        };
        let started = match self.clips.get(&tile.sound.hash) {
            Some(ClipState::Ready(ready)) => {
                let region = ready.region(tile.trim_silence);
                let options = PlayOptions {
                    gain: ready.gain(tile.normalize, tile.trim_silence),
                    looping: tile.looping,
                    region: Some(region.clone()),
                    fade_in: Duration::from_secs_f32(tile.fade_in_seconds),
                    fade_out: Duration::from_secs_f32(tile.fade_out_seconds),
                    reverb: tile.reverb,
                };
                engine
                    .play(Arc::clone(&ready.clip), options)
                    .map(|voice| Playing {
                        voice,
                        set: set_id,
                        start: region.start as u64,
                        length: region.len() as u64,
                        rate: ready.clip.rate(),
                    })
                    .ok_or_else(|| "Too many sounds are playing at once".to_owned())
            }
            Some(ClipState::Failed(message)) => Err(format!("Can't play “{title}”: {message}")),
            Some(ClipState::Loading) | None => Err(format!("“{title}” is still loading")),
        };
        match started {
            Ok(playing) => {
                self.playing.insert(id, playing);
            }
            Err(message) => self.set_status(message, true),
        }
    }

    /// Stops a tile with its fade-out (a few milliseconds at least).
    fn stop_tile(&mut self, id: TileId) {
        let Some(playing) = self.playing.remove(&id) else {
            return;
        };
        let fade = self
            .library
            .tile(id)
            .map_or(0.0, |(_, tile)| tile.fade_out_seconds);
        if let Audio::Running(engine) = &mut self.audio {
            engine.stop(playing.voice, Duration::from_secs_f32(fade));
        }
    }

    fn stop_all(&mut self, fade: Duration) {
        if let Audio::Running(engine) = &mut self.audio {
            engine.stop_all(fade);
        }
        self.playing.clear();
    }

    fn set_tile(&mut self, id: TileId, setting: TileSetting) {
        let Some(tile) = self.library.tile_mut(id) else {
            return;
        };
        match setting {
            TileSetting::Looping(looping) => tile.looping = looping,
            TileSetting::TrimSilence(trim) => tile.trim_silence = trim,
            TileSetting::Normalize(normalize) => tile.normalize = normalize,
            TileSetting::FadeIn(seconds) => {
                tile.fade_in_seconds = seconds.clamp(0.0, MAX_FADE_SECONDS);
            }
            TileSetting::FadeOut(seconds) => {
                tile.fade_out_seconds = seconds.clamp(0.0, MAX_FADE_SECONDS);
            }
            TileSetting::Reverb(amount) => tile.reverb = amount.clamp(0.0, 1.0),
        }
        self.changed();
        if let (Some(playing), Audio::Running(engine)) = (self.playing.get(&id), &mut self.audio) {
            match setting {
                TileSetting::Looping(looping) => engine.set_looping(playing.voice, looping),
                TileSetting::Reverb(amount) => engine.set_reverb(playing.voice, amount),
                _ => {}
            }
        }
    }

    // ----- menu actions ---------------------------------------------------

    /// Carries out a request that concerns only this window.
    fn act(&mut self, ctx: &egui::Context, action: MenuAction) {
        match action {
            MenuAction::AddSound => self.add_sound_dialog(),
            MenuAction::ImportFolder => self.import_folder_dialog(),
            MenuAction::ImportBoard => self.import_board_dialog(),
            MenuAction::ShowLibraryFolder => self.show_library_folder(),
            MenuAction::NewSet => self.apply_sidebar(SidebarAction::NewSet),
            MenuAction::RenameSet => {
                self.rename = self.library.set(self.current).map(Rename::new);
            }
            MenuAction::RemoveSet => self.ask_remove_set(self.current),
            MenuAction::RenameTile => {
                if self.selected.is_some() {
                    self.prefs.show_inspector = true;
                    self.prefs.show_generator = false;
                    self.focus_title = true;
                }
            }
            MenuAction::RemoveTile => {
                if let Some(id) = self.selected {
                    self.remove_tile(id);
                }
            }
            MenuAction::Undo => self.undo(),
            MenuAction::DuplicateTile => {
                if let Some(id) = self.selected {
                    self.duplicate_tile(id, None);
                }
            }
            MenuAction::MoveTile(direction) => self.nudge_selected(direction),
            MenuAction::Redo => self.redo(),
            MenuAction::ToggleSpectrograms => {
                self.prefs.show_spectrograms = !self.prefs.show_spectrograms;
            }
            MenuAction::ToggleSets => self.prefs.show_sets = !self.prefs.show_sets,
            MenuAction::ToggleInspector => {
                // From the Generator, it brings the inspector back.
                if self.prefs.show_generator {
                    self.prefs.show_generator = false;
                    self.prefs.show_inspector = true;
                } else {
                    self.prefs.show_inspector = !self.prefs.show_inspector;
                }
            }
            MenuAction::ToggleGenerator => {
                self.prefs.show_generator = !self.prefs.show_generator;
            }
            MenuAction::TileStyle(style) => self.prefs.tile_style = style,
            MenuAction::TileSize(size) => self.prefs.tile_size = size,
            MenuAction::NextTileSize => {
                let size = self.prefs.tile_size.next();
                self.prefs.tile_size = size;
                let fit = menu::shortcut_text(ctx, MenuAction::FitColumns);
                self.set_status(
                    format!("{} · {fit} fits the columns to the window", size.name()),
                    false,
                );
            }
            MenuAction::SetColumns(columns) => self.set_columns(self.current, columns),
            MenuAction::Wider => {
                let columns = self.current_set().columns.saturating_add(1);
                self.set_columns(self.current, columns);
            }
            MenuAction::FitColumns => {
                let fit = self.fit_columns();
                let set = self.current_set();
                if fit.max(set.min_columns()) == set.columns {
                    let message = format!("“{}” already fits the window", set.name);
                    self.set_status(message, false);
                } else {
                    self.set_columns(self.current, fit);
                }
            }
            MenuAction::Narrower => {
                let columns = self.current_set().columns.saturating_sub(1).max(1);
                self.set_columns(self.current, columns);
            }
            MenuAction::Theme(theme) => {
                self.prefs.theme = theme;
                ctx.set_theme(theme.preference());
            }
            MenuAction::PlaySelected => {
                if let Some(id) = self.selected {
                    self.execute(Command::ToggleTile(id));
                }
            }
            MenuAction::StopAll => self.execute(Command::StopAll),
            MenuAction::FadeAll => self.execute(Command::FadeAll),
            MenuAction::ToggleMute => {
                self.prefs.muted = !self.prefs.muted;
                self.apply_master_gain();
            }
            MenuAction::SetVolume(volume) => self.execute(Command::SetMasterVolume(volume)),
            MenuAction::ToggleClickPlays => self.prefs.click_plays = !self.prefs.click_plays,
            MenuAction::Move(direction) => self.move_selection(direction),
            MenuAction::ShowShortcuts => self.show_shortcuts = true,
            MenuAction::PolishSelected => {
                if let Some(id) = self.selected {
                    self.polish_tile(id);
                }
            }
            MenuAction::PolishSet => self.polish_set(),
            MenuAction::EditPolish => self.show_polish = true,
            MenuAction::ToggleAmbience => {
                self.execute(Command::SetAmbience(!self.prefs.ambience.enabled));
                let message = if self.prefs.ambience.enabled {
                    format!("Ambience: {}", self.prefs.ambience.describe())
                } else {
                    "Ambience off".to_owned()
                };
                self.set_status(message, false);
            }
            MenuAction::ChooseEmoji => {
                if let Some(id) = self.selected {
                    self.open_emoji_picker(id);
                }
            }
            MenuAction::OpenWebsite => ctx.open_url(egui::OpenUrl::new_tab(WEBSITE)),
        }
    }

    /// Moves the selection to the nearest tile in `direction`: along the
    /// row for left and right, to the closest tile of the nearest row above
    /// or below for up and down. With nothing selected, the first tile.
    fn move_selection(&mut self, direction: Direction) {
        let set = self.current_set();
        let Some(from) = self
            .selected
            .and_then(|id| set.tile(id))
            .map(|tile| tile.slot)
        else {
            self.selected = set
                .tiles
                .iter()
                .min_by_key(|tile| (tile.slot.row, tile.slot.column))
                .map(|tile| tile.id);
            self.reveal_selected = self.selected.is_some();
            return;
        };
        let distance = |slot: Slot| -> Option<(u8, u8)> {
            let (row, column) = (slot.row, slot.column);
            match direction {
                Direction::Left if row == from.row && column < from.column => {
                    Some((0, from.column - column))
                }
                Direction::Right if row == from.row && column > from.column => {
                    Some((0, column - from.column))
                }
                Direction::Up if row < from.row => {
                    Some((from.row - row, column.abs_diff(from.column)))
                }
                Direction::Down if row > from.row => {
                    Some((row - from.row, column.abs_diff(from.column)))
                }
                _ => None,
            }
        };
        let nearest = set
            .tiles
            .iter()
            .filter_map(|tile| distance(tile.slot).map(|distance| (distance, tile.id)))
            .min_by_key(|(distance, _)| *distance);
        if let Some((_, id)) = nearest {
            self.selected = Some(id);
            self.reveal_selected = true;
        }
    }

    /// Gives a tile the Polish preset, or takes it off if the tile has it.
    fn polish_tile(&mut self, id: TileId) {
        let Some((_, tile)) = self.library.tile(id) else {
            return;
        };
        let polish = if self.prefs.polish.applies_to(tile) {
            Polish::OFF
        } else {
            self.prefs.polish
        };
        for setting in polish.settings() {
            self.execute(Command::SetTile(id, setting));
        }
    }

    /// Gives every tile of the front set the Polish preset.
    fn polish_set(&mut self) {
        let polish = self.prefs.polish;
        let tiles: Vec<TileId> = self
            .current_set()
            .tiles
            .iter()
            .filter(|tile| !polish.applies_to(tile))
            .map(|tile| tile.id)
            .collect();
        for &id in &tiles {
            for setting in polish.settings() {
                self.execute(Command::SetTile(id, setting));
            }
        }
        let message = match tiles.len() {
            0 => "Every tile in this set is already polished".to_owned(),
            1 => "Polished 1 tile".to_owned(),
            n => format!("Polished {n} tiles"),
        };
        self.set_status(message, false);
    }

    fn ask_remove_set(&mut self, id: SetId) {
        if self.library.sets.len() <= 1 {
            return;
        }
        let Some(set) = self.library.set(id) else {
            return;
        };
        let tiles = match set.tiles.len() {
            0 => "It has no tiles.".to_owned(),
            1 => "Its tile goes with it; the sound file stays in the library.".to_owned(),
            n => format!("Its {n} tiles go with it; the sound files stay in the library."),
        };
        self.confirm = Some(Confirm {
            title: format!("Remove the set “{}”?", set.name),
            body: tiles,
            verb: "Remove",
            pending: Pending::RemoveSet(id),
        });
    }

    fn show_library_folder(&mut self) {
        let root = self.store.root().to_path_buf();
        let program = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(target_os = "windows") {
            "explorer"
        } else {
            "xdg-open"
        };
        if let Err(err) = std::process::Command::new(program).arg(&root).spawn() {
            self.set_status(format!("Could not open {}: {err}", root.display()), true);
        }
    }

    // ----- library edits --------------------------------------------------

    /// Records a change just made: it can be undone, and the status says
    /// what happened, with an Undo button.
    fn record(&mut self, change: Change) {
        if let Some(message) = change.done_message() {
            self.set_undo_status(message);
        }
        self.history.record(change);
    }

    /// Removes a tile at once; Undo puts it back.
    fn remove_tile(&mut self, id: TileId) {
        if let Some(change) = self.take_tile(id) {
            self.record(change);
        }
    }

    /// Copies a tile into `at`, or the next free slot, and selects the copy.
    fn duplicate_tile(&mut self, id: TileId, at: Option<Slot>) {
        let Some((set, tile)) = self.library.tile(id) else {
            return;
        };
        let (set, source) = (set.id, tile.title.clone());
        let copy = self
            .library
            .set_mut(set)
            .and_then(|target| target.duplicate(id, at));
        let Some((_, tile)) = copy.and_then(|copy| self.library.tile(copy)) else {
            self.set_status("There is no free slot in this set for a copy", true);
            return;
        };
        let tile = Box::new(tile.clone());
        self.selected = Some(tile.id);
        self.reveal_selected = true;
        self.changed();
        self.record(Change::Duplicated { set, tile, source });
    }

    /// Moves a tile to another slot of its set, swapping with a tile there.
    fn move_tile(&mut self, id: TileId, to: Slot) {
        if let Some(change) = self.shift_tile(id, to) {
            self.record(change);
        }
    }

    /// Moves (or copies) a tile to another set, keeping its slot there if
    /// it is free. The front set stays in front; the status says where the
    /// tile went.
    fn transfer_tile(&mut self, id: TileId, to: SetId, copy: bool) {
        let Some(name) = self.library.set(to).map(|set| set.name.clone()) else {
            return;
        };
        let change = if copy {
            self.copy_to_set(id, to, None)
        } else {
            self.move_to_set(id, to, None)
        };
        match change {
            Some(change) => self.record(change),
            None => self.set_status(format!("“{name}” has no free slot"), true),
        }
    }

    fn move_to_set(&mut self, id: TileId, to: SetId, preferred: Option<Slot>) -> Option<Change> {
        let to_name = self.library.set(to)?.name.clone();
        let (from, from_slot, _) = self.library.move_tile_to_set(id, to, preferred)?;
        if let Some(playing) = self.playing.get_mut(&id) {
            // It keeps playing; the sidebar's speaker follows it.
            playing.set = to;
        }
        if self.selected == Some(id) {
            self.selected = None;
        }
        self.changed();
        let tile = Box::new(self.library.tile(id)?.1.clone());
        Some(Change::ToSet {
            tile,
            from,
            from_slot,
            to,
            to_name,
            copy: false,
        })
    }

    fn copy_to_set(&mut self, id: TileId, to: SetId, preferred: Option<Slot>) -> Option<Change> {
        let to_name = self.library.set(to)?.name.clone();
        let (from, from_slot) = self
            .library
            .tile(id)
            .map(|(set, tile)| (set.id, tile.slot))?;
        let copy = self.library.copy_tile_to_set(id, to, preferred)?;
        self.changed();
        let tile = Box::new(self.library.tile(copy)?.1.clone());
        Some(Change::ToSet {
            tile,
            from,
            from_slot,
            to,
            to_name,
            copy: true,
        })
    }

    /// How many columns fit the grid's room as last drawn, at the current
    /// tile size; the front set's width before the grid has been drawn.
    fn fit_columns(&self) -> u8 {
        self.grid_layout
            .map_or(self.current_set().columns, |layout| {
                layout.fitting_columns()
            })
    }

    /// Makes a set's grid `columns` wide, as far as its tiles allow, and
    /// says so if they stop it.
    fn set_columns(&mut self, set: SetId, columns: u8) {
        let Some(change) = self.resize_set(set, columns) else {
            if let Some(target) = self.library.set(set)
                && columns < target.min_columns()
            {
                let message = format!(
                    "Tiles in column {} keep “{}” at least that wide; move them first",
                    target.min_columns(),
                    target.name
                );
                self.set_status(message, false);
            }
            return;
        };
        self.record(change);
    }

    /// Changes a set's width; `None` if it stays as it is.
    fn resize_set(&mut self, set: SetId, columns: u8) -> Option<Change> {
        let target = self.library.set_mut(set)?;
        let from = target.columns;
        let to = target.set_columns(columns);
        if to == from {
            return None;
        }
        let name = target.name.clone();
        self.changed();
        Some(Change::Columns {
            set,
            name,
            from,
            to,
        })
    }

    /// Moves the selected tile one slot over, swapping with a tile there.
    fn nudge_selected(&mut self, direction: Direction) {
        let Some(id) = self.selected else {
            return;
        };
        let Some((set, tile)) = self.library.tile(id) else {
            return;
        };
        let Slot { row, column } = tile.slot;
        let to = match direction {
            Direction::Up => row.checked_sub(1).map(|row| Slot::new(row, column)),
            Direction::Down => Some(Slot::new(row + 1, column)),
            Direction::Left => column.checked_sub(1).map(|column| Slot::new(row, column)),
            Direction::Right => Some(Slot::new(row, column + 1)),
        };
        if let Some(to) = to.filter(|&to| set.contains(to)) {
            self.move_tile(id, to);
            self.reveal_selected = true;
        }
    }

    fn shift_tile(&mut self, id: TileId, to: Slot) -> Option<Change> {
        let (set, tile) = self.library.tile(id)?;
        let (set, title) = (set.id, tile.title.clone());
        let from = self.library.set_mut(set)?.move_tile(id, to)?;
        self.changed();
        Some(Change::Moved {
            set,
            tile: id,
            title,
            from,
            to,
        })
    }

    /// Takes a tile off its set, for removing or redoing a removal.
    fn take_tile(&mut self, id: TileId) -> Option<Change> {
        let set = self.library.tile(id)?.0.id;
        self.stop_tile(id);
        let tile = self.library.set_mut(set)?.remove_tile(id)?;
        if self.selected == Some(id) {
            self.selected = None;
        }
        self.changed();
        Some(Change::RemovedTile {
            set,
            tile: Box::new(tile),
        })
    }

    /// Takes a set out of the library (never the last one).
    fn take_set(&mut self, id: SetId) -> Option<Change> {
        if self.library.sets.len() <= 1 {
            return None;
        }
        let index = self.library.sets.iter().position(|set| set.id == id)?;
        let tiles: Vec<TileId> = self.library.sets[index]
            .tiles
            .iter()
            .map(|tile| tile.id)
            .collect();
        for tile in tiles {
            self.stop_tile(tile);
        }
        let set = self.library.remove_set(id)?;
        if self.current == id {
            self.current = self.library.sets[index.min(self.library.sets.len() - 1)].id;
            self.selected = None;
        }
        self.changed();
        Some(Change::RemovedSet { index, set })
    }

    /// Puts a whole tile into its set (or the front set, if that is gone):
    /// in its own slot if free, else the first free one. Selects it.
    fn insert_tile(
        &mut self,
        set: SetId,
        mut tile: Box<Tile>,
    ) -> Result<(SetId, Box<Tile>), Box<Tile>> {
        let home = if self.library.set(set).is_some() {
            set
        } else {
            self.current
        };
        let slot = self
            .library
            .set_mut(home)
            .and_then(|target| target.put_back(&tile));
        let Some(slot) = slot else {
            self.set_status("There is no free slot to put the tile back in", true);
            return Err(tile);
        };
        tile.slot = slot;
        self.execute(Command::SelectSet(home));
        self.selected = Some(tile.id);
        self.reveal_selected = true;
        self.changed();
        Ok((home, tile))
    }

    /// Makes a change again (redo). Returns the record for undoing it.
    fn apply(&mut self, change: Change) -> Result<Change, Change> {
        match change {
            Change::Columns { set, to, .. } => {
                let done = self.resize_set(set, to);
                self.execute(Command::SelectSet(set));
                done.ok_or(change)
            }
            Change::ToSet {
                mut tile,
                from,
                from_slot,
                to,
                to_name,
                copy: true,
            } => {
                // The copy goes back where it was, if that slot is free.
                let slot = self.library.set_mut(to).and_then(|set| set.put_back(&tile));
                if let Some(slot) = slot {
                    tile.slot = slot;
                    self.changed();
                }
                let change = Change::ToSet {
                    tile,
                    from,
                    from_slot,
                    to,
                    to_name,
                    copy: true,
                };
                if slot.is_some() {
                    Ok(change)
                } else {
                    Err(change)
                }
            }
            Change::ToSet {
                tile,
                from,
                from_slot,
                to,
                to_name,
                copy: false,
            } => {
                let (id, slot) = (tile.id, tile.slot);
                self.move_to_set(id, to, Some(slot)).ok_or(Change::ToSet {
                    tile,
                    from,
                    from_slot,
                    to,
                    to_name,
                    copy: false,
                })
            }
            Change::RemovedTile { set, tile } => {
                let id = tile.id;
                self.take_tile(id).ok_or(Change::RemovedTile { set, tile })
            }
            Change::RemovedSet { index, set } => {
                let id = set.id;
                self.take_set(id).ok_or(Change::RemovedSet { index, set })
            }
            Change::Duplicated { set, tile, source } => match self.insert_tile(set, tile) {
                Ok((set, tile)) => Ok(Change::Duplicated { set, tile, source }),
                Err(tile) => Err(Change::Duplicated { set, tile, source }),
            },
            Change::Generated { set, tile } => match self.insert_tile(set, tile) {
                Ok((set, tile)) => Ok(Change::Generated { set, tile }),
                Err(tile) => Err(Change::Generated { set, tile }),
            },
            Change::Moved {
                set,
                tile,
                title,
                from,
                to,
            } => {
                let moved = self.shift_tile(tile, to);
                self.show_moved(set, tile);
                moved.ok_or(Change::Moved {
                    set,
                    tile,
                    title,
                    from,
                    to,
                })
            }
        }
    }

    /// Undoes a change. Returns the record for redoing it.
    fn revert(&mut self, change: Change) -> Result<Change, Change> {
        match change {
            Change::Columns {
                set,
                ref name,
                from,
                to,
            } => {
                let width = self
                    .library
                    .set_mut(set)
                    .map(|target| target.set_columns(from));
                self.execute(Command::SelectSet(set));
                if width == Some(from) {
                    self.changed();
                    Ok(Change::Columns {
                        set,
                        name: name.clone(),
                        from,
                        to,
                    })
                } else {
                    // Tiles placed since keep it wider: leave it as it is.
                    if let Some(width) = width {
                        self.library
                            .set_mut(set)
                            .map(|target| target.set_columns(width));
                    }
                    Err(change)
                }
            }
            Change::ToSet {
                mut tile,
                from,
                from_slot,
                to,
                to_name,
                copy,
            } => {
                let id = tile.id;
                let undone = if copy {
                    self.stop_tile(id);
                    let removed = self.library.set_mut(to).and_then(|set| set.remove_tile(id));
                    removed.map(|_| ())
                } else {
                    // Back to its old slot if free; remember where it was, for
                    // redo.
                    let back = self.library.move_tile_to_set(id, from, Some(from_slot));
                    back.map(|(_, slot_there, _)| {
                        tile.slot = slot_there;
                        if let Some(playing) = self.playing.get_mut(&id) {
                            playing.set = from;
                        }
                        self.show_moved(from, id);
                    })
                };
                if undone.is_some() {
                    self.changed();
                }
                let change = Change::ToSet {
                    tile,
                    from,
                    from_slot,
                    to,
                    to_name,
                    copy,
                };
                if undone.is_some() {
                    Ok(change)
                } else {
                    Err(change)
                }
            }
            Change::RemovedTile { set, tile } => match self.insert_tile(set, tile) {
                Ok((set, tile)) => Ok(Change::RemovedTile { set, tile }),
                Err(tile) => Err(Change::RemovedTile { set, tile }),
            },
            Change::RemovedSet { index, set } => {
                let id = set.id;
                let index = index.min(self.library.sets.len());
                let record = set.clone();
                self.library.sets.insert(index, set);
                self.execute(Command::SelectSet(id));
                self.changed();
                Ok(Change::RemovedSet { index, set: record })
            }
            Change::Duplicated { set, tile, source } => match self.take_tile(tile.id) {
                Some(Change::RemovedTile { set, tile }) => {
                    Ok(Change::Duplicated { set, tile, source })
                }
                _ => Err(Change::Duplicated { set, tile, source }),
            },
            Change::Generated { set, tile } => match self.take_tile(tile.id) {
                Some(Change::RemovedTile { set, tile }) => Ok(Change::Generated { set, tile }),
                _ => Err(Change::Generated { set, tile }),
            },
            Change::Moved {
                set,
                tile,
                title,
                from,
                to,
            } => {
                let moved = self.shift_tile(tile, from);
                self.show_moved(set, tile);
                let change = Change::Moved {
                    set,
                    tile,
                    title,
                    from,
                    to,
                };
                if moved.is_some() {
                    Ok(change)
                } else {
                    Err(change)
                }
            }
        }
    }

    /// Brings a moved tile into view, selected.
    fn show_moved(&mut self, set: SetId, tile: TileId) {
        if self.library.tile(tile).is_some() {
            self.execute(Command::SelectSet(set));
            self.selected = Some(tile);
            self.reveal_selected = true;
        }
    }

    fn undo(&mut self) {
        let Some(change) = self.history.take_undo() else {
            return;
        };
        match self.revert(change) {
            Ok(change) => {
                self.set_status(change.undone_message(), false);
                self.history.push_redo(change);
            }
            // No free slot yet: it can be undone once there is one.
            Err(change @ Change::RemovedTile { .. }) => self.history.push_undo(change),
            Err(change) => self.set_status(
                format!("“{}” can no longer be undone", change.describe()),
                true,
            ),
        }
    }

    fn redo(&mut self) {
        let Some(change) = self.history.take_redo() else {
            return;
        };
        match self.apply(change) {
            Ok(change) => {
                if let Some(message) = change.done_message() {
                    self.set_undo_status(message);
                }
                self.history.push_undo(change);
            }
            Err(change @ (Change::Duplicated { .. } | Change::Generated { .. })) => {
                self.history.push_redo(change);
            }
            Err(change) => self.set_status(
                format!("“{}” can no longer be redone", change.describe()),
                true,
            ),
        }
    }

    fn open_emoji_picker(&mut self, id: TileId) {
        if let Some((_, tile)) = self.library.tile(id) {
            self.emoji_picker = Some(emoji::Picker::new(id, &tile.title, &tile.emoji));
        }
    }

    fn set_emoji(&mut self, id: TileId, emoji: Option<&str>) {
        let emoji = emoji.unwrap_or_default();
        if let Some(tile) = self.library.tile_mut(id)
            && tile.emoji != emoji
        {
            tile.emoji = emoji.to_owned();
            self.changed();
        }
    }

    /// Feeds the meters with the levels the mixer has seen since the last
    /// frame, and keeps redrawing while any of them has something to show.
    fn update_meters(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        self.meter_time = now;
        let (outputs, voices): ([f32; 2], Vec<(TileId, f32)>) = match &self.audio {
            Audio::Running(engine) => (
                engine.take_output_peaks(),
                self.playing
                    .iter()
                    .map(|(&id, playing)| {
                        (id, engine.take_voice_peak(playing.voice).unwrap_or(0.0))
                    })
                    .collect(),
            ),
            Audio::Unavailable { .. } => ([0.0; 2], Vec::new()),
        };
        for (meter, peak) in self.output_meters.iter_mut().zip(outputs) {
            meter.feed(peak, now);
        }
        for meter in self.tile_meters.values_mut() {
            meter.feed(0.0, now);
        }
        for (id, peak) in voices {
            self.tile_meters.entry(id).or_default().feed(peak, now);
        }
        let playing = &self.playing;
        self.tile_meters
            .retain(|id, meter| playing.contains_key(id) || !meter.is_resting(now));
        let moving = !self.tile_meters.is_empty()
            || self
                .output_meters
                .iter()
                .any(|meter| !meter.is_resting(now));
        if moving {
            ctx.request_repaint_after(PLAYING_REPAINT);
        }
    }

    fn rename_tile(&mut self, id: TileId, title: String) {
        if let Some(tile) = self.library.tile_mut(id)
            && tile.title != title
        {
            tile.title = title;
            self.changed();
        }
    }

    fn recolor_tile(&mut self, id: TileId, color: TileColor) {
        if let Some(tile) = self.library.tile_mut(id) {
            tile.color = color;
            self.changed();
        }
    }

    /// Removes a set (after the confirmation); Undo puts it back.
    fn remove_set(&mut self, id: SetId) {
        if let Some(change) = self.take_set(id) {
            self.record(change);
        }
    }

    fn apply_sidebar(&mut self, action: SidebarAction) {
        match action {
            SidebarAction::Select(id) => self.execute(Command::SelectSet(id)),
            SidebarAction::StartRename(id) => {
                self.rename = self.library.set(id).map(Rename::new);
            }
            SidebarAction::Rename(id, text) => {
                let name = text.trim();
                if let Some(set) = self.library.set_mut(id)
                    && !name.is_empty()
                    && set.name != name
                {
                    set.name = name.to_owned();
                    self.changed();
                }
                self.rename = None;
            }
            SidebarAction::CancelRename => self.rename = None,
            SidebarAction::NewSet => {
                let id = self.library.add_set();
                self.execute(Command::SelectSet(id));
                self.rename = self.library.set(id).map(Rename::new);
                self.changed();
            }
            SidebarAction::RemoveSet(id) => self.ask_remove_set(id),
            SidebarAction::DropTile { tile, set, copy } => {
                self.spring_origin = None;
                self.transfer_tile(tile, set, copy);
            }
            SidebarAction::SetColumns(set, columns) => self.set_columns(set, columns),
            SidebarAction::SpringOpen(set) => {
                self.spring_origin.get_or_insert(self.current);
                self.execute(Command::SelectSet(set));
            }
            SidebarAction::Polish(id) => {
                self.execute(Command::SelectSet(id));
                self.polish_set();
            }
        }
    }

    fn resolve(&mut self, pending: Pending) {
        match pending {
            Pending::RemoveSet(id) => self.remove_set(id),
        }
    }

    // ----- generating -----------------------------------------------------

    /// Starts rendering a track of the Generator's settings, for the front
    /// set. One at a time.
    fn start_generation(&mut self, ctx: &egui::Context) {
        if self.generation.is_some() {
            return;
        }
        let generation = Generation::start(
            ctx.clone(),
            self.store.clone(),
            self.prefs.ambience,
            self.prefs.track_seconds,
            self.current,
        );
        self.set_status(format!("Generating “{}”…", generation.title), false);
        self.generation = Some(generation);
    }

    /// True while a track is being generated.
    pub fn is_generating(&self) -> bool {
        self.generation.is_some()
    }

    /// Places a finished track on a tile, or says why not.
    fn poll_generation(&mut self) {
        let Some(result) = self.generation.as_ref().and_then(Generation::poll) else {
            return;
        };
        let Some(generation) = self.generation.take() else {
            return;
        };
        match result {
            Ok(Some(sound)) => self.place_track(&generation, sound),
            Ok(None) => {
                self.set_status(format!("Stopped generating “{}”", generation.title), false)
            }
            Err(err) => self.set_status(
                format!("Could not generate “{}”: {err}", generation.title),
                true,
            ),
        }
    }

    fn place_track(&mut self, generation: &Generation, sound: Sound) {
        let set = if self.library.set(generation.set).is_some() {
            generation.set
        } else {
            self.current
        };
        let placed = self
            .library
            .set_mut(set)
            .and_then(|target| target.place(sound, None));
        let Some(id) = placed else {
            self.set_status(
                format!(
                    "There is no free slot in this set for “{}”",
                    generation.title
                ),
                true,
            );
            return;
        };
        if let Some(tile) = self.library.tile_mut(id) {
            tile.emoji = generation.emoji.to_owned();
        }
        let Some((_, tile)) = self.library.tile(id) else {
            return;
        };
        let tile = Box::new(tile.clone());
        self.execute(Command::SelectSet(set));
        self.selected = Some(id);
        self.reveal_selected = true;
        self.changed();
        self.record(Change::Generated { set, tile });
    }

    // ----- importing ------------------------------------------------------

    /// Imports files, folders and boards; loose files go into the front set,
    /// the first at `slot` if it is free.
    fn import(&mut self, paths: &[PathBuf], slot: Option<Slot>) {
        let report: Report =
            Importer::new(&self.store, &mut self.library).paths(paths, self.current, slot);
        // Replaced sets may have lost tiles that were playing.
        let gone: Vec<TileId> = self
            .playing
            .keys()
            .filter(|&&id| self.library.tile(id).is_none())
            .copied()
            .collect();
        for id in gone {
            self.stop_tile(id);
        }
        if let Some(&first) = report.sets.first() {
            self.execute(Command::SelectSet(first));
        }
        if report.tiles > 0 || !report.sets.is_empty() {
            self.changed();
        }
        self.set_status(report.summary(), !report.problems.is_empty());
    }

    fn pick_and_import(&mut self, picked: Option<Vec<PathBuf>>) {
        if let Some(paths) = picked.filter(|paths| !paths.is_empty()) {
            self.import(&paths, None);
        }
    }

    fn add_sound_dialog(&mut self) {
        let mut extensions: Vec<&str> = olio_audio::AUDIO_EXTENSIONS.to_vec();
        extensions.extend(olio_audio::VIDEO_EXTENSIONS);
        let picked = rfd::FileDialog::new()
            .set_title("Add Sound")
            .add_filter("Audio and video", &extensions)
            .pick_files();
        self.pick_and_import(picked);
    }

    fn import_folder_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Import Folder as Set")
            .pick_folders();
        self.pick_and_import(picked);
    }

    fn import_board_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Import Board")
            .add_filter("Board (JSON)", &["json"])
            .pick_file()
            .map(|path| vec![path]);
        self.pick_and_import(picked);
    }

    fn import_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_owned())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        let slot = ctx
            .pointer_latest_pos()
            .zip(self.grid_layout)
            .and_then(|(pos, layout)| layout.slot_at(pos));
        self.import(&dropped, slot);
    }

    // ----- each frame -----------------------------------------------------

    fn maintain_audio(&mut self, ctx: &egui::Context) {
        if let Some(silent) = &mut self.silent {
            silent.render(ctx.input(|input| input.time));
        }
        let mut failed = false;
        match &mut self.audio {
            Audio::Running(engine) => {
                for voice in engine.poll() {
                    self.playing.retain(|_, playing| playing.voice != voice);
                }
                failed = engine.output_failed();
            }
            Audio::Unavailable { retry_at, .. } => {
                let now = Instant::now();
                if now >= *retry_at {
                    self.audio = open_audio();
                    if let Audio::Running(engine) = &self.audio
                        && let Some(output) = engine.output()
                    {
                        let message = format!("Playing through {}", output.device);
                        self.set_status(message, false);
                        self.apply_master_gain();
                        self.apply_ambience();
                    }
                } else {
                    ctx.request_repaint_after(*retry_at - now);
                }
            }
        }
        if failed {
            self.playing.clear();
            self.audio = Audio::Unavailable {
                reason: "the output device stopped".to_owned(),
                retry_at: Instant::now(),
            };
            self.set_status("The audio output stopped; reconnecting…", true);
        }
    }

    fn request_clips(&mut self) {
        let set = self
            .library
            .set(self.current)
            .unwrap_or(&self.library.sets[0]);
        for tile in &set.tiles {
            self.clips
                .request(&tile.sound.hash, || self.store.sound_path(&tile.sound));
        }
    }

    fn progress(&self) -> HashMap<TileId, Progress> {
        let Audio::Running(engine) = &self.audio else {
            return HashMap::new();
        };
        self.playing
            .iter()
            .map(|(&id, playing)| {
                let position = engine.position(playing.voice).unwrap_or(playing.start);
                let into = position.saturating_sub(playing.start).min(playing.length);
                let rate = f64::from(playing.rate);
                let progress = Progress {
                    elapsed_seconds: into as f64 / rate,
                    total_seconds: playing.length as f64 / rate,
                    fraction: if playing.length == 0 {
                        0.0
                    } else {
                        into as f32 / playing.length as f32
                    },
                };
                (id, progress)
            })
            .collect()
    }

    /// Tile keys and set shortcuts. Ignored while a text field has focus;
    /// a button focused with Tab does not block them.
    fn keyboard_commands(&self, ctx: &egui::Context) -> Vec<Command> {
        if ctx.text_edit_focused() {
            return Vec::new();
        }
        let set = self.current_set();
        let events = ctx.input(|input| input.events.clone());
        let mut commands = Vec::new();
        for event in events {
            let Event::Key {
                key,
                physical_key,
                pressed: true,
                repeat: false,
                modifiers,
            } = event
            else {
                continue;
            };
            // The grid follows key positions, whatever the layout.
            let key = physical_key.unwrap_or(key);
            if modifiers.command && !modifiers.alt && !modifiers.shift {
                if let Some(set) = keys::set_number(key).and_then(|n| self.library.sets.get(n - 1))
                {
                    commands.push(Command::SelectSet(set.id));
                }
                continue;
            }
            if modifiers.command || modifiers.ctrl || modifiers.shift {
                continue;
            }
            let tile = keys::tile_char(key)
                .and_then(|key| {
                    TileKey {
                        key,
                        alt: modifiers.alt,
                    }
                    .slot()
                })
                .and_then(|slot| set.tile_at(slot));
            if let Some(tile) = tile {
                commands.push(Command::ToggleTile(tile.id));
            }
        }
        commands
    }

    /// Holding Escape stops everything. A tap clears the selection, and
    /// explains the hold if something is playing.
    fn escape_hold(&mut self, ctx: &egui::Context, commands: &mut Vec<Command>) {
        let typing = ctx.text_edit_focused();
        let pressed = !typing
            && ctx.input(|input| {
                input.events.iter().any(|event| {
                    matches!(
                        event,
                        Event::Key {
                            key: Key::Escape,
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } if modifiers.is_none()
                    )
                })
            });
        let down = ctx.input(|input| input.key_down(Key::Escape));
        if pressed {
            self.escape_down_since = Some(Instant::now());
            self.escape_fired = false;
        }
        let Some(since) = self.escape_down_since else {
            return;
        };
        if down {
            let held = since.elapsed();
            if held >= STOP_ALL_HOLD {
                if !self.escape_fired {
                    self.escape_fired = true;
                    commands.push(Command::StopAll);
                }
            } else {
                ctx.request_repaint_after(STOP_ALL_HOLD - held);
            }
        } else {
            if !self.escape_fired {
                if self.playing.is_empty() {
                    self.selected = None;
                } else {
                    self.set_status("Hold Esc to stop all sounds", false);
                }
            }
            self.escape_down_since = None;
        }
    }

    fn autosave(&mut self, ctx: &egui::Context) {
        let Some(since) = self.unsaved_since else {
            return;
        };
        if since.elapsed() < SAVE_DELAY {
            ctx.request_repaint_after(SAVE_DELAY - since.elapsed());
            return;
        }
        self.save_now();
    }

    fn save_now(&mut self) {
        if self.unsaved_since.take().is_none() || self.load_error.is_some() {
            return;
        }
        if let Err(err) = self.store.save(&self.library) {
            self.set_status(format!("Could not save the library: {err}"), true);
        }
    }

    fn drive_screenshot(&mut self, ctx: &egui::Context) {
        let loading = self.is_loading();
        let Some(job) = self.screenshot.as_mut() else {
            return;
        };
        ctx.request_repaint();
        job.frames += 1;

        if job.frames >= 10 && !loading && !job.requested {
            job.requested = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::default()));
            return;
        }
        let shot = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(shot) = shot {
            let [width, height] = shot.size;
            let written = image::save_buffer(
                &job.path,
                shot.as_raw(),
                width as u32,
                height as u32,
                image::ExtendedColorType::Rgba8,
            );
            match written {
                Ok(()) => println!("screenshot written to {}", job.path.display()),
                Err(err) => eprintln!("could not write screenshot: {err}"),
            }
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    // ----- panels ---------------------------------------------------------

    /// Master volume, the playing bubbles, Fade All and Stop All.
    fn toolbar(&self, ui: &mut Ui, commands: &mut Vec<Command>, actions: &mut Vec<MenuAction>) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let muted = self.prefs.muted;
            let mute = ui
                .selectable_label(muted, if muted { "🔇" } else { "🔊" })
                .on_hover_text(format!(
                    "{} ({})",
                    if muted { "Unmute" } else { "Mute" },
                    menu::shortcut_text(ui.ctx(), MenuAction::ToggleMute)
                ));
            mute.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, muted, "Mute"));
            if mute.clicked() {
                actions.push(MenuAction::ToggleMute);
            }
            let mut percent = (self.prefs.volume * 100.0).round();
            ui.spacing_mut().slider_width = 120.0;
            let slider = egui::Slider::new(&mut percent, 0.0..=100.0)
                .show_value(false)
                .trailing_fill(true);
            let slider = ui
                .add_enabled(!self.prefs.muted, slider)
                .on_hover_text(format!("Master volume: {percent:.0}%"));
            if slider.changed() {
                actions.push(MenuAction::SetVolume(percent / 100.0));
            }
            ui.add_space(4.0);
            meter::stereo(
                ui,
                &self.output_meters,
                self.meter_time,
                egui::vec2(110.0, 14.0),
            );
            ui.separator();

            let dark = ui.visuals().dark_mode;
            let mut playing: Vec<(&str, TileColor, TileId, &str)> = self
                .playing
                .keys()
                .filter_map(|&id| self.library.tile(id))
                .map(|(_, tile)| {
                    (
                        tile.title.as_str(),
                        tile.color,
                        tile.id,
                        tile.emoji.as_str(),
                    )
                })
                .collect();
            playing.sort_by(|a, b| a.0.cmp(b.0));
            let ambience = &self.prefs.ambience;
            if playing.is_empty() && !ambience.enabled {
                ui.weak("Nothing playing");
            }
            if ambience.enabled {
                // The ambience's own bubble: its background and style; a
                // click switches it off.
                let emoji = generator::emoji(ambience.background);
                let color = ui.visuals().weak_text_color();
                let text = RichText::new(format!("■ {}", ambience.describe())).color(color);
                let bubble = match self.images.texture(ui.ctx(), emoji) {
                    Some(texture) => {
                        let picture = egui::Image::from_texture(egui::load::SizedTexture::new(
                            texture,
                            egui::vec2(16.0, 16.0),
                        ));
                        let stop = RichText::new("■").color(color);
                        let name = RichText::new(ambience.describe()).color(color);
                        egui::Button::new((stop, picture, name))
                    }
                    None => egui::Button::new(text),
                }
                .fill(ui.visuals().faint_bg_color);
                if ui
                    .add(bubble)
                    .on_hover_text(format!(
                        "Ambience: {}, {}. Click to switch it off ({}).",
                        ambience.background.name().to_lowercase(),
                        ambience.scale.name(),
                        menu::shortcut_text(ui.ctx(), MenuAction::ToggleAmbience)
                    ))
                    .clicked()
                {
                    actions.push(MenuAction::ToggleAmbience);
                }
            }
            for (title, color, id, emoji) in playing {
                let text = RichText::new(format!("■ {title}")).color(palette::strong(color, dark));
                let picture = (!emoji.is_empty())
                    .then(|| self.images.texture(ui.ctx(), emoji))
                    .flatten()
                    .map(|texture| {
                        egui::Image::from_texture(egui::load::SizedTexture::new(
                            texture,
                            egui::vec2(16.0, 16.0),
                        ))
                    });
                let bubble = match picture {
                    Some(picture) => {
                        let stop = RichText::new("■").color(palette::strong(color, dark));
                        let title = RichText::new(title).color(palette::strong(color, dark));
                        egui::Button::new((stop, picture, title))
                    }
                    None => egui::Button::new(text),
                }
                .fill(palette::tint(color, dark, 40));
                if ui.add(bubble).on_hover_text("Stop").clicked() {
                    commands.push(Command::StopTile(id));
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let any = !self.playing.is_empty();
                ui.add_space(4.0);
                if ui
                    .add_enabled(any, egui::Button::new("⏹ Stop All"))
                    .on_hover_text(format!(
                        "Stop every sound ({}, or hold Esc)",
                        menu::shortcut_text(ui.ctx(), MenuAction::StopAll)
                    ))
                    .clicked()
                {
                    commands.push(Command::StopAll);
                }
                if ui
                    .add_enabled(any, egui::Button::new("Fade All"))
                    .on_hover_text(format!(
                        "Fade every sound out over {} s ({})",
                        FADE_ALL.as_secs(),
                        menu::shortcut_text(ui.ctx(), MenuAction::FadeAll)
                    ))
                    .clicked()
                {
                    commands.push(Command::FadeAll);
                }
            });
        });
    }

    /// Panel toggles, Add Sound and Import, the status line, and the output
    /// device.
    fn bottom_bar(&self, ui: &mut Ui, actions: &mut Vec<MenuAction>) {
        ui.horizontal(|ui| {
            let ctx = ui.ctx().clone();
            if ui
                .selectable_label(self.prefs.show_sets, "Sets")
                .on_hover_text(format!(
                    "Show or hide the sets ({})",
                    menu::shortcut_text(&ctx, MenuAction::ToggleSets)
                ))
                .clicked()
            {
                actions.push(MenuAction::ToggleSets);
            }
            ui.separator();
            if ui
                .button("+ Add Sound…")
                .on_hover_text(format!(
                    "Add audio or video files to this set ({})",
                    menu::shortcut_text(&ctx, MenuAction::AddSound)
                ))
                .clicked()
            {
                actions.push(MenuAction::AddSound);
            }
            ui.menu_button("Import", |ui| {
                if ui.button("Folder as Set…").clicked() {
                    actions.push(MenuAction::ImportFolder);
                    ui.close();
                }
                if ui.button("Board (JSON)…").clicked() {
                    actions.push(MenuAction::ImportBoard);
                    ui.close();
                }
            });
            ui.separator();

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .selectable_label(
                        self.prefs.show_inspector && !self.prefs.show_generator,
                        "Inspector",
                    )
                    .on_hover_text(format!(
                        "Show or hide the inspector ({})",
                        menu::shortcut_text(&ctx, MenuAction::ToggleInspector)
                    ))
                    .clicked()
                {
                    actions.push(MenuAction::ToggleInspector);
                }
                if ui
                    .selectable_label(self.prefs.show_generator, "Generator")
                    .on_hover_text(format!(
                        "Make music and backgrounds, live or as tracks ({})",
                        menu::shortcut_text(&ctx, MenuAction::ToggleGenerator)
                    ))
                    .clicked()
                {
                    actions.push(MenuAction::ToggleGenerator);
                }
                ui.separator();
                match &self.audio {
                    Audio::Running(engine) => {
                        if let Some(output) = engine.output() {
                            ui.weak(format!(
                                "{} · {:.1} kHz",
                                output.device,
                                output.rate as f32 / 1000.0
                            ))
                            .on_hover_text("The output device");
                        }
                    }
                    Audio::Unavailable { reason, .. } => {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            format!("No audio output: {reason}"),
                        );
                    }
                }
                ui.separator();
                // The status takes what is left, truncated.
                let status = if let Some(error) = &self.load_error {
                    Some((
                        format!("Library not loaded, changes are not saved: {error}"),
                        true,
                    ))
                } else {
                    self.status
                        .as_ref()
                        .filter(|status| status.remaining().is_some())
                        .map(|status| (status.text.clone(), status.error))
                };
                let undoable = self
                    .status
                    .as_ref()
                    .is_some_and(|status| status.undo && status.remaining().is_some())
                    && self.history.next_undo().is_some();
                if undoable
                    && ui
                        .small_button("Undo")
                        .on_hover_text(format!(
                            "Put it back ({})",
                            menu::shortcut_text(&ctx, MenuAction::Undo)
                        ))
                        .clicked()
                {
                    actions.push(MenuAction::Undo);
                }
                if let Some((text, error)) = status {
                    let mut text = RichText::new(text);
                    if error {
                        text = text.color(ui.visuals().error_fg_color);
                    }
                    ui.add(egui::Label::new(text).truncate());
                }
            });
        });
    }
}

impl eframe::App for OlioApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drive_screenshot(&ctx);
        self.clips.poll();
        self.poll_generation();
        self.maintain_audio(&ctx);
        self.update_meters(&ctx);
        self.request_clips();
        if self
            .selected
            .is_some_and(|id| self.current_set().tile(id).is_none())
        {
            self.selected = None;
        }

        // A drag that opened another set and ended without a drop goes
        // back to the set it started in.
        if !egui::DragAndDrop::has_any_payload(&ctx)
            && let Some(origin) = self.spring_origin.take()
        {
            self.execute(Command::SelectSet(origin));
        }

        // A confirmation takes the keyboard while it is up.
        if let Some(confirm) = self.confirm.take() {
            match confirm::show(&ctx, &confirm) {
                Reply::Waiting => self.confirm = Some(confirm),
                Reply::Cancelled => {}
                Reply::Confirmed => self.resolve(confirm.pending),
            }
        }
        // So does the emoji picker.
        if let Some(mut picker) = self.emoji_picker.take() {
            match emoji::picker(&ctx, &mut picker, &self.images) {
                emoji::Reply::Waiting => self.emoji_picker = Some(picker),
                emoji::Reply::Cancelled => {}
                emoji::Reply::Chose(choice) => self.set_emoji(picker.tile, choice),
            }
        }
        let mut commands = Vec::new();
        let mut actions = Vec::new();
        if self.confirm.is_none() && self.emoji_picker.is_none() {
            actions = menu::pressed(&ctx);
            commands = self.keyboard_commands(&ctx);
            self.escape_hold(&ctx, &mut commands);
        }
        self.import_dropped_files(&ctx);
        let files_hovering = ctx.input(|input| !input.raw.hovered_files.is_empty());

        egui::Panel::top("olio-top").show(ui, |ui| {
            let state = MenuState {
                prefs: &self.prefs,
                tile_selected: self.selected.is_some(),
                tile_polished: self
                    .selected
                    .and_then(|id| self.library.tile(id))
                    .is_some_and(|(_, tile)| self.prefs.polish.applies_to(tile)),
                set_has_tiles: !self.current_set().tiles.is_empty(),
                undo: self.history.next_undo().map(Change::describe),
                redo: self.history.next_redo().map(Change::describe),
                set_removable: self.library.sets.len() > 1,
                columns: self.current_set().columns,
                min_columns: self.current_set().min_columns(),
                fit_columns: self.fit_columns(),
                playing: !self.playing.is_empty(),
            };
            actions.extend(menu::bar(ui, &state));
            ui.separator();
            ui.add_space(2.0);
            self.toolbar(ui, &mut commands, &mut actions);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("olio-bottom").show(ui, |ui| {
            ui.add_space(4.0);
            self.bottom_bar(ui, &mut actions);
            ui.add_space(4.0);
        });

        let sounding: HashSet<SetId> = self.playing.values().map(|playing| playing.set).collect();
        let fit = self.fit_columns();
        let sidebar_actions = if self.prefs.show_sets {
            egui::Panel::left("olio-sets")
                .resizable(true)
                .default_size(210.0)
                .min_size(150.0)
                .show(ui, |ui| {
                    sidebar::show(
                        ui,
                        &self.library,
                        self.current,
                        &sounding,
                        &mut self.rename,
                        fit,
                    )
                })
                .inner
        } else {
            Vec::new()
        };

        let mut generator_actions = Vec::new();
        let inspector_actions = if self.prefs.show_generator {
            let set = self
                .library
                .set(self.current)
                .unwrap_or(&self.library.sets[0]);
            let view = generator::View {
                images: &self.images,
                busy: self
                    .generation
                    .as_ref()
                    .map(|generation| (generation.title.as_str(), generation.progress())),
                track_seconds: self.prefs.track_seconds,
                set_name: &set.name,
            };
            generator_actions = egui::Panel::right("olio-generator")
                .resizable(true)
                .default_size(400.0)
                .min_size(340.0)
                .show(ui, |ui| {
                    generator::show(ui, &mut self.prefs.ambience, &view)
                })
                .inner;
            Vec::new()
        } else if self.prefs.show_inspector {
            // Fields, not `current_set()`, so the draft can be borrowed too.
            let set = self
                .library
                .set(self.current)
                .unwrap_or(&self.library.sets[0]);
            let tile = self.selected.and_then(|id| set.tile(id));
            let state = tile.and_then(|tile| self.clips.get(&tile.sound.hash));
            let playing = tile.is_some_and(|tile| self.playing.contains_key(&tile.id));
            let focus_title = std::mem::take(&mut self.focus_title);
            egui::Panel::right("olio-inspector")
                .resizable(true)
                .default_size(300.0)
                .min_size(240.0)
                .show(ui, |ui| {
                    let view = inspector::View {
                        set,
                        tile,
                        state,
                        playing,
                        focus_title,
                        polish: self.prefs.polish,
                        images: &self.images,
                        spectrograms: self.prefs.show_spectrograms,
                        fit_columns: fit,
                        level: tile.and_then(|tile| self.tile_meters.get(&tile.id).copied()),
                        now: self.meter_time,
                    };
                    inspector::show(ui, &view, &mut self.title_draft)
                })
                .inner
        } else {
            Vec::new()
        };

        let progress = self.progress();
        let reveal_selected = std::mem::take(&mut self.reveal_selected);
        let mut canvas = egui::Frame::central_panel(ui.style());
        if self.prefs.tile_style == TileStyle::Soft {
            canvas = canvas.fill(palette::soft(ui.visuals().dark_mode).canvas);
        }
        let dragged = egui::DragAndDrop::payload::<TileId>(&ctx)
            .and_then(|id| self.library.tile(*id))
            .map(|(set, tile)| (tile, set.id));
        let grid = egui::CentralPanel::default()
            .frame(canvas)
            .show(ui, |ui| {
                let view = GridView {
                    set: self.current_set(),
                    clips: &self.clips,
                    playing: &progress,
                    selected: self.selected,
                    reveal_selected,
                    click_plays: self.prefs.click_plays,
                    polish: self.prefs.polish,
                    style: self.prefs.tile_style,
                    size: self.prefs.tile_size,
                    images: &self.images,
                    spectrograms: self.prefs.show_spectrograms,
                    levels: &self.tile_meters,
                    now: self.meter_time,
                    files_hovering,
                    dragged,
                };
                grid::show(ui, &view)
            })
            .inner;
        self.grid_layout = grid.layout;
        if let Some(selection) = grid.select {
            self.selected = selection;
        }
        commands.extend(grid.commands);

        if self.show_shortcuts {
            menu::shortcuts_window(&ctx, &mut self.show_shortcuts);
        }
        if self.show_polish {
            polish::window(&ctx, &mut self.show_polish, &mut self.prefs.polish);
        }
        for action in generator_actions {
            match action {
                generator::Action::Changed => self.apply_ambience(),
                generator::Action::Length(seconds) => self.prefs.track_seconds = seconds,
                generator::Action::Generate => self.start_generation(&ctx),
                generator::Action::Cancel => {
                    if let Some(generation) = &self.generation {
                        generation.cancel();
                    }
                }
            }
        }

        let acted = !commands.is_empty() || !actions.is_empty();
        for command in commands {
            self.execute(command);
        }
        for id in grid.remove {
            self.remove_tile(id);
        }
        for (id, color) in grid.recolor {
            self.recolor_tile(id, color);
        }
        for id in grid.polish {
            self.polish_tile(id);
        }
        for drop in grid.drops {
            self.spring_origin = None;
            let home = self.library.tile(drop.tile).map(|(set, _)| set.id);
            if home == Some(drop.set) {
                if drop.copy {
                    self.duplicate_tile(drop.tile, Some(drop.slot));
                } else {
                    self.move_tile(drop.tile, drop.slot);
                    self.selected = Some(drop.tile);
                }
                continue;
            }
            // From another set, opened by holding the tile over it.
            let change = if drop.copy {
                self.copy_to_set(drop.tile, drop.set, Some(drop.slot))
            } else {
                self.move_to_set(drop.tile, drop.set, Some(drop.slot))
            };
            match change {
                Some(change) => {
                    if let Change::ToSet { tile, .. } = &change {
                        self.selected = Some(tile.id);
                    }
                    self.record(change);
                }
                None => self.set_status("This set has no free slot", true),
            }
        }
        if let Some(id) = grid.duplicate {
            self.duplicate_tile(id, None);
        }
        if let Some(id) = grid.choose_emoji {
            self.selected = Some(id);
            self.open_emoji_picker(id);
        }
        if let Some(id) = grid.rename {
            self.selected = Some(id);
            self.act(&ctx, MenuAction::RenameTile);
        }
        for action in sidebar_actions {
            self.apply_sidebar(action);
        }
        for action in inspector_actions {
            match action {
                InspectorAction::Command(command) => self.execute(command),
                InspectorAction::Rename(id, title) => self.rename_tile(id, title),
                InspectorAction::Recolor(id, color) => self.recolor_tile(id, color),
                InspectorAction::Remove(id) => self.remove_tile(id),
                InspectorAction::Polish(id) => self.polish_tile(id),
                InspectorAction::ChooseEmoji(id) => self.open_emoji_picker(id),
                InspectorAction::SetColumns(columns) => self.set_columns(self.current, columns),
            }
        }
        for action in actions {
            self.act(&ctx, action);
        }

        self.autosave(&ctx);
        if acted {
            ctx.request_repaint();
        }
        if !self.playing.is_empty() {
            ctx.request_repaint_after(PLAYING_REPAINT);
        }
        if let Some(remaining) = self.status.as_ref().and_then(Status::remaining) {
            ctx.request_repaint_after(remaining);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.prefs.store(storage);
    }

    fn on_exit(&mut self) {
        self.save_now();
    }
}

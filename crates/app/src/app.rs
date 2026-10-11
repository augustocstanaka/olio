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
use olio_audio::{Engine, PlayOptions, VoiceId};
use olio_control::{Command, TileKey, TileSetting};
use olio_library::{
    Library, Lock, MAX_FADE_SECONDS, Set, SetId, Slot, Store, Tile, TileColor, TileId,
};

use crate::Options;
use crate::clips::{ClipState, Clips};
use crate::confirm::{self, Confirm, Pending, Reply};
use crate::grid::{self, GridLayout, GridView, Progress};
use crate::import::{Importer, Report};
use crate::inspector::{self, InspectorAction};
use crate::menu::{self, Direction, MenuAction, MenuState};
use crate::prefs::{Polish, Preferences, TileStyle};
use crate::sidebar::{self, Rename, SidebarAction};
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
            audio: if options.silent {
                // An engine nobody renders: voices start and are tracked,
                // but nothing is heard.
                Audio::Running(Box::new(Engine::offline(48_000).0))
            } else {
                open_audio()
            },
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
            grid_layout: None,
            screenshot: options.screenshot.map(|path| ScreenshotJob {
                path,
                frames: 0,
                requested: false,
            }),
        };
        app.apply_master_gain();
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
            since: Instant::now(),
        });
    }

    fn changed(&mut self) {
        self.unsaved_since.get_or_insert_with(Instant::now);
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
                    gain: 1.0,
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
                    self.focus_title = true;
                }
            }
            MenuAction::RemoveTile => {
                if let Some(id) = self.selected {
                    self.ask_remove_tile(id);
                }
            }
            MenuAction::ToggleSets => self.prefs.show_sets = !self.prefs.show_sets,
            MenuAction::ToggleInspector => {
                self.prefs.show_inspector = !self.prefs.show_inspector;
            }
            MenuAction::TileStyle(style) => self.prefs.tile_style = style,
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

    fn ask_remove_tile(&mut self, id: TileId) {
        let Some((_, tile)) = self.library.tile(id) else {
            return;
        };
        self.confirm = Some(Confirm {
            title: format!("Remove “{}”?", tile.title),
            body: "The tile leaves this set. The sound file stays in the library.".to_owned(),
            verb: "Remove",
            pending: Pending::RemoveTile(id),
        });
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

    fn remove_tile(&mut self, id: TileId) {
        self.stop_tile(id);
        for set in &mut self.library.sets {
            set.remove_tile(id);
        }
        if self.selected == Some(id) {
            self.selected = None;
        }
        self.changed();
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

    fn remove_set(&mut self, id: SetId) {
        if self.library.sets.len() <= 1 {
            return;
        }
        let Some(index) = self.library.sets.iter().position(|set| set.id == id) else {
            return;
        };
        let tiles: Vec<TileId> = self.library.sets[index]
            .tiles
            .iter()
            .map(|tile| tile.id)
            .collect();
        for tile in tiles {
            self.stop_tile(tile);
        }
        self.library.remove_set(id);
        self.current = self.library.sets[index.min(self.library.sets.len() - 1)].id;
        self.selected = None;
        self.changed();
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
            SidebarAction::Polish(id) => {
                self.execute(Command::SelectSet(id));
                self.polish_set();
            }
        }
    }

    fn resolve(&mut self, pending: Pending) {
        match pending {
            Pending::RemoveTile(id) => self.remove_tile(id),
            Pending::RemoveSet(id) => self.remove_set(id),
        }
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
            ui.separator();

            let dark = ui.visuals().dark_mode;
            let mut playing: Vec<(&str, TileColor, TileId)> = self
                .playing
                .keys()
                .filter_map(|&id| self.library.tile(id))
                .map(|(_, tile)| (tile.title.as_str(), tile.color, tile.id))
                .collect();
            playing.sort_by(|a, b| a.0.cmp(b.0));
            if playing.is_empty() {
                ui.weak("Nothing playing");
            }
            for (title, color, id) in playing {
                let text = RichText::new(format!("■ {title}")).color(palette::strong(color, dark));
                let bubble = egui::Button::new(text).fill(palette::tint(color, dark, 40));
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
                    .selectable_label(self.prefs.show_inspector, "Inspector")
                    .on_hover_text(format!(
                        "Show or hide the inspector ({})",
                        menu::shortcut_text(&ctx, MenuAction::ToggleInspector)
                    ))
                    .clicked()
                {
                    actions.push(MenuAction::ToggleInspector);
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
        self.maintain_audio(&ctx);
        self.request_clips();
        if self
            .selected
            .is_some_and(|id| self.current_set().tile(id).is_none())
        {
            self.selected = None;
        }

        // A confirmation takes the keyboard while it is up.
        if let Some(confirm) = self.confirm.take() {
            match confirm::show(&ctx, &confirm) {
                Reply::Waiting => self.confirm = Some(confirm),
                Reply::Cancelled => {}
                Reply::Confirmed => self.resolve(confirm.pending),
            }
        }
        let mut commands = Vec::new();
        let mut actions = Vec::new();
        if self.confirm.is_none() {
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
                set_removable: self.library.sets.len() > 1,
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
        let sidebar_actions = if self.prefs.show_sets {
            egui::Panel::left("olio-sets")
                .resizable(true)
                .default_size(210.0)
                .min_size(150.0)
                .show(ui, |ui| {
                    sidebar::show(ui, &self.library, self.current, &sounding, &mut self.rename)
                })
                .inner
        } else {
            Vec::new()
        };

        let inspector_actions = if self.prefs.show_inspector {
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
                    files_hovering,
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

        let acted = !commands.is_empty() || !actions.is_empty();
        for command in commands {
            self.execute(command);
        }
        for id in grid.remove {
            self.ask_remove_tile(id);
        }
        for (id, color) in grid.recolor {
            self.recolor_tile(id, color);
        }
        for id in grid.polish {
            self.polish_tile(id);
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
                InspectorAction::Remove(id) => self.ask_remove_tile(id),
                InspectorAction::Polish(id) => self.polish_tile(id),
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

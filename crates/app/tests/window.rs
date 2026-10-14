//! The main window, driven headlessly through its accessibility tree with
//! egui_kittest. Audio is silent: voices start and are tracked, but no
//! device is opened.
//!
//! `OLIO_RENDER=1 cargo test -p olio-app --test window` also renders the
//! window offscreen to `target/screenshots/window.png` (needs a GPU adapter,
//! so it is off by default and in CI).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use olio_app::import::Importer;
use olio_app::prefs::{TileSize, TileStyle};
use olio_app::{OlioApp, Options};
use olio_audio::{Clip, WavFormat};
use olio_library::Store;

const BOARD: &str = r#"{
  "sets": [{
    "name": "Show",
    "columns": 3,
    "tiles": [
      { "file": "Applause.wav", "key": "1", "color": "purple", "fade_out": 1, "emoji": "clap" },
      { "file": "Big Game.wav", "key": "2", "color": "pink", "reverb": 0.4, "emoji": "🏆" },
      { "file": "Country Rock.wav", "key": "3", "color": "red", "loop": true },
      { "file": "Rock Out.wav", "key": "Q", "color": "purple", "fade_in": 0.5 },
      { "file": "Pensive.wav", "key": "W", "color": "pink" },
      { "file": "Tense.wav", "key": "E", "color": "red", "emoji": "scream" }
    ]
  }]
}"#;

const TITLES: [&str; 6] = [
    "Applause",
    "Big Game",
    "Country Rock",
    "Rock Out",
    "Pensive",
    "Tense",
];

/// A library with one board set of six tones, silence around each.
fn library(dir: &Path) -> PathBuf {
    let sounds = dir.join("sounds");
    std::fs::create_dir_all(&sounds).unwrap();
    for (i, name) in TITLES.iter().enumerate() {
        let rate = 22_050;
        let mut samples = vec![0.0f32; rate / 4];
        samples.extend((0..rate * (i + 1)).map(|n| {
            let t = n as f32 / rate as f32;
            let pitch = 220.0 * (i + 2) as f32;
            (t * pitch * std::f32::consts::TAU).sin() * (0.2 + 0.6 * (t * 3.0).sin().abs())
        }));
        samples.extend(vec![0.0f32; rate / 4]);
        Clip::new(rate as u32, 1, samples)
            .write_wav(&sounds.join(format!("{name}.wav")), WavFormat::Int16)
            .unwrap();
    }
    std::fs::write(sounds.join("board.json"), BOARD).unwrap();

    let root = dir.join("library");
    let store = Store::open(&root).unwrap();
    let mut library = store.load().unwrap();
    let first = library.sets[0].id;
    let report =
        Importer::new(&store, &mut library).paths(&[sounds.join("board.json")], first, None);
    assert!(report.problems.is_empty(), "{:?}", report.problems);
    store.save(&library).unwrap();
    root
}

fn window(library: PathBuf, render: bool) -> Harness<'static, OlioApp> {
    let options = Options {
        library: library.clone(),
        silent: true,
        set: Some("Show".to_owned()),
        screenshot: None,
        import: Vec::new(),
    };
    let store = Store::open(library).unwrap();
    // 60 steps a second, so two clicks a step apart make a double-click.
    let mut builder = Harness::builder()
        .with_size([1280.0, 800.0])
        .with_step_dt(1.0 / 60.0);
    if render {
        builder = builder.wgpu();
    }
    let mut harness = builder.build_eframe(move |cc| OlioApp::new(cc, store, options));
    // Sounds decode on a background thread.
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state().is_loading() {
        assert!(Instant::now() < deadline, "sounds never finished loading");
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(3);
    harness
}

#[test]
fn the_window_opens_on_the_chosen_set_with_its_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let harness = window(library(dir.path()), false);
    assert_eq!(harness.state().current_set_name(), "Show");
    for title in TITLES {
        harness.get_by_label(title);
    }
    // The set's name shows in the sidebar and the inspector.
    assert_eq!(harness.query_all_by_label("Show").count(), 2);
    harness.get_by_label("My Sounds");
}

#[test]
fn keys_start_and_stop_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.key_press(egui::Key::Num1);
    harness.key_press(egui::Key::W);
    harness.run_steps(2);
    assert_eq!(harness.state().playing_titles(), ["Applause", "Pensive"]);

    harness.key_press(egui::Key::Num1);
    harness.run_steps(2);
    assert_eq!(harness.state().playing_titles(), ["Pensive"]);

    // Keys with no tile do nothing.
    harness.key_press(egui::Key::P);
    harness.run_steps(2);
    assert_eq!(harness.state().playing_titles(), ["Pensive"]);
}

#[test]
fn double_clicking_a_tile_plays_it_and_its_bubble_stops_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Tense").click();
    harness.run_steps(1);
    harness.get_by_label("Tense").click();
    harness.run_steps(2);
    assert_eq!(harness.state().playing_titles(), ["Tense"]);

    harness.get_by_label("■ Tense").click();
    harness.run_steps(2);
    assert!(harness.state().playing_titles().is_empty());
}

#[test]
fn selecting_a_tile_shows_its_settings_in_the_inspector() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Country Rock").click();
    harness.run_steps(2);
    harness.get_by_label("Loop");
    harness.get_by_label("Trim silence");
    harness.get_by_label("Reverb");
    harness.get_by_label("Remove Tile");
}

#[test]
fn keys_still_play_after_clicking_a_checkbox() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Applause").click();
    harness.run_steps(2);
    harness.get_by_label("Loop").click();
    harness.run_steps(2);
    harness.key_press(egui::Key::E);
    harness.run_steps(2);
    assert_eq!(harness.state().playing_titles(), ["Tense"]);
}

#[test]
fn a_tile_can_be_renamed_with_spaces_in_the_inspector() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Pensive").click();
    harness.run_steps(2);
    harness
        .get_by_role(egui::accesskit::Role::TextInput)
        .click();
    harness.run_steps(1);
    // Tile keys type into the field instead of playing tiles.
    harness.key_press(egui::Key::Q);
    harness.run_steps(1);
    assert!(harness.state().playing_titles().is_empty());
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    for text in ["Very ", "Pensive ", "Mood"] {
        harness.event(egui::Event::Text(text.to_owned()));
        harness.run_steps(1);
    }
    harness.run_steps(2);
    harness.get_by_label("Very Pensive Mood");
}

#[test]
fn arrow_keys_move_the_selection_and_return_plays_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    // Nothing selected: the first tile.
    harness.key_press(egui::Key::ArrowRight);
    harness.run_steps(2);
    assert_eq!(harness.state().selected_title(), Some("Applause"));
    harness.key_press(egui::Key::ArrowRight);
    harness.key_press(egui::Key::ArrowDown);
    harness.run_steps(2);
    assert_eq!(harness.state().selected_title(), Some("Pensive"));
    // Nothing further right on that row.
    harness.key_press(egui::Key::ArrowRight);
    harness.key_press(egui::Key::ArrowRight);
    harness.run_steps(2);
    assert_eq!(harness.state().selected_title(), Some("Tense"));

    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert_eq!(harness.state().playing_titles(), ["Tense"]);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert!(harness.state().playing_titles().is_empty());

    // A tap of Escape clears the selection.
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(harness.state().selected_title(), None);
}

#[test]
fn removing_a_tile_is_immediate_and_undo_puts_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Big Game").click();
    harness.run_steps(2);
    harness.key_press(egui::Key::Backspace);
    harness.run_steps(2);
    assert_eq!(harness.query_all_by_label("Big Game").count(), 0);
    harness.get_by_label("Removed “Big Game”");

    // ⌘Z (Ctrl+Z) puts it back in its slot with its settings, selected.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    let tile = harness
        .state()
        .selected_tile()
        .expect("the tile is back and selected");
    assert_eq!(tile.title, "Big Game");
    assert_eq!((tile.slot.row, tile.slot.column), (0, 1));
    assert_eq!((tile.reverb, tile.emoji.as_str()), (0.4, "🏆"));

    // Redo removes it again; the status bar's Undo button brings it back.
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Z,
    );
    harness.run_steps(2);
    assert_eq!(harness.query_all_by_label("Big Game").count(), 0);
    harness.get_by_label("Undo").click();
    harness.run_steps(2);
    harness.get_by_label("Big Game");
}

#[test]
fn a_removed_set_comes_back_with_its_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Set").click();
    harness.run_steps(2);
    harness.get_by_label_contains("Remove Set…").click();
    // The dialog takes a few frames to size and place itself; until then
    // its buttons are not where the accessibility tree says.
    harness.run_steps(10);
    harness.get_by_label("Remove").click();
    harness.run_steps(2);
    assert_eq!(harness.state().current_set_name(), "My Sounds");
    assert_eq!(harness.query_all_by_label("Applause").count(), 0);

    harness.get_by_label("Edit").click();
    harness.run_steps(2);
    harness
        .get_by_label_contains("Undo Remove Set “Show”")
        .click();
    harness.run_steps(2);
    assert_eq!(harness.state().current_set_name(), "Show");
    for title in TITLES {
        harness.get_by_label(title);
    }
    // Back in its place in the sidebar, after My Sounds.
    harness.get_by_label("Put back the set “Show”");
}

#[test]
fn normalize_is_on_by_default_and_can_be_turned_off() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Pensive").click();
    harness.run_steps(2);
    assert!(harness.state().selected_tile().unwrap().normalize);
    harness.get_by_label_contains("normalized");
    harness.get_by_label("Normalize level").click();
    harness.run_steps(2);
    assert!(!harness.state().selected_tile().unwrap().normalize);
    assert_eq!(harness.query_all_by_label_contains("normalized").count(), 0);
}

#[test]
fn spectrograms_show_and_can_be_hidden() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Tense").click();
    harness.run_steps(2);
    harness.get_by_label("Spectrogram");
    harness.get_by_label("View").click();
    harness.run_steps(2);
    harness.get_by_label_contains("✔ Spectrograms").click();
    harness.run_steps(2);
    assert!(!harness.state().preferences().show_spectrograms);
    assert_eq!(harness.query_all_by_label("Spectrogram").count(), 0);
}

#[test]
fn the_minus_key_fades_everything_out() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.key_press(egui::Key::Num1);
    harness.key_press(egui::Key::Num2);
    harness.run_steps(2);
    assert_eq!(harness.state().playing_titles().len(), 2);
    harness.key_press(egui::Key::Minus);
    harness.run_steps(2);
    assert!(harness.state().playing_titles().is_empty());
}

#[test]
fn the_view_menu_hides_the_inspector_and_a_shortcut_shows_it_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("View").click();
    harness.run_steps(2);
    // Menu items are labelled with their shortcut too.
    harness.get_by_label_contains("✔ Inspector").click();
    harness.run_steps(2);
    assert!(!harness.state().preferences().show_inspector);
    assert_eq!(harness.query_all_by_label("Trim silence").count(), 0);

    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::I,
    );
    harness.run_steps(2);
    assert!(harness.state().preferences().show_inspector);
}

#[test]
fn polish_toggles_the_fades_and_reverb_in_one_click() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Applause").click();
    harness.run_steps(2);
    harness.get_by_label_contains("✨ Polish").click();
    harness.run_steps(2);
    let tile = harness.state().selected_tile().unwrap();
    assert_eq!(
        (tile.fade_in_seconds, tile.fade_out_seconds, tile.reverb),
        (1.0, 1.0, 0.3)
    );
    harness.get_by_label_contains("✔ Polished");

    // The shortcut takes it off again.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    harness.run_steps(2);
    let tile = harness.state().selected_tile().unwrap();
    assert_eq!(
        (tile.fade_in_seconds, tile.fade_out_seconds, tile.reverb),
        (0.0, 0.0, 0.0)
    );
    harness.get_by_label_contains("✨ Polish");
}

#[test]
fn polish_all_polishes_every_tile_of_the_set() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    let polish_all = |harness: &mut Harness<'static, OlioApp>| {
        harness.get_by_label("Tile").click();
        harness.run_steps(2);
        harness
            .get_by_label_contains("Polish All Tiles in Set")
            .click();
        harness.run_steps(2);
    };
    polish_all(&mut harness);
    harness.get_by_label("Polished 6 tiles");
    polish_all(&mut harness);
    harness.get_by_label("Every tile in this set is already polished");

    // A polished tile shows it.
    harness.get_by_label("Big Game").click();
    harness.run_steps(2);
    harness.get_by_label_contains("✔ Polished");
}

#[test]
fn the_polish_preset_opens_from_the_playback_menu() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    assert_eq!(harness.state().preferences().polish.reverb, 0.3);
    harness.get_by_label("Playback").click();
    harness.run_steps(2);
    harness.get_by_label_contains("Polish Preset…").click();
    harness.run_steps(2);
    harness.get_by_label("Polish Preset");
    harness.get_by_label_contains("Reset to 1 s in · 1 s out · 30 % reverb");
}

#[test]
fn an_emoji_is_chosen_by_searching_and_pressing_return() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Pensive").click();
    harness.run_steps(2);
    assert_eq!(harness.state().selected_tile().unwrap().emoji, "");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::E);
    harness.run_steps(2);
    harness.get_by_label("Emoji for “Pensive”");
    // Keys type into the search instead of playing tiles.
    harness.event(egui::Event::Text("tada".to_owned()));
    harness.run_steps(2);
    assert!(harness.state().playing_titles().is_empty());
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert_eq!(harness.state().selected_tile().unwrap().emoji, "🎉");
    assert_eq!(harness.query_all_by_label("Emoji for “Pensive”").count(), 0);
}

#[test]
fn the_inspector_opens_the_picker_and_an_emoji_can_be_removed() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Applause").click();
    harness.run_steps(2);
    assert_eq!(harness.state().selected_tile().unwrap().emoji, "👏");

    // Escape cancels.
    harness.get_by_label("Emoji").click();
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(
        harness.query_all_by_label("Emoji for “Applause”").count(),
        0
    );
    assert_eq!(harness.state().selected_tile().unwrap().emoji, "👏");

    // A click on a suggested emoji picks it.
    harness.get_by_label("Emoji").click();
    harness.run_steps(2);
    harness.get_by_label("party popper").click();
    harness.run_steps(2);
    assert_eq!(harness.state().selected_tile().unwrap().emoji, "🎉");

    // A group tab shows its emoji.
    harness.get_by_label("Emoji").click();
    harness.run_steps(2);
    harness.get_by_label("Animals and nature").click();
    harness.run_steps(2);
    harness.get_by_label("dog face");
    harness.get_by_label("No Emoji").click();
    harness.run_steps(2);
    assert_eq!(harness.state().selected_tile().unwrap().emoji, "");
}

#[test]
fn the_toolbar_and_inspector_show_levels() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Output level");
    harness.get_by_label("Tense").click();
    harness.run_steps(2);
    harness.get_by_label("Tile level");
    // A playing tile's bubble keeps its name beside its emoji.
    harness.key_press(egui::Key::E);
    harness.run_steps(2);
    harness.get_by_label("■ Tense");
    assert_eq!(harness.state().playing_titles(), ["Tense"]);

    // The silent output renders the mix, so the meters move.
    harness.run_steps(10);
    let floor = -48.0;
    assert!(harness.state().output_level_db() > floor);
    assert!(harness.state().tile_level_db("Tense").unwrap() > floor);
    assert_eq!(harness.state().tile_level_db("Pensive"), None);

    // Muted, the output falls silent while the tile's own meter still shows
    // it playing.
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::M,
    );
    harness.run_steps(180);
    assert!(harness.state().output_level_db() < floor);
    assert!(harness.state().tile_level_db("Tense").unwrap() > floor);
}

#[test]
fn the_view_menu_switches_to_soft_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    assert_eq!(harness.state().preferences().tile_style, TileStyle::Flat);
    harness.get_by_label("View").click();
    harness.run_steps(2);
    harness.get_by_label_contains("Soft Tiles").click();
    harness.run_steps(2);
    assert_eq!(harness.state().preferences().tile_style, TileStyle::Soft);
    // Tiles still play in the new style.
    harness.key_press(egui::Key::Num1);
    harness.run_steps(2);
    assert_eq!(harness.state().playing_titles(), ["Applause"]);
}

/// Presses the primary button at `at`, starting a drag.
fn press(harness: &mut Harness<'static, OlioApp>, at: egui::Pos2) {
    harness.hover_at(at);
    harness.run_steps(1);
    harness.drag_at(at);
    harness.run_steps(1);
}

/// Moves the pointer from one point to another in steps, as a hand would.
fn glide(harness: &mut Harness<'static, OlioApp>, from: egui::Pos2, to: egui::Pos2) {
    for step in 1..=10 {
        harness.hover_at(from + (to - from) * (step as f32 / 10.0));
        harness.run_steps(1);
    }
}

/// Releases the primary button at `at` with `modifiers` held.
fn release(harness: &mut Harness<'static, OlioApp>, at: egui::Pos2, modifiers: egui::Modifiers) {
    harness.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers,
    });
    harness.run_steps(2);
}

/// Drags with the primary button from one point to another and releases
/// with `modifiers` held.
fn drag(
    harness: &mut Harness<'static, OlioApp>,
    from: egui::Pos2,
    to: egui::Pos2,
    modifiers: egui::Modifiers,
) {
    press(harness, from);
    glide(harness, from, to);
    release(harness, to, modifiers);
}

fn slot_of(harness: &Harness<'static, OlioApp>) -> (u8, u8) {
    let tile = harness.state().selected_tile().expect("a tile is selected");
    (tile.slot.row, tile.slot.column)
}

fn center(harness: &Harness<'static, OlioApp>, label: &str) -> egui::Pos2 {
    harness.get_by_label(label).rect().center()
}

#[test]
fn dragging_a_tile_to_an_empty_slot_moves_it_and_undo_moves_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    let (from, to) = (
        center(&harness, "Big Game"),
        center(&harness, "Empty slot A"),
    );
    drag(&mut harness, from, to, egui::Modifiers::NONE);
    assert_eq!(harness.state().selected_title(), Some("Big Game"));
    assert_eq!(slot_of(&harness), (2, 0), "now played by A");
    harness.get_by_label("Empty slot 2");

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(slot_of(&harness), (0, 1));
    harness.get_by_label("Moved “Big Game” back");
}

#[test]
fn dropping_a_tile_on_another_swaps_them() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    let (from, to) = (center(&harness, "Applause"), center(&harness, "Tense"));
    drag(&mut harness, from, to, egui::Modifiers::NONE);
    assert_eq!(slot_of(&harness), (1, 2));
    harness.get_by_label("Tense").click();
    harness.run_steps(2);
    assert_eq!(slot_of(&harness), (0, 0));
    // E now plays Applause.
    harness.key_press(egui::Key::E);
    harness.run_steps(2);
    assert_eq!(harness.state().playing_titles(), ["Applause"]);
}

#[test]
fn a_drag_with_the_copy_modifier_drops_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    let (from, to) = (
        center(&harness, "Pensive"),
        center(&harness, "Empty slot S"),
    );
    drag(&mut harness, from, to, egui::Modifiers::ALT);
    assert_eq!(harness.state().selected_title(), Some("Pensive copy"));
    assert_eq!(slot_of(&harness), (2, 1));
    harness.get_by_label("Pensive");
    harness.get_by_label("Duplicated “Pensive”");
}

#[test]
fn duplicate_copies_into_the_next_free_slot_and_undo_takes_it_away() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Country Rock").click();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    harness.run_steps(2);
    let copy = harness.state().selected_tile().unwrap();
    assert_eq!(copy.title, "Country Rock copy");
    assert!(copy.looping, "settings come along");
    // Row 1 is full, so the first free slot after it: A.
    assert_eq!(slot_of(&harness), (2, 0));

    harness.get_by_label("Edit").click();
    harness.run_steps(2);
    harness
        .get_by_label_contains("Undo Duplicate “Country Rock”")
        .click();
    harness.run_steps(2);
    assert_eq!(harness.query_all_by_label("Country Rock copy").count(), 0);
}

#[test]
fn shift_and_an_arrow_moves_the_selected_tile() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Country Rock").click();
    harness.run_steps(2);
    // The last column: nowhere to go right.
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowRight);
    harness.run_steps(2);
    assert_eq!(slot_of(&harness), (0, 2));
    // Down swaps with Tense; the plain arrows still move the selection.
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
    harness.run_steps(2);
    assert_eq!(slot_of(&harness), (1, 2));
    harness.key_press(egui::Key::ArrowUp);
    harness.run_steps(2);
    assert_eq!(harness.state().selected_title(), Some("Tense"));
}

#[test]
fn a_tile_dropped_on_another_set_moves_there_keeping_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    let (from, to) = (center(&harness, "Big Game"), center(&harness, "My Sounds"));
    drag(&mut harness, from, to, egui::Modifiers::NONE);
    harness.get_by_label("Moved “Big Game” to “My Sounds”");
    assert_eq!(
        harness.state().current_set_name(),
        "Show",
        "the front set stays"
    );
    assert_eq!(harness.query_all_by_label("Big Game").count(), 0);

    harness.get_by_label("My Sounds").click();
    harness.run_steps(2);
    harness.get_by_label("Big Game").click();
    harness.run_steps(2);
    assert_eq!(slot_of(&harness), (0, 1), "still played by 2");

    // Undo takes it home and shows it there.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(harness.state().current_set_name(), "Show");
    assert_eq!(harness.state().selected_title(), Some("Big Game"));
    assert_eq!(slot_of(&harness), (0, 1));
}

#[test]
fn a_tile_dropped_on_another_set_with_the_copy_modifier_is_copied() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    let (from, to) = (center(&harness, "Applause"), center(&harness, "My Sounds"));
    drag(&mut harness, from, to, egui::Modifiers::ALT);
    harness.get_by_label("Copied “Applause” to “My Sounds”");
    harness.get_by_label("Applause");

    harness.get_by_label("My Sounds").click();
    harness.run_steps(2);
    harness.get_by_label("Applause");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    harness.get_by_label("Removed the copy of “Applause” from “My Sounds”");
    assert_eq!(harness.query_all_by_label("Applause").count(), 0);
}

#[test]
fn a_tile_dropped_on_its_own_set_stays_put() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    // "Show" is also the inspector's heading: take the sidebar's, on the left.
    let to = harness
        .query_all_by_label("Show")
        .map(|node| node.rect().center())
        .min_by(|a, b| a.x.total_cmp(&b.x))
        .unwrap();
    let from = center(&harness, "Big Game");
    drag(&mut harness, from, to, egui::Modifiers::NONE);
    harness.get_by_label("Big Game").click();
    harness.run_steps(2);
    assert_eq!(slot_of(&harness), (0, 1));
    assert_eq!(harness.query_all_by_label_contains("Moved").count(), 0);
}

#[test]
fn a_set_widens_and_narrows_but_not_past_its_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    assert_eq!(harness.query_all_by_label("Empty slot 4").count(), 0);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::CloseBracket);
    harness.run_steps(2);
    harness.get_by_label("Empty slot 4");
    harness.get_by_label("Empty slot R");

    // Back to three, then no further: tiles sit in the third column.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::OpenBracket);
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::OpenBracket);
    harness.run_steps(2);
    assert_eq!(harness.query_all_by_label("Empty slot 4").count(), 0);
    harness.get_by_label("Country Rock");
    harness.get_by_label_contains("keep “Show” at least that wide");

    // Undo the narrowing.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    harness.get_by_label("Empty slot 4");
}

#[test]
fn the_inspector_sets_the_columns_of_the_set_in_front() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Keys 1–3, Q–E, A–D, Z–C");
    harness.get_by_label("More columns").click();
    harness.run_steps(2);
    harness.get_by_label("More columns").click();
    harness.run_steps(2);
    harness.get_by_label("Keys 1–5, Q–T, A–G, Z–B");
    harness.get_by_label("Empty slot 5");
    harness.get_by_label("Fewer columns").click();
    harness.run_steps(2);
    assert_eq!(harness.query_all_by_label("Empty slot 5").count(), 0);
}

#[test]
fn compact_tiles_are_smaller_and_still_play() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    let regular = harness.get_by_label("Applause").rect().height();
    harness.get_by_label("View").click();
    harness.run_steps(2);
    harness.get_by_label_contains("Compact Tiles").click();
    harness.run_steps(2);
    assert_eq!(harness.state().preferences().tile_size, TileSize::Compact);
    let compact = harness.get_by_label("Applause").rect().height();
    assert!(
        compact < 70.0 && compact < regular,
        "{compact} vs {regular}"
    );
    harness.key_press(egui::Key::Num1);
    harness.run_steps(2);
    assert_eq!(harness.state().playing_titles(), ["Applause"]);
}

/// How many columns the front set shows: the test board's three, plus any
/// empty slots on the number row after them.
fn columns_shown(harness: &Harness<'static, OlioApp>) -> usize {
    3 + "4567890"
        .chars()
        .take_while(|key| {
            harness
                .query_all_by_label(&format!("Empty slot {key}"))
                .count()
                > 0
        })
        .count()
}

/// The left edge of the inspector: the right-most "Show" is its heading.
fn inspector_left(harness: &Harness<'static, OlioApp>) -> f32 {
    harness
        .query_all_by_label("Show")
        .map(|node| node.rect().left())
        .fold(f32::MIN, f32::max)
}

#[test]
fn fit_to_window_widens_the_grid_as_far_as_tiles_still_fit() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Backslash);
    harness.run_steps(2);
    let columns = columns_shown(&harness);
    assert!(columns > 3, "the window has room for more than three");
    // No column hides under the inspector, and tiles keep their size.
    let last = "4567890".chars().nth(columns - 4).unwrap();
    let right = harness
        .get_by_label(&format!("Empty slot {last}"))
        .rect()
        .right();
    assert!(right <= inspector_left(&harness), "{right}");
    assert!(harness.get_by_label("Applause").rect().width() >= 120.0);

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Backslash);
    harness.run_steps(2);
    harness.get_by_label("“Show” already fits the window");

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(columns_shown(&harness), 3);
}

#[test]
fn compact_tiles_fit_more_columns_and_the_inspector_fits_too() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Fit to Window").click();
    harness.run_steps(2);
    let regular = columns_shown(&harness);

    harness.get_by_label("View").click();
    harness.run_steps(2);
    harness.get_by_label_contains("Compact Tiles").click();
    harness.run_steps(2);
    harness.get_by_label("Fit to Window").click();
    harness.run_steps(2);
    let compact = columns_shown(&harness);
    assert!(compact > regular, "{compact} vs {regular}");
}

#[test]
fn a_shortcut_steps_through_the_tile_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    let next = |harness: &mut Harness<'static, OlioApp>| {
        harness.key_press_modifiers(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::T,
        );
        harness.run_steps(2);
        harness.state().preferences().tile_size
    };
    assert_eq!(harness.state().preferences().tile_size, TileSize::Regular);
    assert_eq!(next(&mut harness), TileSize::Large);
    harness.get_by_label_contains("Large Tiles ·");
    assert_eq!(next(&mut harness), TileSize::Compact);
    assert!(harness.get_by_label("Applause").rect().height() < 70.0);
    assert_eq!(next(&mut harness), TileSize::Regular);
}

#[test]
fn the_ambience_switches_on_with_a_shortcut_and_can_be_heard_on_the_meter() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    assert!(!harness.state().preferences().ambience.enabled);
    harness.run_steps(30);
    assert!(harness.state().output_level_db() < -48.0, "silent before");

    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::B,
    );
    harness.run_steps(2);
    assert!(harness.state().preferences().ambience.enabled);
    harness.get_by_label("Ambience: Rain Study");
    // Past the fade-in, the rain shows on the output meter with nothing
    // else playing.
    harness.run_steps(150);
    assert!(
        harness.state().output_level_db() > -48.0,
        "the bed can be heard"
    );

    // Its bubble in the toolbar switches it off.
    harness.get_by_label("■ Rain Study").click();
    harness.run_steps(2);
    harness.get_by_label("Ambience off");
    assert!(!harness.state().preferences().ambience.enabled);
    assert_eq!(harness.query_all_by_label("■ Rain Study").count(), 0);
}

fn open_generator(harness: &mut Harness<'static, OlioApp>) {
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::G,
    );
    harness.run_steps(3);
    assert!(harness.state().preferences().show_generator);
}

/// Clicks a control in the Generator, scrolling to it first.
fn click_in_generator(harness: &mut Harness<'static, OlioApp>, label: &str) {
    harness.get_by_label(label).scroll_to_me();
    harness.run_steps(3);
    harness.get_by_label(label).click();
    harness.run_steps(2);
}

/// Picks a style from the Generator's dropdown.
fn pick_style(harness: &mut Harness<'static, OlioApp>, name: &str) {
    use egui::accesskit::Role;
    harness
        .get_by_role_and_label(Role::ComboBox, "Style")
        .scroll_to_me();
    harness.run_steps(3);
    harness
        .get_by_role_and_label(Role::ComboBox, "Style")
        .click();
    harness.run_steps(2);
    // The list scrolls once there are styles of one's own; and the
    // pointer must be over the list a frame before it presses.
    harness.get_by_label(name).scroll_to_me();
    harness.run_steps(2);
    harness.get_by_label(name).hover();
    harness.run_steps(2);
    harness.get_by_label(name).click();
    harness.run_steps(2);
}

/// Shows every setting in the Generator.
fn show_details(harness: &mut Harness<'static, OlioApp>) {
    harness.get_by_label("Details").click();
    harness.run_steps(3);
    assert!(harness.state().preferences().generator_details);
}

#[test]
fn the_generator_picks_styles_and_a_touch_makes_them_custom() {
    use olio_audio::ambience::{Background, Form, Instrument};
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    open_generator(&mut harness);
    harness.get_by_label("Generate Track");
    // The essentials only, to begin with.
    assert_eq!(harness.query_all_by_label("Declarative").count(), 0);

    pick_style(&mut harness, "Cathedral");
    let settings = harness.state().preferences().ambience;
    assert_eq!(settings.describe(), "Cathedral");
    assert!(settings.layer(Instrument::Drone).enabled);
    assert!(!settings.layer(Instrument::Harp).enabled);

    click_in_generator(&mut harness, "Harp");
    let settings = harness.state().preferences().ambience;
    assert!(settings.layer(Instrument::Harp).enabled);
    assert_eq!(settings.describe(), "Custom: G Aeolian");
    harness.get_by_label_contains("G Aeolian: ");

    click_in_generator(&mut harness, "Waves");
    assert_eq!(
        harness.state().preferences().ambience.background,
        Background::Waves
    );
    click_in_generator(&mut harness, "Piano");
    assert!(
        harness
            .state()
            .preferences()
            .ambience
            .layer(Instrument::Piano)
            .enabled
    );
    show_details(&mut harness);
    // Each instrument's level, notes and register come with details.
    harness.get_by_role_and_label(egui::accesskit::Role::Slider, "Harp notes");
    assert!(
        harness.query_all_by_label("Middle").count() >= 2,
        "a register each"
    );
    click_in_generator(&mut harness, "Declarative");
    harness.get_by_label("Pulse");
    click_in_generator(&mut harness, "Pieces");
    assert!(matches!(
        harness.state().preferences().ambience.form,
        Form::Piece { .. }
    ));
}

/// Sets the slider named `name` to `value`, as a screen reader would.
fn set_slider(harness: &mut Harness<'static, OlioApp>, name: &str, value: f64) {
    use egui::accesskit::Role;
    let slider = harness.get_by_role_and_label(Role::Slider, name);
    slider.scroll_to_me();
    let (target_node, target_tree) = slider.accesskit_node().locate();
    harness.event(egui::Event::AccessKitActionRequest(
        egui::accesskit::ActionRequest {
            target_node,
            target_tree,
            action: egui::accesskit::Action::SetValue,
            data: Some(egui::accesskit::ActionData::NumericValue(value)),
        },
    ));
    harness.run_steps(3);
}

#[test]
fn each_instrument_plays_as_many_notes_as_its_slider_says() {
    use egui::accesskit::Role;
    use olio_audio::Density;
    use olio_audio::ambience::Instrument;
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    open_generator(&mut harness);
    assert_eq!(
        harness
            .query_all_by_role_and_label(Role::Slider, "Harp notes")
            .count(),
        0,
        "with details"
    );
    show_details(&mut harness);
    // Rain Study's harp plays now and then: its slider sits in the middle.
    let harp = harness
        .state()
        .preferences()
        .ambience
        .layer(Instrument::Harp)
        .density;
    assert_eq!(harp, Density::OCCASIONAL);

    set_slider(&mut harness, "Harp notes", 90.0);
    set_slider(&mut harness, "Bass notes", 5.0);
    let settings = harness.state().preferences().ambience;
    assert!((settings.layer(Instrument::Harp).density.amount() - 0.9).abs() < 1e-3);
    assert_eq!(settings.layer(Instrument::Harp).density.name(), "busy");
    assert_eq!(
        settings.layer(Instrument::Bass).density.name(),
        "very sparse"
    );
    assert!(settings.spacing(Instrument::Harp) < settings.spacing(Instrument::Bass));
    assert_eq!(settings.describe(), "Custom: D Dorian");
    set_slider(&mut harness, "Harp level", 30.0);
    assert!(
        (harness
            .state()
            .preferences()
            .ambience
            .layer(Instrument::Harp)
            .level
            - 0.3)
            .abs()
            < 1e-3
    );

    // The drone holds one note: it has no such slider.
    click_in_generator(&mut harness, "Drone");
    assert!(
        harness
            .state()
            .preferences()
            .ambience
            .layer(Instrument::Drone)
            .enabled
    );
    harness.get_by_role_and_label(Role::Slider, "Drone level");
    assert_eq!(
        harness
            .query_all_by_role_and_label(Role::Slider, "Drone notes")
            .count(),
        0
    );
}

#[test]
fn the_background_has_a_level_of_its_own() {
    use egui::accesskit::Role;
    use olio_audio::ambience::DEFAULT_BACKGROUND_LEVEL;
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    open_generator(&mut harness);
    assert_eq!(
        harness.state().preferences().ambience.background_level,
        DEFAULT_BACKGROUND_LEVEL
    );
    // Among the essentials, beside the backgrounds.
    set_slider(&mut harness, "Background level", 30.0);
    let settings = harness.state().preferences().ambience;
    assert!((settings.background_level - 0.3).abs() < 1e-3);
    assert_eq!(settings.describe(), "Custom: D Dorian");
    // No background, nothing to set.
    click_in_generator(&mut harness, "None");
    assert_eq!(
        harness
            .query_all_by_role_and_label(Role::Slider, "Background level")
            .count(),
        0
    );
}

#[test]
fn the_generator_character_sliders_move_and_make_the_settings_custom() {
    use olio_audio::ambience::Mood;
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    open_generator(&mut harness);
    pick_style(&mut harness, "Campfire");
    harness.get_by_label_contains("in the key.");
    assert_eq!(
        harness.state().preferences().ambience.mood(),
        Some(Mood::Gentle)
    );

    // A mood sets all three amounts; the amounts come with details.
    click_in_generator(&mut harness, "Unsettled");
    let settings = harness.state().preferences().ambience;
    assert_eq!(settings.mood(), Some(Mood::Unsettled));
    assert_eq!(settings.chaos, 0.8);
    assert_eq!(harness.query_all_by_label("Chaos").count(), 0);
    click_in_generator(&mut harness, "Gentle");
    show_details(&mut harness);
    let before = harness.state().preferences().ambience;
    assert_eq!(before.chaos, 0.0);

    // Set to exact values, as a screen reader would.
    set_slider(&mut harness, "Chaos", 40.0);
    set_slider(&mut harness, "Variety", 10.0);

    let settings = harness.state().preferences().ambience;
    assert_eq!((settings.chaos, settings.variety), (0.4, 0.1));
    assert_eq!(settings.concentration, before.concentration);
    assert_eq!(settings.describe(), "Custom: D Mixolydian");
    assert_eq!(settings.mood(), None);
    harness.get_by_label("Custom");
    harness.get_by_label_contains("Repeats a short motif, around the chord, with clashes");
}

#[test]
fn an_evolving_ambience_builds_up_and_says_where_it_has_got_to() {
    use olio_audio::ambience::Journey;
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    open_generator(&mut harness);
    click_in_generator(&mut harness, "Evolves");
    let prefs = harness.state().preferences();
    assert_eq!(prefs.ambience.journey, Journey::Evolve);
    // A way of playing the style, not another style.
    assert_eq!(prefs.style_name(), "Rain Study");
    pick_style(&mut harness, "Campfire");
    assert_eq!(
        harness.state().preferences().ambience.journey,
        Journey::Evolve,
        "kept across styles"
    );
    assert_eq!(harness.query_all_by_label_contains("Now:").count(), 0);

    // Playing live, it starts with one instrument and says so.
    click_in_generator(&mut harness, "Play live");
    harness.run_steps(5);
    harness.get_by_label("Now: D Mixolydian, fire; building up.");

    // Steady has nothing to say.
    click_in_generator(&mut harness, "Steady");
    harness.run_steps(3);
    assert_eq!(harness.query_all_by_label_contains("Now:").count(), 0);
}

#[test]
fn surprise_me_plays_something_new_and_back_returns() {
    use olio_audio::ambience::{Journey, Style};
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    open_generator(&mut harness);
    click_in_generator(&mut harness, "Evolves");
    assert!(!harness.state().preferences().ambience.enabled);
    assert_eq!(harness.query_all_by_label("Back").count(), 0);

    click_in_generator(&mut harness, "Surprise Me");
    let first = harness.state().preferences().ambience;
    assert!(first.enabled, "playing at once");
    assert_eq!(first.style, None);
    assert_eq!(first.journey, Journey::Evolve, "played the way chosen");
    assert!((2..=4).contains(&first.playing().count()));
    harness.get_by_label_contains("Surprise: ");
    harness.get_by_label_contains("Save Style… keeps it.");
    click_in_generator(&mut harness, "Surprise Me");
    let second = harness.state().preferences().ambience;
    assert!(!second.same_music(&first), "another");

    // Back to what played before the first of them, still playing.
    click_in_generator(&mut harness, "Back");
    let back = harness.state().preferences().ambience;
    assert_eq!(back.style, Some(Style::RainStudy));
    assert!(back.enabled);
    harness.get_by_label("Back to Rain Study");
    assert_eq!(harness.query_all_by_label("Back").count(), 0);

    // A surprise kept as a style has no Back.
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::R,
    );
    harness.run_steps(3);
    harness.get_by_label("Back");
    pick_style(&mut harness, "Campfire");
    harness.run_steps(2);
    assert_eq!(harness.query_all_by_label("Back").count(), 0);
}

/// Types `name` over whatever the focused field holds, then Return.
fn type_name(harness: &mut Harness<'static, OlioApp>, name: &str) {
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text(name.to_owned()));
    harness.run_steps(1);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
}

#[test]
fn a_custom_style_is_saved_picked_renamed_and_removed() {
    use olio_audio::ambience::{Instrument, Mode, ScaleKind};
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    open_generator(&mut harness);
    // A built-in style is not saved again.
    assert_eq!(harness.query_all_by_label("Save Style…").count(), 0);
    pick_style(&mut harness, "Cathedral");
    click_in_generator(&mut harness, "Harp");

    // Escape leaves nothing saved.
    click_in_generator(&mut harness, "Save Style…");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(harness.state().preferences().styles.is_empty());
    assert_eq!(
        harness
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .count(),
        0
    );

    // Saved under a name, it is what the Generator plays.
    click_in_generator(&mut harness, "Save Style…");
    assert_eq!(
        harness
            .get_by_role(egui::accesskit::Role::TextInput)
            .value()
            .as_deref(),
        Some("G Aeolian"),
        "the key to start from"
    );
    type_name(&mut harness, "Night Harp");
    harness.get_by_label("Saved the style “Night Harp”");
    let prefs = harness.state().preferences();
    assert_eq!(prefs.styles.len(), 1);
    assert_eq!(prefs.styles[0].name, "Night Harp");
    assert_eq!(prefs.style_name(), "Night Harp");
    harness.get_by_label_contains("Your style: G Aeolian, no background;");
    click_in_generator(&mut harness, "New variation");
    assert_eq!(harness.state().preferences().saved_style(), Some(0));

    // Away to a built-in style and back from the list.
    pick_style(&mut harness, "Rain Study");
    assert_eq!(harness.state().preferences().saved_style(), None);
    pick_style(&mut harness, "Night Harp");
    let prefs = harness.state().preferences();
    assert_eq!(prefs.saved_style(), Some(0));
    assert!(prefs.ambience.layer(Instrument::Harp).enabled);
    assert_eq!(prefs.ambience.scale.kind, ScaleKind::Mode(Mode::Aeolian));

    // Renamed; a built-in name is refused.
    click_in_generator(&mut harness, "Rename…");
    type_name(&mut harness, "Cathedral");
    harness.get_by_label("That is a built-in style");
    type_name(&mut harness, "Night Harp 2");
    harness.get_by_label("Renamed the style to “Night Harp 2”");
    assert_eq!(harness.state().preferences().styles[0].name, "Night Harp 2");

    // Removed once confirmed; Undo puts it back, Redo takes it again.
    click_in_generator(&mut harness, "Remove Style");
    // The dialog takes a few frames to size and place itself.
    harness.run_steps(10);
    harness.get_by_label("Remove the style “Night Harp 2”?");
    harness.get_by_label("Remove").click();
    harness.run_steps(3);
    harness.get_by_label("Removed the style “Night Harp 2”");
    assert!(harness.state().preferences().styles.is_empty());
    harness.get_by_label("Save Style…");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(3);
    harness.get_by_label("Put back the style “Night Harp 2”");
    assert_eq!(harness.state().preferences().saved_style(), Some(0));
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Z,
    );
    harness.run_steps(3);
    assert!(harness.state().preferences().styles.is_empty());
}

#[test]
fn a_focused_slider_takes_the_arrow_keys_from_the_tiles() {
    use egui::accesskit::Role;
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Big Game").click();
    harness.run_steps(2);
    open_generator(&mut harness);
    pick_style(&mut harness, "Campfire");
    show_details(&mut harness);
    assert_eq!(harness.state().preferences().ambience.chaos, 0.0);

    // As Tab would leave it.
    harness
        .get_by_role_and_label(Role::Slider, "Chaos")
        .scroll_to_me();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Slider, "Chaos").focus();
    harness.run_steps(2);
    assert!(
        harness
            .get_by_role_and_label(Role::Slider, "Chaos")
            .is_focused()
    );

    harness.key_press(egui::Key::ArrowRight);
    harness.run_steps(2);
    assert!(harness.state().preferences().ambience.chaos > 0.0);
    assert_eq!(harness.state().selected_title(), Some("Big Game"));
    harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowRight);
    harness.run_steps(2);
    assert_eq!(slot_of(&harness), (0, 1), "the tile stays put");

    // A click on a tile takes the focus, and the arrows, back.
    harness.get_by_label("Big Game").click();
    harness.run_steps(2);
    let chaos = harness.state().preferences().ambience.chaos;
    harness.key_press(egui::Key::ArrowRight);
    harness.run_steps(2);
    assert_eq!(harness.state().selected_title(), Some("Country Rock"));
    assert_eq!(harness.state().preferences().ambience.chaos, chaos);
}

#[test]
fn the_generator_takes_the_inspectors_place_and_gives_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Tense").click();
    harness.run_steps(2);
    harness.get_by_label("Trim silence");

    // The Generator button in the status bar.
    harness.get_by_label("Generator").click();
    harness.run_steps(3);
    assert!(harness.state().preferences().show_generator);
    assert_eq!(harness.query_all_by_label("Trim silence").count(), 0);
    harness.get_by_label("Generate Track");

    // The Inspector button in the status bar brings the inspector back.
    harness.get_by_label("Inspector").click();
    harness.run_steps(3);
    assert!(!harness.state().preferences().show_generator);
    harness.get_by_label("Trim silence");
}

/// Steps the window until the generator is done.
fn wait_for_generating(harness: &mut Harness<'static, OlioApp>, seconds: u64) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while harness.state().is_generating() {
        assert!(Instant::now() < deadline, "the generator never finished");
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(3);
}

#[test]
fn a_set_of_moods_lands_as_a_new_set_and_undo_takes_it_off() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    open_generator(&mut harness);
    harness.get_by_label("30 s").click();
    harness.run_steps(2);

    // Cancelled before a track is done, it leaves nothing behind.
    harness.get_by_label("Variations").click();
    harness.run_steps(2);
    assert!(harness.state().is_generating());
    // The button has only just appeared: the pointer goes over it first.
    harness.get_by_label("Cancel").hover();
    harness.run_steps(1);
    harness.get_by_label("Cancel").click();
    wait_for_generating(&mut harness, 60);
    harness.get_by_label("Stopped generating “Rain Study Variations”");
    assert_eq!(harness.state().current_set_name(), "Show");

    harness.get_by_label("Moods").click();
    harness.run_steps(2);
    harness.get_by_label("Generating “Rain Study Moods”: 5 tracks…");
    harness.get_by_label_contains("Generating “Rain Study Moods”: 1 of 5");
    wait_for_generating(&mut harness, 300);
    harness.get_by_label("Generated the set “Rain Study Moods”: 5 tracks");
    assert_eq!(harness.state().current_set_name(), "Rain Study Moods");
    for mood in ["Still", "Gentle", "Wandering", "Restless", "Unsettled"] {
        harness.get_by_label(&format!("Rain Study {mood}"));
    }
    let tile = harness
        .state()
        .selected_tile()
        .expect("the first, selected");
    assert_eq!(tile.title, "Rain Study Still");
    assert_eq!(tile.emoji, "🌧️");

    // One undo takes the whole set off; redo brings it back.
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(3);
    harness.get_by_label("Removed the set “Rain Study Moods”");
    assert_ne!(harness.state().current_set_name(), "Rain Study Moods");
    assert_eq!(harness.query_all_by_label("Rain Study Still").count(), 0);
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Z,
    );
    harness.run_steps(3);
    assert_eq!(harness.state().current_set_name(), "Rain Study Moods");
    harness.get_by_label("Rain Study Gentle");
}

#[test]
fn a_generated_track_lands_on_a_tile_and_undo_takes_it_off() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    open_generator(&mut harness);
    harness.get_by_label("30 s").click();
    harness.run_steps(2);
    assert_eq!(harness.state().preferences().track_seconds, 30);
    harness.get_by_label("Generate Track").click();
    harness.run_steps(2);
    assert!(harness.state().is_generating());
    // The status says so, and the panel shows how far it has got.
    harness.get_by_label("Generating “Rain Study 30 s”…");
    harness.get_by_label("Cancel");

    let deadline = Instant::now() + Duration::from_secs(120);
    while harness.state().is_generating() {
        assert!(Instant::now() < deadline, "the track never finished");
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(3);
    harness.get_by_label("Generated “Rain Study 30 s”");
    let tile = harness
        .state()
        .selected_tile()
        .expect("the new tile, selected");
    assert_eq!(tile.title, "Rain Study 30 s");
    assert_eq!(tile.emoji, "🌧️");
    assert_eq!(harness.state().current_set_name(), "Show");
    harness.get_by_label("Rain Study 30 s");

    // It plays like any sound, once decoded.
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state().is_loading() {
        assert!(Instant::now() < deadline, "the track never loaded");
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.get_by_label("Rain Study 30 s").click();
    harness.run_steps(1);
    harness.get_by_label("Rain Study 30 s").click();
    harness.run_steps(30);
    assert_eq!(harness.state().playing_titles(), ["Rain Study 30 s"]);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Escape);
    harness.run_steps(2);

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(harness.query_all_by_label("Rain Study 30 s").count(), 0);
    harness.get_by_label("Removed the track “Rain Study 30 s”");
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Z,
    );
    harness.run_steps(2);
    harness.get_by_label("Rain Study 30 s");
}

/// Steps for a little longer than a dragged tile must be held over a set
/// to open it (0.6 s at 60 steps a second).
const HOLD_STEPS: usize = 45;

#[test]
fn holding_a_tile_over_a_set_opens_it_to_drop_on_a_slot() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    let (from, set) = (center(&harness, "Big Game"), center(&harness, "My Sounds"));
    press(&mut harness, from);
    glide(&mut harness, from, set);
    assert_eq!(
        harness.state().current_set_name(),
        "Show",
        "passing over opens nothing"
    );
    harness.run_steps(HOLD_STEPS);
    assert_eq!(harness.state().current_set_name(), "My Sounds");

    let slot = center(&harness, "Empty slot Q");
    glide(&mut harness, set, slot);
    release(&mut harness, slot, egui::Modifiers::NONE);
    assert_eq!(
        harness.state().current_set_name(),
        "My Sounds",
        "it stays open"
    );
    assert_eq!(harness.state().selected_title(), Some("Big Game"));
    assert_eq!(slot_of(&harness), (1, 0));
    harness.get_by_label("Moved “Big Game” to “My Sounds”");

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(harness.state().current_set_name(), "Show");
    assert_eq!(slot_of(&harness), (0, 1));
}

#[test]
fn letting_go_elsewhere_after_opening_a_set_goes_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    let (from, set) = (center(&harness, "Big Game"), center(&harness, "My Sounds"));
    press(&mut harness, from);
    glide(&mut harness, from, set);
    harness.run_steps(HOLD_STEPS);
    assert_eq!(harness.state().current_set_name(), "My Sounds");

    // Over the toolbar, where nothing takes a tile.
    let nowhere = egui::pos2(700.0, 64.0);
    glide(&mut harness, set, nowhere);
    release(&mut harness, nowhere, egui::Modifiers::NONE);
    harness.run_steps(2);
    assert_eq!(harness.state().current_set_name(), "Show");
    harness.get_by_label("Big Game").click();
    harness.run_steps(2);
    assert_eq!(slot_of(&harness), (0, 1), "it never moved");
}

#[test]
fn escape_cancels_a_drag_and_goes_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    let (from, set) = (center(&harness, "Big Game"), center(&harness, "My Sounds"));
    press(&mut harness, from);
    glide(&mut harness, from, set);
    harness.run_steps(HOLD_STEPS);
    let slot = center(&harness, "Empty slot Q");
    glide(&mut harness, set, slot);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(harness.state().current_set_name(), "Show");
    // Letting go now drops nothing.
    release(&mut harness, slot, egui::Modifiers::NONE);
    harness.get_by_label("Big Game");
    assert_eq!(harness.query_all_by_label_contains("Moved").count(), 0);
}

#[test]
fn render_the_window_to_a_png() {
    if std::env::var_os("OLIO_RENDER").is_none() {
        eprintln!("skipped: set OLIO_RENDER=1 to render the window");
        return;
    }
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
    std::fs::create_dir_all(&out).unwrap();
    for (theme, soft, name) in [
        (egui::Theme::Dark, false, "window.png"),
        (egui::Theme::Light, false, "window-light.png"),
        (egui::Theme::Dark, true, "window-soft.png"),
        (egui::Theme::Light, true, "window-soft-light.png"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut harness = window(library(dir.path()), true);
        harness.ctx.set_theme(theme);
        if soft {
            harness.get_by_label("View").click();
            harness.run_steps(2);
            harness.get_by_label_contains("Soft Tiles").click();
            harness.run_steps(2);
        }
        harness.get_by_label("Pensive").click();
        harness.key_press(egui::Key::Num2);
        harness.key_press(egui::Key::Q);
        // Long enough for the start dip to finish.
        harness.run_steps(20);
        let image = harness.render().expect("a GPU adapter to render with");
        let path = out.join(name);
        image.save(&path).unwrap();
        println!("rendered {}", path.display());
    }

    // Mid-drag: Big Game held over Tense, which would swap.
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), true);
    let (from, to) = (center(&harness, "Big Game"), center(&harness, "Tense"));
    harness.hover_at(from);
    harness.run_steps(1);
    harness.drag_at(from);
    harness.run_steps(1);
    for step in 1..=10 {
        harness.hover_at(from + (to - from + egui::vec2(18.0, 14.0)) * (step as f32 / 10.0));
        harness.run_steps(1);
    }
    let image = harness.render().expect("a GPU adapter to render with");
    let path = out.join("window-drag.png");
    image.save(&path).unwrap();
    println!("rendered {}", path.display());

    // On over the sidebar: My Sounds would take it.
    let (now, set) = (
        from + (to - from + egui::vec2(18.0, 14.0)),
        center(&harness, "My Sounds"),
    );
    for step in 1..=10 {
        harness.hover_at(now + (set - now) * (step as f32 / 10.0));
        harness.run_steps(1);
    }
    let image = harness.render().expect("a GPU adapter to render with");
    let path = out.join("window-drag-set.png");
    image.save(&path).unwrap();
    println!("rendered {}", path.display());

    // Held over My Sounds: halfway, then open with the tile over Q.
    harness.run_steps(HOLD_STEPS / 2);
    let image = harness.render().expect("a GPU adapter to render with");
    image.save(out.join("window-spring-hold.png")).unwrap();
    harness.run_steps(HOLD_STEPS);
    let slot = center(&harness, "Empty slot Q");
    glide(&mut harness, set, slot);
    let image = harness.render().expect("a GPU adapter to render with");
    image.save(out.join("window-spring-open.png")).unwrap();

    // Compact tiles, six columns wide, two playing.
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), true);
    harness.get_by_label("View").click();
    harness.run_steps(2);
    harness.get_by_label_contains("Compact Tiles").click();
    harness.run_steps(2);
    for _ in 0..3 {
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::CloseBracket);
        harness.run_steps(1);
    }
    harness.get_by_label("Pensive").click();
    harness.key_press(egui::Key::Num2);
    harness.key_press(egui::Key::Q);
    harness.run_steps(20);
    let image = harness.render().expect("a GPU adapter to render with");
    image.save(out.join("window-compact.png")).unwrap();

    // Large tiles, light.
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), true);
    harness.ctx.set_theme(egui::Theme::Light);
    harness.get_by_label("View").click();
    harness.run_steps(2);
    harness.get_by_label_contains("Large Tiles").click();
    harness.run_steps(2);
    harness.key_press(egui::Key::Num2);
    harness.run_steps(20);
    let image = harness.render().expect("a GPU adapter to render with");
    image.save(out.join("window-large.png")).unwrap();

    // The Generator, on Seaside Piano, evolving, playing live.
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), true);
    open_generator(&mut harness);
    pick_style(&mut harness, "Seaside Piano");
    harness.get_by_label("Evolves").click();
    harness.run_steps(2);
    harness.get_by_label("Play live").click();
    harness.run_steps(20);
    harness.get_by_label_contains("building up");
    let image = harness.render().expect("a GPU adapter to render with");
    image.save(out.join("window-generator.png")).unwrap();
    // And with every setting: the background's level, each instrument's
    // level and notes.
    show_details(&mut harness);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Slider, "Bass notes")
        .scroll_to_me();
    harness.run_steps(20);
    let image = harness.render().expect("a GPU adapter to render with");
    image
        .save(out.join("window-generator-details.png"))
        .unwrap();
}

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
use egui_kittest::kittest::Queryable;
use olio_app::import::Importer;
use olio_app::prefs::TileStyle;
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
    harness.get_by_label("File").click();
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

/// Drags with the primary button from one point to another, moving in
/// steps as a hand would, and releases with `modifiers` held.
fn drag(
    harness: &mut Harness<'static, OlioApp>,
    from: egui::Pos2,
    to: egui::Pos2,
    modifiers: egui::Modifiers,
) {
    harness.hover_at(from);
    harness.run_steps(1);
    harness.drag_at(from);
    harness.run_steps(1);
    for step in 1..=10 {
        harness.hover_at(from + (to - from) * (step as f32 / 10.0));
        harness.run_steps(1);
    }
    harness.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers,
    });
    harness.run_steps(2);
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
}

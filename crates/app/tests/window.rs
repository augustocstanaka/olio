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
      { "file": "Applause.wav", "key": "1", "color": "purple", "fade_out": 1 },
      { "file": "Big Game.wav", "key": "2", "color": "pink", "reverb": 0.4 },
      { "file": "Country Rock.wav", "key": "3", "color": "red", "loop": true },
      { "file": "Rock Out.wav", "key": "Q", "color": "purple", "fade_in": 0.5 },
      { "file": "Pensive.wav", "key": "W", "color": "pink" },
      { "file": "Tense.wav", "key": "E", "color": "red" }
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
    harness.get_by_label("Remove Tile…");
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
fn removing_a_tile_asks_first() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), false);
    harness.get_by_label("Big Game").click();
    harness.run_steps(2);
    harness.key_press(egui::Key::Backspace);
    harness.run_steps(2);
    // Cancelled: the tile stays.
    harness.get_by_label("Cancel").click();
    harness.run_steps(2);
    harness.get_by_label("Big Game");
    assert_eq!(harness.query_all_by_label("Cancel").count(), 0);

    harness.key_press(egui::Key::Delete);
    harness.run_steps(2);
    harness.get_by_label("Remove").click();
    harness.run_steps(2);
    assert_eq!(harness.query_all_by_label("Big Game").count(), 0);
    assert_eq!(harness.state().selected_title(), None);
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
}

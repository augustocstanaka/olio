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
fn render_the_window_to_a_png() {
    if std::env::var_os("OLIO_RENDER").is_none() {
        eprintln!("skipped: set OLIO_RENDER=1 to render the window");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut harness = window(library(dir.path()), true);
    harness.get_by_label("Pensive").click();
    harness.key_press(egui::Key::Num2);
    harness.key_press(egui::Key::Q);
    harness.run_steps(4);
    let image = harness.render().expect("a GPU adapter to render with");
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
    std::fs::create_dir_all(&out).unwrap();
    let path = out.join("window.png");
    image.save(&path).unwrap();
    println!("rendered {}", path.display());
}

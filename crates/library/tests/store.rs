use std::fs;

use olio_library::{FORMAT_VERSION, Library, Store, StoreError};

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("library")).unwrap();
    (dir, store)
}

#[test]
fn an_empty_directory_holds_the_default_library() {
    let (_dir, store) = store();
    assert_eq!(store.load().unwrap().sets.len(), 1);
    assert!(store.root().join("sounds").is_dir());
}

#[test]
fn a_saved_library_loads_back_unchanged() {
    let (dir, store) = store();
    let source = dir.path().join("Rock Out.WAV");
    fs::write(&source, b"pretend audio").unwrap();
    let mut library = Library::default();
    let sound = store.import_sound(&source).unwrap();
    library.sets[0].place(sound, None).unwrap();
    library.add_set();

    store.save(&library).unwrap();
    assert_eq!(store.load().unwrap(), library);
}

#[test]
fn saving_keeps_the_previous_file_as_a_backup() {
    let (_dir, store) = store();
    let mut library = Library::default();
    store.save(&library).unwrap();
    let first = fs::read_to_string(store.library_path()).unwrap();
    library.add_set();
    store.save(&library).unwrap();

    let backup = fs::read_to_string(store.root().join("library.json.bak")).unwrap();
    assert_eq!(backup, first);
    assert!(!store.root().join("library.json.tmp").exists());
}

#[test]
fn a_library_from_a_newer_version_is_refused() {
    let (_dir, store) = store();
    let newer = FORMAT_VERSION + 1;
    fs::write(
        store.library_path(),
        format!(r#"{{ "format_version": {newer}, "sets": [], "something_new": true }}"#),
    )
    .unwrap();
    assert!(matches!(
        store.load(),
        Err(StoreError::TooNew { found, .. }) if found == newer
    ));
}

#[test]
fn a_damaged_library_is_reported_not_replaced() {
    let (_dir, store) = store();
    fs::write(store.library_path(), "{ not json").unwrap();
    assert!(matches!(store.load(), Err(StoreError::Parse { .. })));
}

#[test]
fn the_first_format_loads_with_defaults_for_fields_added_since() {
    // Version 1 as first released: tiles without the optional `looping`.
    let (_dir, store) = store();
    fs::write(
        store.library_path(),
        r#"{
          "format_version": 1,
          "sets": [{
            "id": "6f1c3f0e-55a4-4a43-9a4c-1d3c3c0a1b2e",
            "name": "Old",
            "columns": 3,
            "tiles": [{
              "id": "0b8f6d1e-1111-4c4c-8888-123456789abc",
              "slot": { "row": 0, "column": 2 },
              "title": "Anthem",
              "sound": { "hash": "abc", "extension": "wav", "original_name": "Anthem.wav" },
              "color": "purple"
            }]
          }]
        }"#,
    )
    .unwrap();
    let library = store.load().unwrap();
    let tile = &library.sets[0].tiles[0];
    assert_eq!(tile.title, "Anthem");
    assert!(!tile.looping);
    assert!(tile.trim_silence, "trimming is on unless turned off");
    assert_eq!(tile.fade_in_seconds, 0.0);
    assert_eq!(tile.fade_out_seconds, 0.0);
    assert_eq!(tile.reverb, 0.0);
}

#[test]
fn importing_copies_the_file_once_named_by_its_contents() {
    let (dir, store) = store();
    let first = dir.path().join("Anthem.MP3");
    let same_bytes = dir.path().join("copy of anthem.mp3");
    let other = dir.path().join("other");
    fs::write(&first, b"anthem bytes").unwrap();
    fs::write(&same_bytes, b"anthem bytes").unwrap();
    fs::write(&other, b"other bytes").unwrap();

    let a = store.import_sound(&first).unwrap();
    let b = store.import_sound(&same_bytes).unwrap();
    let c = store.import_sound(&other).unwrap();

    assert_eq!(a.hash, b.hash);
    assert_ne!(a.hash, c.hash);
    assert_eq!(a.extension, "mp3");
    assert_eq!(a.original_name, "Anthem.MP3");
    assert_eq!(c.extension, "");
    assert_eq!(fs::read(store.sound_path(&a)).unwrap(), b"anthem bytes");
    assert_eq!(
        fs::read_dir(store.root().join("sounds")).unwrap().count(),
        2
    );

    fs::remove_file(&first).unwrap();
    assert!(
        store.sound_path(&a).exists(),
        "the copy outlives the original"
    );
}

#[test]
fn importing_a_missing_file_fails() {
    let (dir, store) = store();
    assert!(matches!(
        store.import_sound(&dir.path().join("nope.wav")),
        Err(StoreError::Io { .. })
    ));
}

#[test]
fn a_converted_import_stores_what_the_converter_writes() {
    let (dir, store) = store();
    let video = dir.path().join("Clip.MP4");
    fs::write(&video, b"pretend video").unwrap();
    let sound = store
        .import_converted(&video, "wav", |path| {
            fs::write(path, b"extracted audio").map_err(|err| err.to_string())
        })
        .unwrap();
    assert_eq!(sound.extension, "wav");
    assert_eq!(sound.original_name, "Clip.MP4");
    assert_eq!(
        fs::read(store.sound_path(&sound)).unwrap(),
        b"extracted audio"
    );

    // The same video again reuses the conversion without converting.
    let again = store
        .import_converted(&video, "wav", |_| panic!("converted twice"))
        .unwrap();
    assert_eq!(again, sound);
}

#[test]
fn a_failed_conversion_leaves_nothing_behind() {
    let (dir, store) = store();
    let video = dir.path().join("broken.mov");
    fs::write(&video, b"not really a video").unwrap();
    let result = store.import_converted(&video, "wav", |path| {
        fs::write(path, b"half").unwrap();
        Err("no audio track".to_owned())
    });
    assert!(matches!(result, Err(StoreError::Convert { .. })));
    assert_eq!(
        fs::read_dir(store.root().join("sounds")).unwrap().count(),
        0
    );
}

#[test]
fn the_lock_shows_the_library_is_open_until_dropped() {
    let (_dir, store) = store();
    assert!(store.lock_holder().is_none());
    let lock = store.lock().unwrap();
    assert!(store.lock_holder().is_some());
    drop(lock);
    assert!(store.lock_holder().is_none());
}

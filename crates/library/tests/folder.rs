use std::cmp::Ordering;
use std::fs;
use std::path::Path;

use olio_library::folder::{natural_cmp, sorted_files, sorted_subfolders};

#[test]
fn names_sort_alphabetically_ignoring_case_with_numbers_by_value() {
    let mut names = vec![
        "track 10.mp3",
        "Track 2.mp3",
        "applause.wav",
        "Zebra.wav",
        "track 1.mp3",
        "Bell.wav",
        "track 02b.mp3",
    ];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(
        names,
        [
            "applause.wav",
            "Bell.wav",
            "track 1.mp3",
            "Track 2.mp3",
            "track 02b.mp3",
            "track 10.mp3",
            "Zebra.wav",
        ]
    );
    assert_eq!(natural_cmp("a", "a"), Ordering::Equal);
    assert_eq!(natural_cmp("a", "ab"), Ordering::Less);
}

#[test]
fn folder_listings_are_sorted_filtered_and_skip_hidden_files() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "b 10.wav",
        "B 9.wav",
        "a.mp3",
        "notes.txt",
        "._a.mp3",
        ".DS_Store",
    ] {
        fs::write(dir.path().join(name), b"").unwrap();
    }
    fs::create_dir(dir.path().join("Sub 2")).unwrap();
    fs::create_dir(dir.path().join("sub 10")).unwrap();
    fs::create_dir(dir.path().join(".hidden")).unwrap();

    let is_audio = |path: &Path| path.extension().is_some_and(|ext| ext != "txt");
    let files: Vec<String> = sorted_files(dir.path(), is_audio)
        .unwrap()
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, ["a.mp3", "B 9.wav", "b 10.wav"]);

    let folders: Vec<String> = sorted_subfolders(dir.path())
        .unwrap()
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(folders, ["Sub 2", "sub 10"]);
}

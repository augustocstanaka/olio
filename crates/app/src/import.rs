//! Bringing sounds into the library, for the window and for `olio import`:
//!
//! - loose audio files go into a chosen set;
//! - a folder becomes a set named after it, its sounds in natural
//!   alphabetical order, and each subfolder with sounds becomes another;
//! - a `.json` board (see `docs/BOARD-JSON.md`) adds or replaces the sets it
//!   describes.
//!
//! Audio files are copied as they are. Video files (MP4, MOV, M4V, MKV) have
//! their audio track extracted to a WAV file, so the library never holds
//! the video.

use std::fs;
use std::path::{Path, PathBuf};

use olio_audio::WavFormat;
use olio_control::TileKey;
use olio_library::folder::{natural_cmp, sorted_files, sorted_subfolders};
use olio_library::{
    Library, MAX_FADE_SECONDS, MAX_TILES, Manifest, Set, SetId, Slot, Sound, Store, TileSpec,
    columns_to_fit, parse_emoji,
};

/// How deep folder imports look, so a symbolic link loop cannot recurse
/// forever.
const MAX_FOLDER_DEPTH: usize = 8;

/// What an import did.
#[derive(Debug, Default)]
pub struct Report {
    pub tiles: usize,
    /// Sets created or replaced, in order.
    pub sets: Vec<SetId>,
    /// The subset of `sets` that already existed and had their tiles
    /// replaced.
    pub replaced: Vec<SetId>,
    pub problems: Vec<String>,
}

impl Report {
    /// One line for the status bar or the terminal.
    pub fn summary(&self) -> String {
        let sounds = plural(self.tiles, "sound");
        let mut summary = match (self.tiles, self.sets.len()) {
            (0, _) if self.problems.is_empty() => "Nothing to import".to_owned(),
            (0, _) => "Nothing was imported".to_owned(),
            (_, 0) => format!("Added {sounds}"),
            (_, sets) => format!("Added {sounds} in {}", plural(sets, "set")),
        };
        if let Some(first) = self.problems.first() {
            summary.push_str(&format!(": {first}"));
            if self.problems.len() > 1 {
                summary.push_str(&format!(" (and {} more)", self.problems.len() - 1));
            }
        }
        summary
    }
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn is_board(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

pub struct Importer<'a> {
    store: &'a Store,
    library: &'a mut Library,
}

impl<'a> Importer<'a> {
    pub fn new(store: &'a Store, library: &'a mut Library) -> Self {
        Self { store, library }
    }

    /// Imports a mix of boards, folders and loose files. Loose files go into
    /// `set`, the first at `slot` if it is free.
    pub fn paths(&mut self, paths: &[PathBuf], set: SetId, slot: Option<Slot>) -> Report {
        let mut report = Report::default();
        let mut loose = Vec::new();
        for path in paths {
            if path.is_dir() {
                self.folder(path, &mut report, 0);
            } else if is_board(path) {
                self.board(path, &mut report);
            } else {
                loose.push(path.clone());
            }
        }
        loose.sort_by(|a, b| natural_cmp(&display_name(a), &display_name(b)));
        self.files_into(set, &loose, slot, &mut report);
        report
    }

    /// The library's copy of one file: copied if it is audio, extracted if
    /// it is a video.
    fn sound(&self, path: &Path) -> Result<Sound, String> {
        let name = display_name(path);
        if !olio_audio::is_supported(path) {
            return Err(format!("{name} is not a supported audio or video file"));
        }
        let imported = if olio_audio::is_video(path) {
            self.store.import_converted(path, "wav", |copy| {
                let clip = olio_audio::decode_file(path).map_err(|err| err.to_string())?;
                clip.write_wav(copy, WavFormat::Int16)
                    .map_err(|err| err.to_string())
            })
        } else {
            self.store.import_sound(path)
        };
        imported.map_err(|err| err.to_string())
    }

    fn files_into(
        &mut self,
        set: SetId,
        files: &[PathBuf],
        slot: Option<Slot>,
        report: &mut Report,
    ) {
        let mut slot = slot;
        for file in files {
            let sound = match self.sound(file) {
                Ok(sound) => sound,
                Err(problem) => {
                    report.problems.push(problem);
                    continue;
                }
            };
            let Some(target) = self.library.set_mut(set) else {
                return;
            };
            if target.place(sound, slot.take()).is_none() {
                report.problems.push(format!("“{}” is full", target.name));
                return;
            }
            report.tiles += 1;
        }
    }

    /// A folder's sounds as one or more new sets, then its subfolders.
    pub fn folder(&mut self, folder: &Path, report: &mut Report, depth: usize) {
        let name = display_name(folder);
        match sorted_files(folder, olio_audio::is_supported) {
            Ok(files) if !files.is_empty() => self.new_sets(&name, &files, report),
            Ok(_) => {}
            Err(err) => report.problems.push(format!("{name}: {err}")),
        }
        if depth >= MAX_FOLDER_DEPTH {
            return;
        }
        match sorted_subfolders(folder) {
            Ok(subfolders) => {
                for subfolder in subfolders {
                    self.folder(&subfolder, report, depth + 1);
                }
            }
            Err(err) => report.problems.push(format!("{name}: {err}")),
        }
    }

    /// New sets named `name` (then "name (2)", ...) holding `files` in order,
    /// up to a full set each.
    fn new_sets(&mut self, name: &str, files: &[PathBuf], report: &mut Report) {
        for (part, chunk) in files.chunks(MAX_TILES).enumerate() {
            let base = match part {
                0 => name.to_owned(),
                n => format!("{name} ({})", n + 1),
            };
            let mut set = Set::new(self.library.unique_set_name(&base));
            set.columns = columns_to_fit(chunk.len());
            let id = set.id;
            self.library.sets.push(set);
            let before = report.tiles;
            self.files_into(id, chunk, None, report);
            if report.tiles == before {
                self.library.remove_set(id);
            } else {
                report.sets.push(id);
            }
        }
    }

    /// The sets a board describes.
    pub fn board(&mut self, path: &Path, report: &mut Report) {
        let name = display_name(path);
        let manifest = match fs::read_to_string(path)
            .map_err(|err| err.to_string())
            .and_then(|text| Manifest::parse(&text).map_err(|err| err.to_string()))
        {
            Ok(manifest) => manifest,
            Err(err) => {
                report.problems.push(format!("{name}: {err}"));
                return;
            }
        };
        let base = path.parent().unwrap_or(Path::new("."));
        for spec in manifest.sets {
            self.board_set(base, spec, report);
        }
    }

    fn board_set(&mut self, base: &Path, spec: olio_library::ManifestSet, report: &mut Report) {
        let name = spec.name.trim().to_owned();
        let mut tiles: Vec<TileSpec> = spec.tiles.iter().map(|tile| tile.spec()).collect();
        if let Some(folder) = &spec.folder {
            match sorted_files(&base.join(folder), olio_audio::is_supported) {
                Ok(files) => tiles.extend(files.into_iter().map(|file| TileSpec {
                    file,
                    ..TileSpec::default()
                })),
                Err(err) => report
                    .problems
                    .push(format!("“{name}”: folder {}: {err}", folder.display())),
            }
        }

        // Keys first, so keyed tiles get their slots; the rest fill in order.
        let mut keyed = Vec::new();
        let mut unkeyed = Vec::new();
        for tile in tiles {
            let slot = match tile.key.as_deref() {
                None => None,
                Some(text) => match TileKey::parse(text).and_then(TileKey::slot) {
                    Some(slot) => Some(slot),
                    None => {
                        report
                            .problems
                            .push(format!("“{name}”: “{text}” is not a key on the grid"));
                        None
                    }
                },
            };
            match slot {
                Some(slot) => keyed.push((tile, Some(slot))),
                None => unkeyed.push((tile, None)),
            }
        }
        let widest_key = keyed
            .iter()
            .filter_map(|(_, slot)| slot.map(|slot| slot.column + 1))
            .max()
            .unwrap_or(1);
        let columns = spec
            .columns
            .unwrap_or_else(|| columns_to_fit(keyed.len() + unkeyed.len()).max(widest_key));

        let existing = spec
            .replace
            .then(|| self.library.sets.iter().position(|set| set.name == name))
            .flatten();
        let id = match existing {
            Some(index) => {
                let set = &mut self.library.sets[index];
                set.tiles.clear();
                set.columns = columns;
                report.replaced.push(set.id);
                set.id
            }
            None => {
                let mut set = Set::new(self.library.unique_set_name(&name));
                set.columns = columns;
                let id = set.id;
                self.library.sets.push(set);
                id
            }
        };
        report.sets.push(id);

        for (tile, slot) in keyed.into_iter().chain(unkeyed) {
            let file = base.join(&tile.file);
            let sound = match self.sound(&file) {
                Ok(sound) => sound,
                Err(problem) => {
                    report.problems.push(format!("“{name}”: {problem}"));
                    continue;
                }
            };
            let Some(set) = self.library.set_mut(id) else {
                return;
            };
            if let Some(slot) = slot
                && !set.is_free(slot)
            {
                let key = tile.key.as_deref().unwrap_or_default();
                let problem = if set.contains(slot) {
                    format!("“{name}”: key {key} is used twice")
                } else {
                    format!("“{name}”: key {key} is outside its {columns} columns")
                };
                report.problems.push(problem);
            }
            let Some(tile_id) = set.place(sound, slot) else {
                report.problems.push(format!("“{name}” is full"));
                return;
            };
            let placed = set.tile_mut(tile_id).expect("just placed");
            if let Some(title) = tile.title.filter(|title| !title.trim().is_empty()) {
                placed.title = title.trim().to_owned();
            }
            if let Some(color) = tile.color {
                placed.color = color;
            }
            if let Some(looping) = tile.looping {
                placed.looping = looping;
            }
            if let Some(trim) = tile.trim_silence {
                placed.trim_silence = trim;
            }
            placed.fade_in_seconds = tile.fade_in.unwrap_or(0.0).clamp(0.0, MAX_FADE_SECONDS);
            placed.fade_out_seconds = tile.fade_out.unwrap_or(0.0).clamp(0.0, MAX_FADE_SECONDS);
            placed.reverb = tile.reverb.unwrap_or(0.0).clamp(0.0, 1.0);
            if let Some(emoji) = tile.emoji.as_deref().and_then(parse_emoji) {
                placed.emoji = emoji.to_owned();
            }
            report.tiles += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use olio_audio::Clip;
    use olio_library::TileColor;

    struct Fixture {
        dir: tempfile::TempDir,
        store: Store,
        library: Library,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open(dir.path().join("library")).unwrap();
            Self {
                dir,
                store,
                library: Library::default(),
            }
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.dir.path().join(relative)
        }

        /// Writes a short tone; `pitch` makes files differ in content.
        fn wav(&self, relative: &str, pitch: f32) -> PathBuf {
            let path = self.path(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let samples = (0..800).map(|i| (i as f32 * pitch).sin() * 0.5).collect();
            Clip::new(8_000, 1, samples)
                .write_wav(&path, WavFormat::Int16)
                .unwrap();
            path
        }

        fn import(&mut self, paths: &[PathBuf]) -> Report {
            let first = self.library.sets[0].id;
            Importer::new(&self.store, &mut self.library).paths(paths, first, None)
        }

        fn set(&self, name: &str) -> &Set {
            self.library
                .sets
                .iter()
                .find(|set| set.name == name)
                .unwrap_or_else(|| panic!("no set {name}"))
        }
    }

    fn titles_in_slot_order(set: &Set) -> Vec<String> {
        let mut tiles: Vec<_> = set.tiles.iter().collect();
        tiles.sort_by_key(|tile| tile.slot);
        tiles.iter().map(|tile| tile.title.clone()).collect()
    }

    #[test]
    fn a_folder_becomes_a_set_in_natural_alphabetical_order() {
        let mut fixture = Fixture::new();
        for (i, name) in ["Track 10.wav", "track 2.mp3", "Applause.wav", "readme.txt"]
            .iter()
            .enumerate()
        {
            if name.ends_with(".txt") {
                fs::write(fixture.path(&format!("Show/{name}")), b"notes").unwrap();
            } else {
                fixture.wav(&format!("Show/{name}"), 0.01 + i as f32 * 0.01);
            }
        }
        fixture.wav("Show/Extras/zap.wav", 0.2);

        let report = fixture.import(&[fixture.path("Show")]);

        assert_eq!(report.tiles, 4, "{:?}", report.problems);
        assert_eq!(report.sets.len(), 2);
        assert_eq!(
            titles_in_slot_order(fixture.set("Show")),
            ["Applause", "track 2", "Track 10"]
        );
        assert_eq!(titles_in_slot_order(fixture.set("Extras")), ["zap"]);
        assert!(report.problems.is_empty(), "text files are skipped quietly");
    }

    #[test]
    fn a_folder_with_more_sounds_than_a_set_holds_fills_several() {
        let mut fixture = Fixture::new();
        let one = fixture.wav("Big/000.wav", 0.05);
        for i in 1..(MAX_TILES + 5) {
            fs::copy(&one, fixture.path(&format!("Big/{i:03}.wav"))).unwrap();
        }
        let report = fixture.import(&[fixture.path("Big")]);
        assert_eq!(report.tiles, MAX_TILES + 5);
        assert_eq!(fixture.set("Big").tiles.len(), MAX_TILES);
        assert_eq!(fixture.set("Big (2)").tiles.len(), 5);
        assert_eq!(fixture.set("Big").columns, 10);
    }

    #[test]
    fn a_video_is_kept_as_its_audio_track() {
        let mut fixture = Fixture::new();
        let video =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../audio/tests/fixtures/tone-video.mp4");
        let report = fixture.import(&[video]);
        assert_eq!(report.tiles, 1, "{:?}", report.problems);
        let tile = &fixture.library.sets[0].tiles[0];
        assert_eq!(tile.sound.extension, "wav");
        assert_eq!(tile.title, "tone-video");
        let clip = olio_audio::decode_file(&fixture.store.sound_path(&tile.sound)).unwrap();
        assert!((clip.duration().as_secs_f64() - 0.5).abs() < 0.1);
    }

    #[test]
    fn loose_files_go_into_the_set_in_order_and_bad_ones_are_reported() {
        let mut fixture = Fixture::new();
        let b = fixture.wav("b.wav", 0.02);
        let a = fixture.wav("a.wav", 0.03);
        let text = fixture.path("notes.txt");
        fs::write(&text, b"hello").unwrap();
        let report = fixture.import(&[b, text, a]);
        assert_eq!(report.tiles, 2);
        assert!(report.sets.is_empty());
        assert_eq!(titles_in_slot_order(&fixture.library.sets[0]), ["a", "b"]);
        assert_eq!(report.problems.len(), 1);
        assert!(report.summary().starts_with("Added 2 sounds: notes.txt"));
    }

    const BOARD: &str = r#"{
      "sets": [
        {
          "name": "Show",
          "replace": true,
          "tiles": [
            "sounds/applause.wav",
            {
              "file": "sounds/theme.wav",
              "title": "Main Theme",
              "key": "Q",
              "color": "blue",
              "loop": true,
              "trim_silence": false,
              "fade_in": 0.5,
              "fade_out": 2,
              "reverb": 0.25,
              "emoji": "musical_note"
            },
            { "file": "sounds/missing.wav" },
            { "file": "sounds/applause.wav", "key": "nope" }
          ],
          "folder": "more"
        }
      ]
    }"#;

    #[test]
    fn a_board_builds_its_sets_with_keys_and_settings() {
        let mut fixture = Fixture::new();
        fixture.wav("boards/sounds/applause.wav", 0.02);
        fixture.wav("boards/sounds/theme.wav", 0.03);
        fixture.wav("boards/more/b.wav", 0.04);
        fixture.wav("boards/more/a.wav", 0.05);
        fs::write(fixture.path("boards/show.json"), BOARD).unwrap();

        let report = fixture.import(&[fixture.path("boards/show.json")]);

        assert_eq!(report.tiles, 5);
        let show = fixture.set("Show");
        let theme = show.tile_at(Slot::new(1, 0)).expect("Q is row 1, column 0");
        assert_eq!(theme.title, "Main Theme");
        assert_eq!(theme.color, TileColor::Blue);
        assert!(theme.looping && !theme.trim_silence);
        assert_eq!(
            (theme.fade_in_seconds, theme.fade_out_seconds, theme.reverb),
            (0.5, 2.0, 0.25)
        );
        assert_eq!(theme.emoji, "🎵");
        assert_eq!(
            titles_in_slot_order(show),
            ["applause", "applause", "a", "b", "Main Theme"],
            "unkeyed tiles fill row 0 in order, then the folder alphabetically"
        );
        assert_eq!(report.problems.len(), 2, "{:?}", report.problems);
        assert!(report.problems.iter().any(|p| p.contains("missing.wav")));
        assert!(
            report
                .problems
                .iter()
                .any(|p| p.contains("“nope” is not a key"))
        );
    }

    #[test]
    fn importing_a_board_with_replace_twice_gives_the_same_board() {
        let mut fixture = Fixture::new();
        fixture.wav("boards/sounds/applause.wav", 0.02);
        fixture.wav("boards/sounds/theme.wav", 0.03);
        fs::write(fixture.path("boards/show.json"), BOARD).unwrap();
        let board = fixture.path("boards/show.json");

        fixture.import(std::slice::from_ref(&board));
        let sets = fixture.library.sets.len();
        let tiles = fixture.set("Show").tiles.len();
        let report = fixture.import(&[board]);

        assert_eq!(fixture.library.sets.len(), sets);
        assert_eq!(fixture.set("Show").tiles.len(), tiles);
        assert_eq!(report.replaced, report.sets);
    }

    #[test]
    fn a_board_without_replace_adds_a_renamed_set() {
        let mut fixture = Fixture::new();
        fixture.wav("a.wav", 0.02);
        fs::write(
            fixture.path("board.json"),
            r#"{ "sets": [ { "name": "My Sounds", "tiles": [ "a.wav" ] } ] }"#,
        )
        .unwrap();
        let report = fixture.import(&[fixture.path("board.json")]);
        assert_eq!(report.tiles, 1);
        assert_eq!(fixture.set("My Sounds 2").tiles.len(), 1);
        assert!(fixture.set("My Sounds").tiles.is_empty(), "untouched");
    }

    #[test]
    fn a_broken_board_changes_nothing_and_says_why() {
        let mut fixture = Fixture::new();
        fs::write(
            fixture.path("bad.json"),
            r#"{ "sets": [ { "nme": "x" } ] }"#,
        )
        .unwrap();
        let before = fixture.library.clone();
        let report = fixture.import(&[fixture.path("bad.json")]);
        assert_eq!(fixture.library, before);
        assert!(
            report.summary().contains("unknown field `nme`"),
            "{}",
            report.summary()
        );
    }
}

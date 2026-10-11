use std::path::PathBuf;

use olio_library::{Manifest, ManifestError, ManifestTile, TileColor};

#[test]
fn a_board_reads_paths_and_detailed_tiles() {
    let manifest = Manifest::parse(
        r#"{
          "sets": [
            {
              "name": "Show Intro",
              "columns": 3,
              "tiles": [
                "sounds/applause.wav",
                {
                  "file": "music/theme.mp3",
                  "title": "Theme",
                  "key": "alt+q",
                  "color": "pink",
                  "loop": true,
                  "trim_silence": false,
                  "fade_in": 0.5,
                  "fade_out": 2,
                  "reverb": 0.3
                }
              ]
            },
            { "name": "Sound Effects", "folder": "sfx", "replace": true }
          ]
        }"#,
    )
    .unwrap();

    let intro = &manifest.sets[0];
    assert_eq!(intro.columns, Some(3));
    assert!(!intro.replace);
    assert_eq!(
        intro.tiles[0],
        ManifestTile::File(PathBuf::from("sounds/applause.wav"))
    );
    let theme = intro.tiles[1].spec();
    assert_eq!(theme.file, PathBuf::from("music/theme.mp3"));
    assert_eq!(theme.title.as_deref(), Some("Theme"));
    assert_eq!(theme.key.as_deref(), Some("alt+q"));
    assert_eq!(theme.color, Some(TileColor::Pink));
    assert_eq!(theme.looping, Some(true));
    assert_eq!(theme.trim_silence, Some(false));
    assert_eq!(theme.fade_in, Some(0.5));
    assert_eq!(theme.fade_out, Some(2.0));
    assert_eq!(theme.reverb, Some(0.3));

    let effects = &manifest.sets[1];
    assert_eq!(effects.folder, Some(PathBuf::from("sfx")));
    assert!(effects.replace);
    assert!(effects.tiles.is_empty());
}

#[track_caller]
fn assert_rejected(json: &str, mentions: &str) {
    let message = match Manifest::parse(json) {
        Ok(_) => panic!("accepted: {json}"),
        Err(err @ (ManifestError::Json(_) | ManifestError::Invalid(_))) => err.to_string(),
    };
    assert!(
        message.contains(mentions),
        "{message:?} does not mention {mentions:?}"
    );
}

#[test]
fn mistakes_are_reported_precisely() {
    assert_rejected(
        r#"{ "sets": [ { "name": "A", "tiles": [ { "file": "a.wav", "loops": true } ] } ] }"#,
        "unknown field `loops`",
    );
    assert_rejected(
        r#"{ "sets": [ { "name": "A", "colums": 3 } ] }"#,
        "unknown field `colums`",
    );
    assert_rejected(
        r#"{ "sets": [ { "name": "A", "tiles": [ 42 ] } ] }"#,
        "a file path",
    );
    assert_rejected(r#"{ "sets": [ { "name": " " } ] }"#, "empty name");
    assert_rejected(
        r#"{ "sets": [ { "name": "A", "columns": 11 } ] }"#,
        "columns must be 1 to 10",
    );
    assert_rejected(
        r#"{ "sets": [ { "name": "A", "tiles": [ { "file": "a.wav", "reverb": 50 } ] } ] }"#,
        "reverb must be 0 to 1",
    );
    assert_rejected(
        r#"{ "sets": [ { "name": "A", "tiles": [ { "file": "a.wav", "fade_in": -1 } ] } ] }"#,
        "fade_in must be 0 to",
    );
    assert_rejected(
        r#"{ "sets": [ { "name": "A", "tiles": [ { "file": "a.wav", "color": "mauve" } ] } ] }"#,
        "unknown variant `mauve`",
    );
    assert_rejected(
        r#"{ "sets": [ { "name": "A", "tiles": [ { "file": "a.wav", "emoji": "party" } ] } ] }"#,
        "emoji must be one emoji or its shortcode",
    );
    assert_rejected("{ \"sets\": [ ", "line 1");
}

#[test]
fn emoji_are_read_as_emoji_or_shortcodes() {
    use olio_library::parse_emoji;
    assert_eq!(parse_emoji("🎉"), Some("🎉"));
    assert_eq!(parse_emoji("tada"), Some("🎉"));
    assert_eq!(parse_emoji(":clap:"), Some("👏"));
    // The variation selector is added, so one emoji is always stored one way.
    assert_eq!(parse_emoji("\u{263a}"), Some("\u{263a}\u{fe0f}"));
    assert_eq!(parse_emoji("🎉🎉"), None);
    assert_eq!(parse_emoji("hello"), None);
    assert_eq!(parse_emoji(""), None);
}

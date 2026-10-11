//! Boards described in JSON: an easy format to write by hand or generate
//! from a script, imported into the library in one go. `docs/BOARD-JSON.md`
//! is the user-facing reference.
//!
//! ```json
//! {
//!   "sets": [
//!     {
//!       "name": "Show Intro",
//!       "tiles": [
//!         "sounds/applause.wav",
//!         { "file": "music/theme.mp3", "key": "Q", "loop": true, "fade_out": 2 }
//!       ]
//!     },
//!     { "name": "Sound Effects", "folder": "sfx" }
//!   ]
//! }
//! ```
//!
//! Paths are relative to the JSON file. Unknown fields are errors, so a typo
//! is reported rather than ignored.

use std::fmt;
use std::path::PathBuf;

use serde::Deserialize;
use serde::de::{self, MapAccess, Visitor};

use crate::model::{MAX_COLUMNS, MAX_FADE_SECONDS, TileColor, parse_emoji};

/// A board to import: a list of sets.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub sets: Vec<ManifestSet>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestSet {
    pub name: String,
    /// Grid width, 1 to 10. Defaults to fitting the tiles in about four rows.
    #[serde(default)]
    pub columns: Option<u8>,
    /// Replace the tiles of an existing set with this name, so running the
    /// same file again gives the same board. Otherwise a set with a name
    /// already in use is added as "Name 2".
    #[serde(default)]
    pub replace: bool,
    /// A folder whose audio files are added after `tiles`, in alphabetical
    /// order.
    #[serde(default)]
    pub folder: Option<PathBuf>,
    #[serde(default)]
    pub tiles: Vec<ManifestTile>,
}

/// A tile: just a file path, or a file with settings.
#[derive(Debug, Clone, PartialEq)]
pub enum ManifestTile {
    File(PathBuf),
    Detailed(TileSpec),
}

impl ManifestTile {
    pub fn file(&self) -> &PathBuf {
        match self {
            ManifestTile::File(file) => file,
            ManifestTile::Detailed(spec) => &spec.file,
        }
    }

    /// The tile's settings; all defaults for a bare path.
    pub fn spec(&self) -> TileSpec {
        match self {
            ManifestTile::File(file) => TileSpec {
                file: file.clone(),
                ..TileSpec::default()
            },
            ManifestTile::Detailed(spec) => spec.clone(),
        }
    }
}

/// A tile's file and settings. Anything left out takes the usual default.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileSpec {
    pub file: PathBuf,
    pub title: Option<String>,
    /// The key that plays it: "1" to "0", "Q" to "P", "A" to ";", "Z" to
    /// "/", optionally with "Alt+" (or "Option+", "⌥") for the lower four
    /// rows. Without a key, the tile takes the next free slot.
    pub key: Option<String>,
    pub color: Option<TileColor>,
    #[serde(rename = "loop")]
    pub looping: Option<bool>,
    pub trim_silence: Option<bool>,
    /// Play at the common loudness (default true).
    pub normalize: Option<bool>,
    /// Seconds.
    pub fade_in: Option<f32>,
    /// Seconds.
    pub fade_out: Option<f32>,
    /// 0 (dry) to 1.
    pub reverb: Option<f32>,
    /// One emoji, or its GitHub shortcode ("tada").
    pub emoji: Option<String>,
}

impl<'de> Deserialize<'de> for ManifestTile {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TileVisitor;

        impl<'de> Visitor<'de> for TileVisitor {
            type Value = ManifestTile;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a file path, or an object with a \"file\" field")
            }

            fn visit_str<E: de::Error>(self, path: &str) -> Result<ManifestTile, E> {
                Ok(ManifestTile::File(PathBuf::from(path)))
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<ManifestTile, M::Error> {
                TileSpec::deserialize(de::value::MapAccessDeserializer::new(map))
                    .map(ManifestTile::Detailed)
            }
        }

        deserializer.deserialize_any(TileVisitor)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

impl Manifest {
    /// Parses and checks a board. Errors name the line and column, or the
    /// set and tile, at fault.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let manifest: Manifest = serde_json::from_str(text)?;
        manifest.check()?;
        Ok(manifest)
    }

    fn check(&self) -> Result<(), ManifestError> {
        let invalid = |message: String| Err(ManifestError::Invalid(message));
        for (index, set) in self.sets.iter().enumerate() {
            let set_name = set.name.trim();
            if set_name.is_empty() {
                return invalid(format!("set {} has an empty name", index + 1));
            }
            if let Some(columns) = set.columns
                && !(1..=MAX_COLUMNS).contains(&columns)
            {
                return invalid(format!(
                    "set \"{set_name}\": columns must be 1 to {MAX_COLUMNS}, not {columns}"
                ));
            }
            for tile in &set.tiles {
                let spec = tile.spec();
                let file = spec.file.display();
                if spec.file.as_os_str().is_empty() {
                    return invalid(format!("set \"{set_name}\": a tile has no file"));
                }
                for (name, value) in [("fade_in", spec.fade_in), ("fade_out", spec.fade_out)] {
                    if let Some(seconds) = value
                        && !(0.0..=MAX_FADE_SECONDS).contains(&seconds)
                    {
                        return invalid(format!(
                            "set \"{set_name}\", {file}: {name} must be 0 to {MAX_FADE_SECONDS} seconds"
                        ));
                    }
                }
                if let Some(reverb) = spec.reverb
                    && !(0.0..=1.0).contains(&reverb)
                {
                    return invalid(format!(
                        "set \"{set_name}\", {file}: reverb must be 0 to 1, not {reverb}"
                    ));
                }
                if let Some(emoji) = &spec.emoji
                    && parse_emoji(emoji).is_none()
                {
                    return invalid(format!(
                        "set \"{set_name}\", {file}: emoji must be one emoji or its shortcode (like \"tada\"), not \"{emoji}\""
                    ));
                }
            }
        }
        Ok(())
    }
}

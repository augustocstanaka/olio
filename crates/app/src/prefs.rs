//! Window preferences: theme, master volume, which panels are shown.
//!
//! They are not part of the library format. eframe keeps them, with the
//! window's size and the panel widths, in `preferences.ron` beside the
//! library, so a scratch library opened with `--library` has its own.

use egui::ThemePreference;
use olio_control::TileSetting;
use olio_library::{MAX_FADE_SECONDS, Tile};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn name(self) -> &'static str {
        match self {
            Self::System => "Match System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub fn preference(self) -> ThemePreference {
        match self {
            Self::System => ThemePreference::System,
            Self::Light => ThemePreference::Light,
            Self::Dark => ThemePreference::Dark,
        }
    }
}

/// How tiles are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TileStyle {
    /// Flat cards with a coloured outline.
    #[default]
    Flat,
    /// Raised, softly lit buttons that press in while they play.
    Soft,
}

impl TileStyle {
    pub const ALL: [Self; 2] = [Self::Flat, Self::Soft];

    pub fn name(self) -> &'static str {
        match self {
            Self::Flat => "Flat Tiles",
            Self::Soft => "Soft Tiles",
        }
    }
}

/// How big tiles are drawn: smaller tiles fit more on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TileSize {
    /// Title, key and time only: a full board on a laptop screen.
    Compact,
    #[default]
    Regular,
    /// Bigger tiles and text, for touch screens and reading from afar.
    Large,
}

impl TileSize {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Regular, Self::Large];

    pub fn name(self) -> &'static str {
        match self {
            Self::Compact => "Compact Tiles",
            Self::Regular => "Regular Tiles",
            Self::Large => "Large Tiles",
        }
    }
}

/// What one click of Polish gives a tile: a fade in, a fade out and some
/// reverb. Adjustable in Playback ▸ Polish Preset.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Polish {
    pub fade_in_seconds: f32,
    pub fade_out_seconds: f32,
    /// 0 (dry) to 1.
    pub reverb: f32,
}

impl Default for Polish {
    fn default() -> Self {
        Self {
            fade_in_seconds: 1.0,
            fade_out_seconds: 1.0,
            reverb: 0.3,
        }
    }
}

impl Polish {
    /// No fades, no reverb: what removing the polish sets.
    pub const OFF: Self = Self {
        fade_in_seconds: 0.0,
        fade_out_seconds: 0.0,
        reverb: 0.0,
    };

    /// True if the tile already has these values (within what a slider
    /// shows).
    pub fn applies_to(&self, tile: &Tile) -> bool {
        let close = |a: f32, b: f32| (a - b).abs() < 0.005;
        close(tile.fade_in_seconds, self.fade_in_seconds)
            && close(tile.fade_out_seconds, self.fade_out_seconds)
            && close(tile.reverb, self.reverb)
    }

    /// The three tile settings, to apply through `Command::SetTile`.
    pub fn settings(self) -> [TileSetting; 3] {
        [
            TileSetting::FadeIn(self.fade_in_seconds),
            TileSetting::FadeOut(self.fade_out_seconds),
            TileSetting::Reverb(self.reverb),
        ]
    }

    /// "1 s in · 1 s out · 30 % reverb", for tooltips and buttons.
    pub fn describe(&self) -> String {
        format!(
            "{} in · {} out · {:.0} % reverb",
            seconds(self.fade_in_seconds),
            seconds(self.fade_out_seconds),
            self.reverb * 100.0
        )
    }

    fn clamped(self) -> Self {
        let fade = |value: f32| {
            if value.is_finite() {
                value.clamp(0.0, MAX_FADE_SECONDS)
            } else {
                0.0
            }
        };
        Self {
            fade_in_seconds: fade(self.fade_in_seconds),
            fade_out_seconds: fade(self.fade_out_seconds),
            reverb: if self.reverb.is_finite() {
                self.reverb.clamp(0.0, 1.0)
            } else {
                0.0
            },
        }
    }
}

/// "1 s", "0.5 s", "2.25 s".
fn seconds(value: f32) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text} s")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: Theme,
    pub tile_style: TileStyle,
    pub tile_size: TileSize,
    /// Draw a spectrogram under each tile's waveform.
    pub show_spectrograms: bool,
    pub polish: Polish,
    /// Master volume, 0 to 1.
    pub volume: f32,
    pub muted: bool,
    pub show_sets: bool,
    pub show_inspector: bool,
    /// A single click plays a tile. Otherwise a click selects it and a
    /// double-click plays it, as in Farrago.
    pub click_plays: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            tile_style: TileStyle::Flat,
            tile_size: TileSize::Regular,
            show_spectrograms: true,
            polish: Polish::default(),
            volume: 1.0,
            muted: false,
            show_sets: true,
            show_inspector: true,
            click_plays: false,
        }
    }
}

const KEY: &str = "preferences";

impl Preferences {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let mut prefs: Self = storage
            .and_then(|storage| eframe::get_value(storage, KEY))
            .unwrap_or_default();
        prefs.volume = if prefs.volume.is_finite() {
            prefs.volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
        prefs.polish = prefs.polish.clamped();
        prefs
    }

    pub fn store(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, KEY, self);
    }

    /// The gain the master volume sets on the mixer. A square law, so the
    /// slider sounds even along its length.
    pub fn master_gain(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.volume * self.volume
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gain_is_quiet_in_the_middle_and_silent_when_muted() {
        let mut prefs = Preferences::default();
        assert_eq!(prefs.master_gain(), 1.0);
        prefs.volume = 0.5;
        assert_eq!(prefs.master_gain(), 0.25);
        prefs.muted = true;
        assert_eq!(prefs.master_gain(), 0.0);
    }

    #[test]
    fn polish_knows_whether_a_tile_has_it() {
        use olio_library::{Slot, Sound};
        let polish = Polish::default();
        assert_eq!(polish.describe(), "1 s in · 1 s out · 30 % reverb");
        let sound = Sound {
            hash: "abc".to_owned(),
            extension: "wav".to_owned(),
            original_name: "a.wav".to_owned(),
        };
        let mut tile = Tile::new(sound, Slot::new(0, 0));
        assert!(!polish.applies_to(&tile));
        tile.fade_in_seconds = 1.0;
        tile.fade_out_seconds = 1.0;
        tile.reverb = 0.3;
        assert!(polish.applies_to(&tile));
        tile.reverb = 0.35;
        assert!(!polish.applies_to(&tile));
        assert_eq!(Polish::OFF.settings()[2], TileSetting::Reverb(0.0));
        assert_eq!(seconds(0.5), "0.5 s");
        assert_eq!(seconds(2.25), "2.25 s");
    }

    #[test]
    fn old_preferences_fill_in_new_fields() {
        let prefs: Preferences = serde_json::from_str(r#"{"theme":"Dark"}"#).unwrap();
        assert_eq!(prefs.theme, Theme::Dark);
        assert_eq!(prefs.volume, 1.0);
        assert!(prefs.show_sets);
        assert_eq!(prefs.polish, Polish::default());
        assert_eq!(prefs.tile_style, TileStyle::Flat);
        assert!(prefs.show_spectrograms);
        assert_eq!(prefs.tile_size, TileSize::Regular);
    }
}

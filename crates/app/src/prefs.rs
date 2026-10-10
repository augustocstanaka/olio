//! Window preferences: theme, master volume, which panels are shown.
//!
//! They are not part of the library format. eframe keeps them, with the
//! window's size and the panel widths, in `preferences.ron` beside the
//! library, so a scratch library opened with `--library` has its own.

use egui::ThemePreference;
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: Theme,
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
    fn old_preferences_fill_in_new_fields() {
        let prefs: Preferences = serde_json::from_str(r#"{"theme":"Dark"}"#).unwrap();
        assert_eq!(prefs.theme, Theme::Dark);
        assert_eq!(prefs.volume, 1.0);
        assert!(prefs.show_sets);
    }
}

//! Window preferences: theme, master volume, which panels are shown, the
//! Generator's settings and the styles saved from them.
//!
//! They are not part of the library format. eframe keeps them, with the
//! window's size and the panel widths, in `preferences.ron` beside the
//! library, so a scratch library opened with `--library` has its own.

use egui::ThemePreference;
use olio_audio::AmbienceSettings;
use olio_audio::ambience::Style;
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

    /// The next size, round from Large back to Compact.
    pub fn next(self) -> Self {
        match self {
            Self::Compact => Self::Regular,
            Self::Regular => Self::Large,
            Self::Large => Self::Compact,
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

/// A style of one's own: Generator settings saved under a name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserStyle {
    pub name: String,
    /// Off, with no built-in style: picking it keeps whether the ambience
    /// plays.
    pub settings: AmbienceSettings,
}

impl UserStyle {
    pub fn new(name: String, settings: AmbienceSettings) -> Self {
        Self {
            name,
            settings: AmbienceSettings {
                enabled: false,
                style: None,
                ..settings
            },
        }
    }
}

/// Names compare ignoring case and the spaces around them.
fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Why `name` cannot name a saved style (the one at `of`, or a new one
/// when `None`), if it cannot. A new style may take a saved one's name:
/// it replaces it.
pub fn style_name_problem(
    styles: &[UserStyle],
    name: &str,
    of: Option<usize>,
) -> Option<&'static str> {
    if name.trim().is_empty() {
        return Some("Give it a name");
    }
    if Style::ALL.iter().any(|style| same_name(style.name(), name)) {
        return Some("That is a built-in style");
    }
    let taken = styles
        .iter()
        .enumerate()
        .any(|(index, style)| Some(index) != of && same_name(&style.name, name));
    (of.is_some() && taken).then_some("Another style has that name")
}

/// The saved style called `name`, ignoring case.
pub fn style_named(styles: &[UserStyle], name: &str) -> Option<usize> {
    styles.iter().position(|style| same_name(&style.name, name))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub theme: Theme,
    pub tile_style: TileStyle,
    pub tile_size: TileSize,
    /// The generated bed behind the sounds, and the Generator's settings.
    pub ambience: AmbienceSettings,
    /// How long a generated track is, in seconds.
    pub track_seconds: u32,
    /// Draw a spectrogram under each tile's waveform.
    pub show_spectrograms: bool,
    pub polish: Polish,
    /// Master volume, 0 to 1.
    pub volume: f32,
    pub muted: bool,
    pub show_sets: bool,
    pub show_inspector: bool,
    /// The Generator in place of the inspector.
    pub show_generator: bool,
    /// The Generator shows every setting, not only the essentials.
    pub generator_details: bool,
    /// Styles of one's own, listed after the built-in ones.
    pub styles: Vec<UserStyle>,
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
            ambience: AmbienceSettings::default(),
            track_seconds: crate::generator::DEFAULT_TRACK_SECONDS,
            show_spectrograms: true,
            polish: Polish::default(),
            volume: 1.0,
            muted: false,
            show_sets: true,
            show_inspector: true,
            show_generator: false,
            generator_details: false,
            styles: Vec::new(),
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
        prefs.ambience = prefs.ambience.clamped();
        prefs.styles.retain(|style| !style.name.trim().is_empty());
        for style in &mut prefs.styles {
            style.name = style.name.trim().to_owned();
            style.settings = UserStyle::new(String::new(), style.settings.clamped()).settings;
        }
        if !crate::generator::TRACK_LENGTHS
            .iter()
            .any(|(seconds, _)| *seconds == prefs.track_seconds)
        {
            prefs.track_seconds = crate::generator::DEFAULT_TRACK_SECONDS;
        }
        prefs
    }

    pub fn store(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, KEY, self);
    }

    /// The saved style the Generator's settings are, if they are one and
    /// not a built-in style.
    pub fn saved_style(&self) -> Option<usize> {
        if self.ambience.style.is_some() {
            return None;
        }
        self.styles
            .iter()
            .position(|style| style.settings.same_music(&self.ambience))
    }

    /// What the Generator plays: "Rain Study", a saved style's name, or
    /// "Custom: D Dorian".
    pub fn style_name(&self) -> String {
        match self.saved_style() {
            Some(index) => self.styles[index].name.clone(),
            None => self.ambience.describe(),
        }
    }

    /// What a track of it is called, before its length: the style, saved
    /// or built-in, or the key.
    pub fn track_name(&self) -> String {
        match (self.ambience.style, self.saved_style()) {
            (Some(style), _) => style.name().to_owned(),
            (None, Some(index)) => self.styles[index].name.clone(),
            (None, None) => self.ambience.scale.name(),
        }
    }

    /// A name for saving the Generator's settings: their key, numbered
    /// while it is taken.
    pub fn suggested_style_name(&self) -> String {
        let base = self.ambience.scale.name();
        (1..)
            .map(|n| match n {
                1 => base.clone(),
                n => format!("{base} {n}"),
            })
            .find(|name| style_named(&self.styles, name).is_none())
            .unwrap_or(base)
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

    /// Preferences as one saved them before the Generator.
    const SAVED_BEFORE_THE_GENERATOR: &str = "(theme:System,tile_style:Flat,tile_size:Regular,\
        ambience:(enabled:true,style:Some(RainStudy),scale:(root:2,kind:Mode(Dorian)),\
        weather:Winter,layers:((enabled:true,level:0.8,density:Sparse,register:Middle),\
        (enabled:true,level:0.7,density:Occasional,register:Middle),\
        (enabled:false,level:0.8,density:Sparse,register:Middle),\
        (enabled:false,level:0.8,density:Sparse,register:Middle),\
        (enabled:false,level:0.8,density:Sparse,register:Middle),\
        (enabled:false,level:0.8,density:Sparse,register:Middle)),presence:Ambient,\
        tempo:60,form:Endless,level:0.35,duck:true,seed:1),show_spectrograms:true,\
        polish:(fade_in_seconds:1.0,fade_out_seconds:1.0,reverb:0.3),volume:0.3,muted:false,\
        show_sets:true,show_inspector:true,click_plays:false)";

    struct Saved(&'static str);

    impl eframe::Storage for Saved {
        fn get_string(&self, key: &str) -> Option<String> {
            (key == KEY).then(|| self.0.to_owned())
        }

        fn set_string(&mut self, _key: &str, _value: String) {}

        fn remove_string(&mut self, _key: &str) {}

        fn flush(&mut self) {}
    }

    #[test]
    fn preferences_saved_before_the_generator_load_with_their_ambience() {
        use olio_audio::ambience::{Background, Instrument};
        let prefs = Preferences::load(Some(&Saved(SAVED_BEFORE_THE_GENERATOR)));
        assert_eq!(prefs.volume, 0.3, "the rest of the preferences survive");
        assert!(prefs.ambience.enabled);
        assert_eq!(prefs.ambience.background, Background::Winter);
        assert!(prefs.ambience.layer(Instrument::Harp).enabled);
        assert!(!prefs.ambience.layer(Instrument::Piano).enabled);
        assert!(!prefs.show_generator);
        assert!(!prefs.generator_details, "the essentials first");
        assert!(prefs.styles.is_empty());
        assert_eq!(prefs.track_seconds, 120);
    }

    #[test]
    fn saved_styles_keep_their_name_and_music_and_are_found_again() {
        use olio_audio::ambience::Instrument;
        let mut prefs = Preferences::default();
        prefs.ambience.enabled = true;
        prefs.ambience.layer_mut(Instrument::Piano).enabled = true;
        assert_eq!(prefs.style_name(), "Custom: D Dorian");
        assert_eq!(prefs.track_name(), "D Dorian");
        assert_eq!(prefs.suggested_style_name(), "D Dorian");
        prefs
            .styles
            .push(UserStyle::new("Rainy Desk".to_owned(), prefs.ambience));
        assert!(!prefs.styles[0].settings.enabled);

        // Stored and loaded again, it is still what the Generator plays.
        #[derive(Default)]
        struct Kept(String);
        impl eframe::Storage for Kept {
            fn get_string(&self, key: &str) -> Option<String> {
                (key == KEY).then(|| self.0.clone())
            }
            fn set_string(&mut self, key: &str, value: String) {
                if key == KEY {
                    self.0 = value;
                }
            }
            fn remove_string(&mut self, _key: &str) {}
            fn flush(&mut self) {}
        }
        let mut kept = Kept::default();
        prefs.store(&mut kept);
        let mut prefs = Preferences::load(Some(&kept));
        assert_eq!(prefs.saved_style(), Some(0));
        assert_eq!(prefs.style_name(), "Rainy Desk");
        assert_eq!(prefs.track_name(), "Rainy Desk");
        prefs.ambience.seed += 1;
        assert_eq!(prefs.saved_style(), Some(0), "a new variation is still it");
        prefs.ambience.chaos = 0.5;
        assert_eq!(prefs.saved_style(), None);

        prefs.styles[0].name = "D Dorian".to_owned();
        assert_eq!(prefs.suggested_style_name(), "D Dorian 2");
        let styles = &prefs.styles;
        assert_eq!(
            style_name_problem(styles, "  ", None),
            Some("Give it a name")
        );
        assert_eq!(
            style_name_problem(styles, "rain study", None),
            Some("That is a built-in style")
        );
        assert_eq!(style_name_problem(styles, "d dorian", None), None);
        assert_eq!(style_name_problem(styles, "d dorian", Some(0)), None);
        prefs
            .styles
            .push(UserStyle::new("Other".to_owned(), prefs.ambience));
        assert_eq!(
            style_name_problem(&prefs.styles, "OTHER", Some(0)),
            Some("Another style has that name")
        );
        assert_eq!(style_named(&prefs.styles, " other "), Some(1));
    }

    #[test]
    fn tile_sizes_go_round() {
        let mut size = TileSize::Compact;
        let mut seen = Vec::new();
        for _ in 0..3 {
            size = size.next();
            seen.push(size);
        }
        assert_eq!(
            seen,
            [TileSize::Regular, TileSize::Large, TileSize::Compact]
        );
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
        assert!(!prefs.ambience.enabled);
    }
}

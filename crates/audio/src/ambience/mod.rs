//! Ambience: a quiet, endless, generated bed behind the sounds. Weather,
//! and up to six instruments (a plucked bass, a harp, bells, strings, a
//! pad and a drone) drawing on a scale you choose.
//!
//! The settings are built from independent parts, so each can grow on its
//! own: the [`Scale`], the [`Layer`]s (one per [`Instrument`]), the
//! [`Presence`] (how noticeable it is), the [`Form`] (endless, or pieces
//! with a shape), and [`Style`] presets that fill them all in.
//!
//! [`composer`] decides what plays when with no audio in it, so the musical
//! rules are tested on their own; [`synth`] and [`weather`] make the sound
//! on the audio thread, allocating nothing while rendering
//! (`CODING_STANDARDS.md`).

mod composer;
mod synth;
mod weather;

use serde::{Deserialize, Serialize};

use crate::mixer::Ramp;
pub use composer::{Composer, Event, Note};
use synth::Synth;
use weather::Bed;

// ----- scale -------------------------------------------------------------

/// The seven church modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    Ionian,
    #[default]
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Aeolian,
    Locrian,
}

impl Mode {
    pub const ALL: [Self; 7] = [
        Self::Ionian,
        Self::Dorian,
        Self::Phrygian,
        Self::Lydian,
        Self::Mixolydian,
        Self::Aeolian,
        Self::Locrian,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Ionian => "Ionian",
            Self::Dorian => "Dorian",
            Self::Phrygian => "Phrygian",
            Self::Lydian => "Lydian",
            Self::Mixolydian => "Mixolydian",
            Self::Aeolian => "Aeolian",
            Self::Locrian => "Locrian",
        }
    }

    /// How it sounds, in a few words.
    pub fn character(self) -> &'static str {
        match self {
            Self::Ionian => "the major scale: bright, settled",
            Self::Dorian => "minor with a lifted sixth: calm, a little hopeful",
            Self::Phrygian => "minor with a lowered second: dark, Spanish",
            Self::Lydian => "major with a raised fourth: floating, dreamlike",
            Self::Mixolydian => "major with a lowered seventh: warm, folk-like",
            Self::Aeolian => "the natural minor: sad, still",
            Self::Locrian => "lowered second and fifth: unsettled, strange",
        }
    }

    fn intervals(self) -> &'static [u8] {
        match self {
            Self::Ionian => &[0, 2, 4, 5, 7, 9, 11],
            Self::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Self::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Self::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            Self::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Self::Aeolian => &[0, 2, 3, 5, 7, 8, 10],
            Self::Locrian => &[0, 1, 3, 5, 6, 8, 10],
        }
    }
}

/// Which notes are in the music. Modes have seven; others fewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ScaleKind {
    Mode(Mode),
    MajorPentatonic,
    MinorPentatonic,
    HarmonicMinor,
    WholeTone,
}

impl Default for ScaleKind {
    fn default() -> Self {
        Self::Mode(Mode::Dorian)
    }
}

impl ScaleKind {
    /// The modes first, then the others.
    pub fn all() -> [Self; 11] {
        [
            Self::Mode(Mode::Ionian),
            Self::Mode(Mode::Dorian),
            Self::Mode(Mode::Phrygian),
            Self::Mode(Mode::Lydian),
            Self::Mode(Mode::Mixolydian),
            Self::Mode(Mode::Aeolian),
            Self::Mode(Mode::Locrian),
            Self::MajorPentatonic,
            Self::MinorPentatonic,
            Self::HarmonicMinor,
            Self::WholeTone,
        ]
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Mode(mode) => mode.name(),
            Self::MajorPentatonic => "Major pentatonic",
            Self::MinorPentatonic => "Minor pentatonic",
            Self::HarmonicMinor => "Harmonic minor",
            Self::WholeTone => "Whole tone",
        }
    }

    pub fn character(self) -> &'static str {
        match self {
            Self::Mode(mode) => mode.character(),
            Self::MajorPentatonic => "five notes, no clashes: open, folk, childlike",
            Self::MinorPentatonic => "five notes, minor: bluesy, weighty",
            Self::HarmonicMinor => "minor with a raised seventh: tense, old-world",
            Self::WholeTone => "six even steps: weightless, nowhere to land",
        }
    }

    /// Semitones above the root of each degree, rising, all under 12.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Self::Mode(mode) => mode.intervals(),
            Self::MajorPentatonic => &[0, 2, 4, 7, 9],
            Self::MinorPentatonic => &[0, 3, 5, 7, 10],
            Self::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Self::WholeTone => &[0, 2, 4, 6, 8, 10],
        }
    }
}

/// Names of the twelve roots, from C.
pub const ROOT_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// A root and a kind: everything melodic asks this which notes exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Scale {
    /// Pitch class, 0 (C) to 11 (B).
    pub root: u8,
    pub kind: ScaleKind,
}

impl Scale {
    /// How many degrees before the octave.
    pub fn degrees(&self) -> i32 {
        self.kind.intervals().len() as i32
    }

    /// The MIDI note of a degree above `base` (a MIDI note on the root):
    /// `len()` is the root an octave up, -1 the top degree below.
    pub fn note(&self, base: u8, degree: i32) -> u8 {
        let intervals = self.kind.intervals();
        let len = intervals.len() as i32;
        let octave = degree.div_euclid(len);
        let step = degree.rem_euclid(len) as usize;
        let semis = i32::from(base) + octave * 12 + i32::from(intervals[step]);
        semis.clamp(0, 127) as u8
    }

    /// True if the note's pitch class is in the scale.
    pub fn contains(&self, midi: u8) -> bool {
        let pitch_class = (i32::from(midi) - i32::from(self.root)).rem_euclid(12) as u8;
        self.kind.intervals().contains(&pitch_class)
    }

    /// The root as a MIDI note in octave 2 (C2 is 36).
    pub fn base(&self) -> u8 {
        36 + self.root % 12
    }

    /// "D Dorian".
    pub fn name(&self) -> String {
        format!(
            "{} {}",
            ROOT_NAMES[usize::from(self.root % 12)],
            self.kind.name()
        )
    }
}

// ----- weather -----------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Weather {
    None,
    #[default]
    Rain,
    Wind,
    /// A thin, cold wind with a whistle in it.
    Winter,
}

impl Weather {
    pub const ALL: [Self; 4] = [Self::None, Self::Rain, Self::Wind, Self::Winter];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Rain => "Rain",
            Self::Wind => "Wind",
            Self::Winter => "Winter",
        }
    }
}

// ----- layers ------------------------------------------------------------

/// The instruments, each a layer of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Instrument {
    /// A plucked string, low: the root and fifth, now and then.
    Bass,
    /// Plucked, high: wandering notes and short rising phrases.
    Harp,
    /// Struck, bright and inharmonic: very sparse, high.
    Bells,
    /// Bowed and sustained, one note at a time, in the middle.
    Strings,
    /// Soft, slow chords that change with the harmony.
    Pad,
    /// The root, low and continuous, breathing slowly.
    Drone,
}

impl Instrument {
    pub const ALL: [Self; 6] = [
        Self::Bass,
        Self::Harp,
        Self::Bells,
        Self::Strings,
        Self::Pad,
        Self::Drone,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Bass => "Bass",
            Self::Harp => "Harp",
            Self::Bells => "Bells",
            Self::Strings => "Strings",
            Self::Pad => "Pad",
            Self::Drone => "Drone",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Bass => "a plucked string, low, the root and fifth now and then",
            Self::Harp => "plucked notes that wander, with short rising phrases",
            Self::Bells => "bright struck notes, very sparse and high",
            Self::Strings => "one bowed note at a time, sustained",
            Self::Pad => "soft slow chords that follow the harmony",
            Self::Drone => "the root, low and continuous",
        }
    }

    /// Its place in [`AmbienceSettings::layers`].
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|i| *i == self).unwrap_or(0)
    }

    /// Typical seconds between its notes at normal density.
    fn spacing(self) -> f32 {
        match self {
            Self::Bass => 9.0,
            Self::Harp => 3.5,
            Self::Bells => 14.0,
            Self::Strings => 12.0,
            Self::Pad => 18.0,
            Self::Drone => 0.0,
        }
    }

    /// Semitones above the scale's base its lowest note sits, at the
    /// middle register.
    fn register(self) -> i32 {
        match self {
            Self::Bass => 0,
            Self::Harp => 24,
            Self::Bells => 36,
            Self::Strings => 12,
            Self::Pad => 12,
            Self::Drone => -12,
        }
    }
}

/// How often a layer plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Density {
    #[default]
    Sparse,
    Occasional,
    Lively,
}

impl Density {
    pub const ALL: [Self; 3] = [Self::Sparse, Self::Occasional, Self::Lively];

    pub fn name(self) -> &'static str {
        match self {
            Self::Sparse => "Sparse",
            Self::Occasional => "Occasional",
            Self::Lively => "Lively",
        }
    }

    /// Multiplies the instrument's spacing.
    fn factor(self) -> f32 {
        match self {
            Self::Sparse => 1.6,
            Self::Occasional => 1.0,
            Self::Lively => 0.5,
        }
    }
}

/// Where a layer sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Register {
    Low,
    #[default]
    Middle,
    High,
}

impl Register {
    pub const ALL: [Self; 3] = [Self::Low, Self::Middle, Self::High];

    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Middle => "Middle",
            Self::High => "High",
        }
    }

    fn semitones(self) -> i32 {
        match self {
            Self::Low => -12,
            Self::Middle => 0,
            Self::High => 12,
        }
    }
}

/// One instrument's part.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layer {
    pub enabled: bool,
    /// 0 to 1, within the whole bed's level.
    pub level: f32,
    pub density: Density,
    pub register: Register,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            enabled: false,
            level: 0.8,
            density: Density::Sparse,
            register: Register::Middle,
        }
    }
}

impl Layer {
    pub const fn on(level: f32, density: Density) -> Self {
        Self {
            enabled: true,
            level,
            density,
            register: Register::Middle,
        }
    }
}

// ----- presence and form ---------------------------------------------------

/// How noticeable the bed is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Presence {
    /// Barely there: very quiet, very sparse, no pulse.
    Background,
    #[default]
    Ambient,
    /// Music to notice: louder, denser, on a slow pulse, with cadences.
    Declarative,
}

impl Presence {
    pub const ALL: [Self; 3] = [Self::Background, Self::Ambient, Self::Declarative];

    pub fn name(self) -> &'static str {
        match self {
            Self::Background => "Background",
            Self::Ambient => "Ambient",
            Self::Declarative => "Declarative",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Background => "barely there: very quiet and sparse, no pulse",
            Self::Ambient => "a quiet bed with a little movement",
            Self::Declarative => "music to notice: louder, denser, on a slow pulse, with cadences",
        }
    }

    /// Level and spacing multipliers.
    fn level(self) -> f32 {
        match self {
            Self::Background => 0.45,
            Self::Ambient => 1.0,
            Self::Declarative => 1.6,
        }
    }

    fn spacing(self) -> f32 {
        match self {
            Self::Background => 1.9,
            Self::Ambient => 1.0,
            Self::Declarative => 0.6,
        }
    }

    pub fn pulsed(self) -> bool {
        self == Self::Declarative
    }
}

/// Endless, or pieces with a beginning and an end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Form {
    #[default]
    Endless,
    /// A piece this long, a pause with weather only, then another.
    Piece { minutes: f32, pause_seconds: f32 },
}

impl Form {
    pub fn name(self) -> &'static str {
        match self {
            Self::Endless => "Endless",
            Self::Piece { .. } => "Pieces",
        }
    }
}

// ----- the settings and styles ---------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AmbienceSettings {
    pub enabled: bool,
    /// The preset these came from, if untouched since.
    pub style: Option<Style>,
    pub scale: Scale,
    pub weather: Weather,
    /// One per [`Instrument`], in [`Instrument::ALL`]'s order.
    pub layers: [Layer; 6],
    pub presence: Presence,
    /// Beats per minute of the pulse, when there is one.
    pub tempo: u16,
    pub form: Form,
    /// The whole bed, 0 to 1; even 1 is well under the sounds.
    pub level: f32,
    /// Quieter while any sound plays.
    pub duck: bool,
    /// Makes the bed repeatable; change it for a new variation.
    pub seed: u64,
}

impl Default for AmbienceSettings {
    fn default() -> Self {
        Style::RainStudy.settings()
    }
}

impl AmbienceSettings {
    /// The settings with every value in range.
    pub fn clamped(mut self) -> Self {
        self.scale.root %= 12;
        let unit = |value: f32, fallback: f32| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                fallback
            }
        };
        self.level = unit(self.level, 0.35);
        for layer in &mut self.layers {
            layer.level = unit(layer.level, 0.8);
        }
        self.tempo = self.tempo.clamp(30, 120);
        if let Form::Piece {
            minutes,
            pause_seconds,
        } = &mut self.form
        {
            *minutes = if minutes.is_finite() {
                minutes.clamp(0.5, 30.0)
            } else {
                3.0
            };
            *pause_seconds = if pause_seconds.is_finite() {
                pause_seconds.clamp(0.0, 120.0)
            } else {
                20.0
            };
        }
        self
    }

    pub fn layer(&self, instrument: Instrument) -> &Layer {
        &self.layers[instrument.index()]
    }

    pub fn layer_mut(&mut self, instrument: Instrument) -> &mut Layer {
        self.style = None;
        &mut self.layers[instrument.index()]
    }

    /// Instruments switched on.
    pub fn playing(&self) -> impl Iterator<Item = Instrument> + '_ {
        Instrument::ALL
            .into_iter()
            .filter(|instrument| self.layer(*instrument).enabled)
    }

    /// Seconds between a layer's notes at these settings, typically.
    fn spacing(&self, instrument: Instrument) -> f32 {
        instrument.spacing() * self.layer(instrument).density.factor() * self.presence.spacing()
    }

    /// The MIDI note of a scale degree in a layer's register.
    fn note(&self, instrument: Instrument, degree: i32) -> u8 {
        let base = i32::from(self.scale.base())
            + instrument.register()
            + self.layer(instrument).register.semitones();
        self.scale.note(base.clamp(0, 115) as u8, degree)
    }

    /// "Rain Study" or "Custom: D Dorian".
    pub fn describe(&self) -> String {
        match self.style {
            Some(style) => style.name().to_owned(),
            None => format!("Custom: {}", self.scale.name()),
        }
    }
}

/// A named bundle of settings. Picking one fills everything in; touching
/// a control makes the settings Custom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Style {
    WinterHush,
    RainStudy,
    Cathedral,
    NightDrift,
    Affirmation,
    Pentatonic,
}

impl Style {
    pub const ALL: [Self; 6] = [
        Self::RainStudy,
        Self::WinterHush,
        Self::NightDrift,
        Self::Cathedral,
        Self::Pentatonic,
        Self::Affirmation,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::WinterHush => "Winter Hush",
            Self::RainStudy => "Rain Study",
            Self::Cathedral => "Cathedral",
            Self::NightDrift => "Night Drift",
            Self::Affirmation => "Affirmation",
            Self::Pentatonic => "Pentatonic Garden",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::WinterHush => "a cold wind, a bass note now and then, a harp far away",
            Self::RainStudy => "rain, harp and a bass string: the default",
            Self::Cathedral => "no weather; a drone, a pad and sparse bells in a minor key",
            Self::NightDrift => "wind, a drone and a slow pad in Lydian, barely there",
            Self::Affirmation => "strings, bass, pad and bells on a slow pulse, in pieces",
            Self::Pentatonic => "rain and harp on five notes: nothing can clash",
        }
    }

    pub fn settings(self) -> AmbienceSettings {
        use Instrument::*;
        let mut layers = [Layer::default(); 6];
        let mut set = |instrument: Instrument, layer: Layer| {
            layers[instrument.index()] = layer;
        };
        let (scale, weather, presence, form, level) = match self {
            Self::RainStudy => {
                set(Bass, Layer::on(0.8, Density::Sparse));
                set(Harp, Layer::on(0.7, Density::Occasional));
                (
                    Scale {
                        root: 2,
                        kind: ScaleKind::Mode(Mode::Dorian),
                    },
                    Weather::Rain,
                    Presence::Ambient,
                    Form::Endless,
                    0.35,
                )
            }
            Self::WinterHush => {
                set(Bass, Layer::on(0.7, Density::Sparse));
                set(Harp, Layer::on(0.5, Density::Sparse));
                (
                    Scale {
                        root: 9,
                        kind: ScaleKind::Mode(Mode::Aeolian),
                    },
                    Weather::Winter,
                    Presence::Background,
                    Form::Endless,
                    0.3,
                )
            }
            Self::NightDrift => {
                set(Drone, Layer::on(0.6, Density::Sparse));
                set(Pad, Layer::on(0.6, Density::Sparse));
                (
                    Scale {
                        root: 5,
                        kind: ScaleKind::Mode(Mode::Lydian),
                    },
                    Weather::Wind,
                    Presence::Background,
                    Form::Endless,
                    0.3,
                )
            }
            Self::Cathedral => {
                set(Drone, Layer::on(0.7, Density::Sparse));
                set(Pad, Layer::on(0.7, Density::Occasional));
                set(Bells, Layer::on(0.5, Density::Sparse));
                (
                    Scale {
                        root: 7,
                        kind: ScaleKind::Mode(Mode::Aeolian),
                    },
                    Weather::None,
                    Presence::Ambient,
                    Form::Endless,
                    0.4,
                )
            }
            Self::Pentatonic => {
                set(Harp, Layer::on(0.8, Density::Occasional));
                set(Bells, Layer::on(0.4, Density::Sparse));
                set(Bass, Layer::on(0.6, Density::Sparse));
                (
                    Scale {
                        root: 7,
                        kind: ScaleKind::MajorPentatonic,
                    },
                    Weather::Rain,
                    Presence::Ambient,
                    Form::Endless,
                    0.35,
                )
            }
            Self::Affirmation => {
                set(Strings, Layer::on(0.8, Density::Occasional));
                set(Bass, Layer::on(0.8, Density::Occasional));
                set(Pad, Layer::on(0.6, Density::Occasional));
                set(Bells, Layer::on(0.5, Density::Sparse));
                (
                    Scale {
                        root: 0,
                        kind: ScaleKind::Mode(Mode::Mixolydian),
                    },
                    Weather::None,
                    Presence::Declarative,
                    Form::Piece {
                        minutes: 3.0,
                        pause_seconds: 20.0,
                    },
                    0.5,
                )
            }
        };
        AmbienceSettings {
            enabled: false,
            style: Some(self),
            scale,
            weather,
            layers,
            presence,
            tempo: 60,
            form,
            level,
            duck: true,
            seed: 1,
        }
    }
}

// ----- random ----------------------------------------------------------

/// A small, fast random source (xorshift64*). Seeded, so a render can be
/// repeated exactly.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// 0 to 1, with 24 bits of variety.
    #[inline]
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// -1 to 1.
    #[inline]
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }

    pub fn chance(&mut self, probability: f32) -> bool {
        self.unit() < probability
    }

    pub fn below(&mut self, n: u32) -> u32 {
        (self.next_u64() % u64::from(n.max(1))) as u32
    }
}

// ----- the generator ----------------------------------------------------

/// Even at full level the bed sits well under the sounds.
const FULL_LEVEL_GAIN: f32 = 0.16;
/// How much quieter while sounds play.
const DUCKED: f32 = 0.35;
/// How much of the instruments goes to the reverb.
const REVERB_SEND: f32 = 0.4;

/// The ambience, synthesised. Built once with everything it needs.
pub struct Ambience {
    rate: u32,
    settings: AmbienceSettings,
    composer: Composer,
    synth: Synth,
    bed: Bed,
    /// Frames rendered so far.
    clock: u64,
    /// On and off, slowly.
    gain: Ramp,
    duck: Ramp,
}

impl Ambience {
    pub fn new(rate: u32, seed: u64) -> Self {
        let settings = AmbienceSettings {
            enabled: false,
            ..AmbienceSettings::default()
        };
        Self {
            rate,
            settings,
            composer: Composer::new(rate, seed),
            synth: Synth::new(rate, seed ^ 0x9E37_79B9_7F4A_7C15),
            bed: Bed::new(rate, seed.rotate_left(17)),
            clock: 0,
            gain: Ramp::at(0.0),
            duck: Ramp::at(1.0),
        }
    }

    pub fn settings(&self) -> &AmbienceSettings {
        &self.settings
    }

    /// Applies new settings: on and off fade over a second; the weather
    /// crosses over; the key, layers and form take effect from the next
    /// event, and a new seed starts the music afresh.
    pub fn set(&mut self, settings: AmbienceSettings) {
        let settings = settings.clamped();
        let was_on = self.settings.enabled;
        if settings.enabled != was_on {
            let target = if settings.enabled { 1.0 } else { 0.0 };
            self.gain.towards(target, self.rate);
            if settings.enabled {
                self.composer.reset(settings.seed);
            }
        } else if settings.seed != self.settings.seed || settings.form != self.settings.form {
            self.composer.reset(settings.seed);
        }
        self.bed.set_weather(if settings.enabled {
            settings.weather
        } else {
            self.settings.weather
        });
        self.settings = settings;
    }

    /// True while anything may be heard.
    pub fn is_audible(&self) -> bool {
        self.settings.enabled || !self.gain.is_silent() || self.synth.is_sounding()
    }

    /// Adds the next `out.len() / channels` frames into `out` (dry) and
    /// the instruments' share into the stereo `bus`. `ducking` while
    /// sounds play.
    pub fn render(&mut self, out: &mut [f32], bus: &mut [f32], channels: usize, ducking: bool) {
        let frames = out.len() / channels;
        if !self.is_audible() {
            self.clock += frames as u64;
            return;
        }
        let level = self.settings.level
            * self.settings.level
            * FULL_LEVEL_GAIN
            * self.settings.presence.level();
        let duck_target = if ducking && self.settings.duck {
            DUCKED
        } else {
            1.0
        };
        if self.duck.target() != duck_target {
            self.duck.towards(duck_target, self.rate / 3);
        }
        let (from, to) = (self.clock, self.clock + frames as u64);
        if self.settings.enabled {
            self.composer.poll(&self.settings, from, to);
        }
        let music_gain = self.composer.music_gain();
        let (sends, _) = bus.as_chunks_mut::<2>();
        for (i, (frame, send)) in out.chunks_exact_mut(channels).zip(sends).enumerate() {
            let now = from + i as u64;
            while let Some(event) = self.composer.due(now) {
                self.synth.play(event, &self.settings);
            }
            let gain = self.gain.next() * self.duck.next() * level;
            let (mut left, mut right) = self.bed.next();
            let (sl, sr) = self.synth.next(music_gain);
            left = (left + sl) * gain;
            right = (right + sr) * gain;
            if channels == 1 {
                frame[0] += 0.5 * (left + right);
            } else {
                frame[0] += left;
                frame[1] += right;
            }
            send[0] += sl * gain * REVERB_SEND;
            send[1] += sr * gain * REVERB_SEND;
        }
        self.clock = to;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_step_through_their_intervals_and_wrap_at_the_octave() {
        let lydian = Scale {
            root: 0,
            kind: ScaleKind::Mode(Mode::Lydian),
        };
        assert_eq!(lydian.note(60, 3), 66, "the raised fourth");
        assert_eq!(lydian.note(60, 7), 72, "the root an octave up");
        assert_eq!(lydian.note(60, -1), 59, "the seventh below");
        let pentatonic = Scale {
            root: 7,
            kind: ScaleKind::MajorPentatonic,
        };
        assert_eq!(pentatonic.degrees(), 5);
        assert_eq!(pentatonic.note(55, 5), 67, "five degrees to the octave");
        assert!(pentatonic.contains(62) && !pentatonic.contains(60));
        for kind in ScaleKind::all() {
            let intervals = kind.intervals();
            assert!(intervals.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(intervals[0] == 0 && *intervals.last().unwrap() < 12);
        }
    }

    #[test]
    fn styles_fill_everything_in_and_a_touch_makes_them_custom() {
        for style in Style::ALL {
            let settings = style.settings();
            assert_eq!(settings.style, Some(style));
            assert!(settings.playing().count() >= 2, "{}", style.name());
            assert_eq!(settings.clamped(), settings, "within range");
        }
        let mut settings = Style::Cathedral.settings();
        assert_eq!(settings.describe(), "Cathedral");
        settings.layer_mut(Instrument::Harp).enabled = true;
        assert_eq!(settings.style, None);
        assert_eq!(settings.describe(), "Custom: G Aeolian");
    }

    #[test]
    fn the_same_seed_renders_the_same_sound() {
        let settings = AmbienceSettings {
            enabled: true,
            ..Style::Pentatonic.settings()
        };
        let render = || {
            let mut ambience = Ambience::new(48_000, 11);
            ambience.set(settings);
            let mut out = vec![0.0; 48_000 * 2];
            let mut bus = vec![0.0; 48_000 * 2];
            ambience.render(&mut out, &mut bus, 2, false);
            out
        };
        assert_eq!(render(), render());
    }

    #[test]
    fn every_instrument_alone_sounds_and_stays_under_the_ceiling() {
        for instrument in Instrument::ALL {
            let mut settings = Style::RainStudy.settings();
            settings.enabled = true;
            settings.weather = Weather::None;
            settings.level = 1.0;
            settings.presence = Presence::Declarative;
            for layer in &mut settings.layers {
                layer.enabled = false;
            }
            *settings.layer_mut(instrument) = Layer::on(1.0, Density::Lively);
            let mut ambience = Ambience::new(48_000, 3);
            ambience.set(settings);
            let mut out = vec![0.0; 48_000 * 2 * 40];
            let mut bus = vec![0.0; 48_000 * 2 * 40];
            ambience.render(&mut out, &mut bus, 2, false);
            let peak = out.iter().fold(0.0f32, |p, s| p.max(s.abs()));
            assert!(peak > 0.002, "{} is silent", instrument.name());
            assert!(peak < 0.6, "{} peaks at {peak}", instrument.name());
            assert!(out.iter().all(|s| s.is_finite()));
        }
    }
}

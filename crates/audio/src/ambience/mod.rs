//! Ambience: a quiet, endless, generated bed behind the sounds. A
//! [`Background`] (rain, wind, waves, fire, thunder, a forest, a desert…)
//! and up to nine instruments (a plucked bass, a harp, bells, strings, a
//! pad, a drone, a piano, a plucked violin and soft hand drums) drawing on
//! a scale you choose. The Generator in the app plays it live and renders
//! it to tracks ([`render_track`]).
//!
//! The settings are built from independent parts, so each can grow on its
//! own: the [`Scale`], the [`Layer`]s (one per [`Instrument`]), the
//! [`Presence`] (how noticeable it is), the [`Form`] (endless, or pieces
//! with a shape), the character (variety, concentration and chaos), the
//! [`Journey`] (steady, building up, or evolving over time), and
//! [`Style`] presets that fill them all in.
//!
//! [`composer`] decides what plays when with no audio in it, so the musical
//! rules are tested on their own; [`synth`] and [`background`] make the
//! sound on the audio thread, allocating nothing while rendering
//! (`CODING_STANDARDS.md`).

mod background;
mod composer;
mod dsp;
mod hall;
mod journey;
mod sampler;
mod synth;
mod track;

use std::fmt;
use std::ops::{Index, IndexMut};

use serde::de::{IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::mixer::Ramp;
use background::Bed;
pub use composer::{Composer, Event, Note, SHAKER};
use hall::{Chorus, Hall};
use journey::Course;
pub use journey::{Journey, Now};
use synth::Synth;
pub use track::{TRACK_RATE, render_track};

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

// ----- background ----------------------------------------------------------

/// The sound under the music: weather or a place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Background {
    None,
    #[default]
    Rain,
    /// Distant rolls of thunder over soft rain.
    Thunder,
    Wind,
    /// A thin, cold wind with a whistle in it.
    Winter,
    /// Waves breaking on a shore, slowly.
    Waves,
    /// Leaves in a breeze and a bird now and then.
    Forest,
    /// A dry wind that lifts sand in its gusts and sometimes sings.
    Desert,
    /// A fire's low roar and its crackles.
    Fire,
}

impl Background {
    pub const ALL: [Self; 9] = [
        Self::None,
        Self::Rain,
        Self::Thunder,
        Self::Wind,
        Self::Winter,
        Self::Waves,
        Self::Forest,
        Self::Desert,
        Self::Fire,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Rain => "Rain",
            Self::Thunder => "Thunder",
            Self::Wind => "Wind",
            Self::Winter => "Winter",
            Self::Waves => "Waves",
            Self::Forest => "Forest",
            Self::Desert => "Desert",
            Self::Fire => "Fire",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::None => "the music alone",
            Self::Rain => "a soft hiss of rain with drops in it",
            Self::Thunder => "soft rain and distant rolls of thunder, far apart",
            Self::Wind => "a low wind that gusts",
            Self::Winter => "a thin, cold wind with a whistle in it",
            Self::Waves => "waves breaking on a shore, slowly",
            Self::Forest => "leaves in a breeze and a bird now and then",
            Self::Desert => "a dry wind that lifts sand and sometimes sings",
            Self::Fire => "a fire's low roar and its crackles",
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
    /// Struck, in the middle: sparse notes, sometimes two together.
    Piano,
    /// Plucked (pizzicato), high: short, bright notes.
    Violin,
    /// Soft hand percussion: a low frame drum and a shaker.
    Drums,
}

impl Instrument {
    /// How many there are.
    pub const COUNT: usize = 9;

    /// In the order of [`AmbienceSettings::layers`]; new instruments are
    /// added at the end so saved settings keep their meaning.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Bass,
        Self::Harp,
        Self::Bells,
        Self::Strings,
        Self::Pad,
        Self::Drone,
        Self::Piano,
        Self::Violin,
        Self::Drums,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Bass => "Bass",
            Self::Harp => "Harp",
            Self::Bells => "Bells",
            Self::Strings => "Strings",
            Self::Pad => "Pad",
            Self::Drone => "Drone",
            Self::Piano => "Piano",
            Self::Violin => "Violin",
            Self::Drums => "Drums",
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
            Self::Piano => "sparse struck notes in the middle, sometimes two together",
            Self::Violin => "plucked (pizzicato): short, bright notes, now and then an answer",
            Self::Drums => "soft hand percussion: a low frame drum and a shaker, very sparse",
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
            Self::Harp => 6.5,
            Self::Bells => 14.0,
            Self::Strings => 12.0,
            Self::Pad => 18.0,
            Self::Drone => 0.0,
            Self::Piano => 8.0,
            Self::Violin => 9.0,
            Self::Drums => 11.0,
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
            Self::Piano => 12,
            Self::Violin => 24,
            // The frame drum's pitch: the root, low.
            Self::Drums => 0,
        }
    }
}

/// How many notes a layer plays: from 0, very sparse, to 1, busy. The
/// three steps it once had sit at [`Density::SPARSE`],
/// [`Density::OCCASIONAL`] and [`Density::LIVELY`], spaced as they were.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Density(f32);

impl Default for Density {
    fn default() -> Self {
        Self::SPARSE
    }
}

impl Density {
    pub const SPARSE: Self = Self(0.25);
    pub const OCCASIONAL: Self = Self(0.5);
    pub const LIVELY: Self = Self(0.85);

    /// `amount`, kept between 0 and 1.
    pub fn new(amount: f32) -> Self {
        Self(if amount.is_finite() {
            amount.clamp(0.0, 1.0)
        } else {
            Self::OCCASIONAL.0
        })
    }

    pub fn amount(self) -> f32 {
        self.0
    }

    /// In words: "sparse", "lively".
    pub fn name(self) -> &'static str {
        match self.0 {
            amount if amount < 0.15 => "very sparse",
            amount if amount < 0.38 => "sparse",
            amount if amount < 0.65 => "occasional",
            amount if amount < 0.9 => "lively",
            _ => "busy",
        }
    }

    /// Multiplies the instrument's spacing: notes 2.6 times as far apart
    /// at 0, a third as far at 1, once as far at the middle.
    fn factor(self) -> f32 {
        2f32.powf(2.8 * (0.5 - self.0))
    }
}

/// A layer's density as saved before it was an amount.
#[derive(Clone, Copy, Default, Deserialize)]
enum Step {
    #[default]
    #[serde(skip)]
    Unset,
    Sparse,
    Occasional,
    Lively,
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
#[serde(from = "SavedLayer")]
pub struct Layer {
    pub enabled: bool,
    /// 0 to 1, within the whole bed's level.
    pub level: f32,
    /// Saved as `notes`: `density` held one of three steps before.
    #[serde(rename = "notes")]
    pub density: Density,
    pub register: Register,
}

/// A layer as saved: its density an amount (`notes`), or from before
/// that, a step (`density`).
#[derive(Deserialize)]
#[serde(default)]
struct SavedLayer {
    enabled: bool,
    level: f32,
    /// NaN when not saved.
    notes: f32,
    density: Step,
    register: Register,
}

impl Default for SavedLayer {
    fn default() -> Self {
        let layer = Layer::default();
        Self {
            enabled: layer.enabled,
            level: layer.level,
            notes: f32::NAN,
            density: Step::Unset,
            register: layer.register,
        }
    }
}

impl From<SavedLayer> for Layer {
    fn from(saved: SavedLayer) -> Self {
        let density = if saved.notes.is_finite() {
            Density::new(saved.notes)
        } else {
            match saved.density {
                Step::Unset | Step::Sparse => Density::SPARSE,
                Step::Occasional => Density::OCCASIONAL,
                Step::Lively => Density::LIVELY,
            }
        };
        Self {
            enabled: saved.enabled,
            level: saved.level,
            density,
            register: saved.register,
        }
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            enabled: false,
            level: 0.8,
            density: Density::SPARSE,
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

/// One [`Layer`] per [`Instrument`], indexed by instrument.
///
/// Saved as a sequence in [`Instrument::ALL`]'s order. Reading accepts a
/// shorter sequence, from before an instrument was added, filling the
/// rest with switched-off layers, and ignores extra ones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layers([Layer; Instrument::COUNT]);

impl Default for Layers {
    fn default() -> Self {
        Self([Layer::default(); Instrument::COUNT])
    }
}

impl Layers {
    pub fn iter(&self) -> std::slice::Iter<'_, Layer> {
        self.0.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Layer> {
        self.0.iter_mut()
    }
}

impl Index<Instrument> for Layers {
    type Output = Layer;

    fn index(&self, instrument: Instrument) -> &Layer {
        &self.0[instrument.index()]
    }
}

impl IndexMut<Instrument> for Layers {
    fn index_mut(&mut self, instrument: Instrument) -> &mut Layer {
        &mut self.0[instrument.index()]
    }
}

impl<'a> IntoIterator for &'a mut Layers {
    type Item = &'a mut Layer;
    type IntoIter = std::slice::IterMut<'a, Layer>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

impl<'a> IntoIterator for &'a Layers {
    type Item = &'a Layer;
    type IntoIter = std::slice::Iter<'a, Layer>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl Serialize for Layers {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Layers {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct LayersVisitor;

        impl<'de> Visitor<'de> for LayersVisitor {
            type Value = Layers;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a sequence of layers")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Layers, A::Error> {
                let mut layers = Layers::default();
                let mut index = 0;
                while index < Instrument::COUNT {
                    match seq.next_element::<Layer>()? {
                        Some(layer) => layers.0[index] = layer,
                        None => return Ok(layers),
                    }
                    index += 1;
                }
                // Layers for instruments this version does not know.
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(layers)
            }
        }

        // A fixed-size array is saved as a tuple; asking for one reads
        // both that and a plain list, whatever their length.
        deserializer.deserialize_tuple(Instrument::COUNT, LayersVisitor)
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

/// A named character: where Variety, Concentration and Chaos sit
/// together. The Generator offers these first and the three amounts in
/// its details. Not saved: read back from the amounts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mood {
    Still,
    Gentle,
    Wandering,
    Restless,
    Unsettled,
}

impl Mood {
    pub const ALL: [Self; 5] = [
        Self::Still,
        Self::Gentle,
        Self::Wandering,
        Self::Restless,
        Self::Unsettled,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Still => "Still",
            Self::Gentle => "Gentle",
            Self::Wandering => "Wandering",
            Self::Restless => "Restless",
            Self::Unsettled => "Unsettled",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Still => "a short motif that keeps coming back, close to home",
            Self::Gentle => "now and then a motif returns; around the chord, in the key",
            Self::Wandering => "always new melodies, wandering wide, in the key",
            Self::Restless => "new melodies and chords, with a note outside the key now and then",
            Self::Unsettled => "dissonant: clashes, clusters and notes outside the key",
        }
    }

    /// Variety, concentration and chaos.
    pub fn character(self) -> (f32, f32, f32) {
        match self {
            Self::Still => (0.15, 0.75, 0.0),
            Self::Gentle => (DEFAULT_VARIETY, DEFAULT_CONCENTRATION, 0.0),
            Self::Wandering => (0.8, 0.25, 0.0),
            Self::Restless => (0.85, 0.4, 0.35),
            Self::Unsettled => (0.7, 0.3, 0.8),
        }
    }
}

/// Endless, or pieces with a beginning and an end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Form {
    #[default]
    Endless,
    /// A piece this long, a pause with the background only, then another.
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

/// Where the background's and the instruments' levels sit unless moved.
pub const DEFAULT_PART_LEVEL: f32 = 0.7;

/// The gain for the background's or the instruments' level: a square
/// law, so the slider sounds even along its length; 1 at the default.
fn part_gain(level: f32) -> f32 {
    let ratio = level / DEFAULT_PART_LEVEL;
    ratio * ratio
}

/// Where variety and concentration sit unless moved: the middle.
pub const DEFAULT_VARIETY: f32 = 0.5;
pub const DEFAULT_CONCENTRATION: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AmbienceSettings {
    pub enabled: bool,
    /// The preset these came from, if untouched since.
    pub style: Option<Style>,
    pub scale: Scale,
    /// Called the weather before there were places in it.
    #[serde(alias = "weather")]
    pub background: Background,
    /// One per [`Instrument`].
    pub layers: Layers,
    pub presence: Presence,
    /// Beats per minute of the pulse, when there is one.
    pub tempo: u16,
    pub form: Form,
    /// How it changes over time: steady, building up, or evolving.
    pub journey: Journey,
    /// 0 to 1: how much the music keeps changing. Low comes back to a
    /// short motif and stays on few chords; high keeps finding new
    /// melodies, more chords and freer rhythms.
    pub variety: f32,
    /// 0 to 1: how closely the notes gather. High keeps to the chord and
    /// the home note in a narrow range; low wanders wide in leaps.
    pub concentration: f32,
    /// 0 to 1: dissonance. Above 0, notes from outside the key, clashing
    /// seconds and tritones, unexpected chords and looser timing.
    pub chaos: f32,
    /// The whole bed, 0 to 1; even 1 is well under the sounds.
    pub level: f32,
    /// The background's own level against the instruments, 0 to 1:
    /// [`DEFAULT_PART_LEVEL`] is as it was made, 1 about twice as loud, 0
    /// silent.
    pub background_level: f32,
    /// All the instruments together against the background, the same way.
    pub instruments_level: f32,
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
        self.background_level = unit(self.background_level, DEFAULT_PART_LEVEL);
        self.instruments_level = unit(self.instruments_level, DEFAULT_PART_LEVEL);
        self.variety = unit(self.variety, DEFAULT_VARIETY);
        self.concentration = unit(self.concentration, DEFAULT_CONCENTRATION);
        self.chaos = unit(self.chaos, 0.0);
        for layer in &mut self.layers {
            layer.level = unit(layer.level, 0.8);
            layer.density = Density::new(layer.density.amount());
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

    /// The mood the character is at, if it is at one.
    pub fn mood(&self) -> Option<Mood> {
        let near = |a: f32, b: f32| (a - b).abs() < 0.005;
        Mood::ALL.into_iter().find(|mood| {
            let (variety, concentration, chaos) = mood.character();
            near(self.variety, variety)
                && near(self.concentration, concentration)
                && near(self.chaos, chaos)
        })
    }

    pub fn set_mood(&mut self, mood: Mood) {
        (self.variety, self.concentration, self.chaos) = mood.character();
    }

    /// True when the music is the same: everything but whether it plays,
    /// how it changes over time, the variation and the style it came from.
    /// Those are how a style is played, not what it is.
    pub fn same_music(&self, other: &Self) -> bool {
        Self {
            enabled: other.enabled,
            journey: other.journey,
            seed: other.seed,
            style: other.style,
            ..*self
        } == *other
    }

    pub fn layer(&self, instrument: Instrument) -> &Layer {
        &self.layers[instrument]
    }

    pub fn layer_mut(&mut self, instrument: Instrument) -> &mut Layer {
        self.style = None;
        &mut self.layers[instrument]
    }

    /// Instruments switched on.
    pub fn playing(&self) -> impl Iterator<Item = Instrument> + '_ {
        Instrument::ALL
            .into_iter()
            .filter(|instrument| self.layer(*instrument).enabled)
    }

    /// Seconds between a layer's notes at these settings, typically.
    pub fn spacing(&self, instrument: Instrument) -> f32 {
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
    SeasidePiano,
    ForestMorning,
    Campfire,
    DesertNight,
    StormWatch,
}

impl Style {
    pub const ALL: [Self; 11] = [
        Self::RainStudy,
        Self::WinterHush,
        Self::NightDrift,
        Self::Cathedral,
        Self::Pentatonic,
        Self::Affirmation,
        Self::SeasidePiano,
        Self::ForestMorning,
        Self::Campfire,
        Self::DesertNight,
        Self::StormWatch,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::WinterHush => "Winter Hush",
            Self::RainStudy => "Rain Study",
            Self::Cathedral => "Cathedral",
            Self::NightDrift => "Night Drift",
            Self::Affirmation => "Affirmation",
            Self::Pentatonic => "Pentatonic Garden",
            Self::SeasidePiano => "Seaside Piano",
            Self::ForestMorning => "Forest Morning",
            Self::Campfire => "Campfire",
            Self::DesertNight => "Desert Night",
            Self::StormWatch => "Storm Watch",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::WinterHush => "a cold wind, a bass note now and then, a harp far away",
            Self::RainStudy => "rain, harp and a bass string: the default",
            Self::Cathedral => "no background; a drone, a pad and sparse bells in a minor key",
            Self::NightDrift => "wind, a drone and a slow pad in Lydian, barely there",
            Self::Affirmation => "strings, bass, pad and bells on a slow pulse, in pieces",
            Self::Pentatonic => "rain and harp on five notes: nothing can clash",
            Self::SeasidePiano => "waves, a sparse piano and a low drone in Lydian",
            Self::ForestMorning => "birds and leaves, a harp, a plucked violin and a bell or two",
            Self::Campfire => "a crackling fire, plucked violin, a bass string, a soft drum",
            Self::DesertNight => "a dry wind, a drone, a few piano notes and a shaker in Phrygian",
            Self::StormWatch => "distant thunder, slow strings and a drone in harmonic minor",
        }
    }

    pub fn settings(self) -> AmbienceSettings {
        use Instrument::*;
        let mut layers = Layers::default();
        let mut set = |instrument: Instrument, layer: Layer| {
            layers[instrument] = layer;
        };
        let scale = |root: u8, kind: ScaleKind| Scale { root, kind };
        let mode = |root: u8, mode: Mode| Scale {
            root,
            kind: ScaleKind::Mode(mode),
        };
        let (scale, background, presence, form, level) = match self {
            Self::RainStudy => {
                set(Bass, Layer::on(0.8, Density::SPARSE));
                set(Harp, Layer::on(0.7, Density::OCCASIONAL));
                (
                    mode(2, Mode::Dorian),
                    Background::Rain,
                    Presence::Ambient,
                    Form::Endless,
                    0.35,
                )
            }
            Self::WinterHush => {
                set(Bass, Layer::on(0.7, Density::SPARSE));
                set(Harp, Layer::on(0.5, Density::SPARSE));
                (
                    mode(9, Mode::Aeolian),
                    Background::Winter,
                    Presence::Background,
                    Form::Endless,
                    0.3,
                )
            }
            Self::NightDrift => {
                set(Drone, Layer::on(0.6, Density::SPARSE));
                set(Pad, Layer::on(0.6, Density::SPARSE));
                (
                    mode(5, Mode::Lydian),
                    Background::Wind,
                    Presence::Background,
                    Form::Endless,
                    0.3,
                )
            }
            Self::Cathedral => {
                set(Drone, Layer::on(0.7, Density::SPARSE));
                set(Pad, Layer::on(0.7, Density::OCCASIONAL));
                set(Bells, Layer::on(0.5, Density::SPARSE));
                (
                    mode(7, Mode::Aeolian),
                    Background::None,
                    Presence::Ambient,
                    Form::Endless,
                    0.4,
                )
            }
            Self::Pentatonic => {
                set(Harp, Layer::on(0.8, Density::OCCASIONAL));
                set(Bells, Layer::on(0.4, Density::SPARSE));
                set(Bass, Layer::on(0.6, Density::SPARSE));
                (
                    scale(7, ScaleKind::MajorPentatonic),
                    Background::Rain,
                    Presence::Ambient,
                    Form::Endless,
                    0.35,
                )
            }
            Self::Affirmation => {
                set(Strings, Layer::on(0.8, Density::OCCASIONAL));
                set(Bass, Layer::on(0.8, Density::OCCASIONAL));
                set(Pad, Layer::on(0.6, Density::OCCASIONAL));
                set(Bells, Layer::on(0.5, Density::SPARSE));
                (
                    mode(0, Mode::Mixolydian),
                    Background::None,
                    Presence::Declarative,
                    Form::Piece {
                        minutes: 3.0,
                        pause_seconds: 20.0,
                    },
                    0.5,
                )
            }
            Self::SeasidePiano => {
                set(Piano, Layer::on(0.7, Density::OCCASIONAL));
                set(Drone, Layer::on(0.45, Density::SPARSE));
                set(Bass, Layer::on(0.5, Density::SPARSE));
                (
                    mode(2, Mode::Lydian),
                    Background::Waves,
                    Presence::Ambient,
                    Form::Endless,
                    0.4,
                )
            }
            Self::ForestMorning => {
                set(Harp, Layer::on(0.6, Density::OCCASIONAL));
                set(Violin, Layer::on(0.5, Density::SPARSE));
                set(Bells, Layer::on(0.35, Density::SPARSE));
                (
                    scale(4, ScaleKind::MajorPentatonic),
                    Background::Forest,
                    Presence::Ambient,
                    Form::Endless,
                    0.35,
                )
            }
            Self::Campfire => {
                set(Violin, Layer::on(0.6, Density::OCCASIONAL));
                set(Bass, Layer::on(0.6, Density::SPARSE));
                set(Drums, Layer::on(0.55, Density::SPARSE));
                (
                    mode(2, Mode::Mixolydian),
                    Background::Fire,
                    Presence::Ambient,
                    Form::Endless,
                    0.4,
                )
            }
            Self::DesertNight => {
                set(Drone, Layer::on(0.6, Density::SPARSE));
                set(Piano, Layer::on(0.45, Density::SPARSE));
                set(Drums, Layer::on(0.45, Density::SPARSE));
                (
                    mode(4, Mode::Phrygian),
                    Background::Desert,
                    Presence::Background,
                    Form::Endless,
                    0.35,
                )
            }
            Self::StormWatch => {
                set(Strings, Layer::on(0.6, Density::SPARSE));
                set(Pad, Layer::on(0.5, Density::SPARSE));
                set(Drone, Layer::on(0.5, Density::SPARSE));
                (
                    scale(9, ScaleKind::HarmonicMinor),
                    Background::Thunder,
                    Presence::Ambient,
                    Form::Endless,
                    0.35,
                )
            }
        };
        AmbienceSettings {
            enabled: false,
            style: Some(self),
            scale,
            background,
            layers,
            presence,
            tempo: 60,
            form,
            journey: Journey::Steady,
            variety: DEFAULT_VARIETY,
            concentration: DEFAULT_CONCENTRATION,
            chaos: 0.0,
            level,
            background_level: DEFAULT_PART_LEVEL,
            instruments_level: DEFAULT_PART_LEVEL,
            duck: true,
            seed: 1,
        }
    }
}

// ----- surprise --------------------------------------------------------

/// One of `choices`, more often the heavier.
fn weighted<T: Copy>(rng: &mut Rng, choices: &[(T, f32)]) -> T {
    let total: f32 = choices.iter().map(|(_, weight)| weight).sum();
    let mut roll = rng.unit() * total;
    for &(choice, weight) in choices {
        if roll < weight {
            return choice;
        }
        roll -= weight;
    }
    choices[choices.len() - 1].0
}

/// Settings picked at random from what tends to sound well together: a
/// key (never Locrian, whole tone rarely), a background or none, two to
/// four instruments with something low or long beneath the rest, a mood
/// and a presence. Off, endless and steady; the same seed picks the same.
pub fn surprise(seed: u64) -> AmbienceSettings {
    use Instrument::*;
    let mut rng = Rng::new(seed ^ 0x5DEE_CE66_D1CE_D1CE);
    let mode = |mode: Mode| ScaleKind::Mode(mode);
    let kind = weighted(
        &mut rng,
        &[
            (mode(Mode::Ionian), 2.0),
            (mode(Mode::Dorian), 3.0),
            (mode(Mode::Phrygian), 1.0),
            (mode(Mode::Lydian), 2.0),
            (mode(Mode::Mixolydian), 2.0),
            (mode(Mode::Aeolian), 3.0),
            (ScaleKind::MajorPentatonic, 2.0),
            (ScaleKind::MinorPentatonic, 2.0),
            (ScaleKind::HarmonicMinor, 1.0),
            (ScaleKind::WholeTone, 0.4),
        ],
    );
    let scale = Scale {
        root: rng.below(12) as u8,
        kind,
    };
    let background = weighted(
        &mut rng,
        &[
            (Background::None, 1.0),
            (Background::Rain, 2.0),
            (Background::Thunder, 1.0),
            (Background::Wind, 1.5),
            (Background::Winter, 1.0),
            (Background::Waves, 2.0),
            (Background::Forest, 2.0),
            (Background::Desert, 1.0),
            (Background::Fire, 1.5),
        ],
    );
    let presence = weighted(
        &mut rng,
        &[
            (Presence::Background, 1.0),
            (Presence::Ambient, 3.5),
            (Presence::Declarative, 0.8),
        ],
    );

    let mut layers = Layers::default();
    // Something low or long beneath, now and then two.
    let grounds = [(Drone, 1.0), (Pad, 1.0), (Bass, 1.2)];
    let ground = weighted(&mut rng, &grounds);
    layers[ground] = Layer::on(rng.range(0.5, 0.7), Density::SPARSE);
    if rng.chance(0.3) {
        let others: Vec<(Instrument, f32)> =
            grounds.into_iter().filter(|(i, _)| *i != ground).collect();
        let second = weighted(&mut rng, &others);
        layers[second] = Layer::on(rng.range(0.45, 0.6), Density::SPARSE);
    }
    // One or two above it.
    let mut melodic = vec![
        (Harp, 1.3),
        (Piano, 1.2),
        (Violin, 1.0),
        (Bells, 0.8),
        (Strings, 0.9),
    ];
    for _ in 0..if rng.chance(0.6) { 1 } else { 2 } {
        let voice = weighted(&mut rng, &melodic);
        melodic.retain(|(i, _)| *i != voice);
        let level = match voice {
            Bells => rng.range(0.35, 0.5),
            _ => rng.range(0.5, 0.75),
        };
        layers[voice] = Layer::on(level, Density::new(rng.range(0.2, 0.6)));
    }
    // Soft drums, sometimes, where there is room and something to notice.
    let playing = layers.iter().filter(|layer| layer.enabled).count();
    if playing < 4 && presence != Presence::Background && rng.chance(0.2) {
        layers[Drums] = Layer::on(rng.range(0.4, 0.55), Density::SPARSE);
    }

    let mood = weighted(
        &mut rng,
        &[
            (Mood::Still, 2.0),
            (Mood::Gentle, 3.0),
            (Mood::Wandering, 2.0),
            (Mood::Restless, 1.0),
            (Mood::Unsettled, 0.4),
        ],
    );
    let (variety, concentration, chaos) = mood.character();
    let tempo = if presence.pulsed() {
        50 + rng.below(23) as u16
    } else {
        60
    };
    AmbienceSettings {
        enabled: false,
        style: None,
        scale,
        background,
        layers,
        presence,
        tempo,
        form: Form::Endless,
        journey: Journey::Steady,
        variety,
        concentration,
        chaos,
        level: rng.range(0.35, 0.45),
        background_level: DEFAULT_PART_LEVEL,
        instruments_level: DEFAULT_PART_LEVEL,
        duck: true,
        seed: seed.max(1),
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
/// How much of the instruments goes into their hall, and how much they
/// are trimmed by so that, hall and all, they sit where they did.
const HALL_SEND: f32 = 0.6;
const MUSIC_TRIM: f32 = 0.72;
/// Below this the hall's tail is no longer heard, and rendering can stop.
const HALL_QUIET: f32 = 1e-6;

/// The ambience, synthesised. Built once with everything it needs.
pub struct Ambience {
    rate: u32,
    settings: AmbienceSettings,
    /// What plays now: the settings as the journey has taken them.
    heard: AmbienceSettings,
    course: Course,
    composer: Composer,
    synth: Synth,
    bed: Bed,
    /// Frames rendered so far.
    clock: u64,
    /// On and off, slowly.
    gain: Ramp,
    duck: Ramp,
    /// The background against the instruments, and they against it.
    bed_gain: Ramp,
    music_level: Ramp,
    /// The instruments' ensemble and their hall, and whether its tail
    /// still rings.
    chorus: Chorus,
    hall: Hall,
    ringing: bool,
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
            heard: settings,
            course: Course::new(rate, seed),
            composer: Composer::new(rate, seed),
            synth: Synth::new(rate, seed ^ 0x9E37_79B9_7F4A_7C15),
            bed: Bed::new(rate, seed.rotate_left(17)),
            clock: 0,
            gain: Ramp::at(0.0),
            duck: Ramp::at(1.0),
            bed_gain: Ramp::at(1.0),
            music_level: Ramp::at(1.0),
            chorus: Chorus::new(rate),
            hall: Hall::new(rate),
            ringing: false,
        }
    }

    pub fn settings(&self) -> &AmbienceSettings {
        &self.settings
    }

    /// Applies new settings: on and off fade over a second; the background
    /// crosses over; the key, layers and form take effect from the next
    /// event, and a new seed, form or journey starts the music afresh.
    pub fn set(&mut self, settings: AmbienceSettings) {
        let settings = settings.clamped();
        let was_on = self.settings.enabled;
        let afresh = if settings.enabled != was_on {
            let target = if settings.enabled { 1.0 } else { 0.0 };
            self.gain.towards(target, self.rate);
            settings.enabled
        } else {
            settings.seed != self.settings.seed
                || settings.form != self.settings.form
                || settings.journey != self.settings.journey
        };
        if afresh {
            self.composer.reset(settings.seed);
            self.course.reset(settings.seed, self.clock);
        }
        self.settings = settings;
    }

    /// Plays an instrument from recordings, or with `None` from its
    /// synthesised voice again. Notes on the old recordings fade out.
    pub fn set_samples(
        &mut self,
        instrument: Instrument,
        samples: Option<std::sync::Arc<crate::instruments::Multisample>>,
    ) {
        self.synth.set_samples(instrument, samples);
    }

    /// Recordings no longer played, for the app to free off the audio
    /// thread.
    pub fn take_released(&mut self) -> Option<std::sync::Arc<crate::instruments::Multisample>> {
        self.synth.take_released()
    }

    /// What a building or evolving ambience plays now; `None` when it is
    /// off or steady.
    pub fn now(&self) -> Option<Now> {
        (self.settings.enabled && self.settings.journey != Journey::Steady)
            .then(|| self.course.now(&self.heard))
    }

    /// Takes up what the journey says plays now: the background crosses
    /// over, a change of key lets held notes go and voices the new chord,
    /// and the synth follows the layers.
    fn follow(&mut self, heard: AmbienceSettings, now: u64) {
        let before = std::mem::replace(&mut self.heard, heard);
        self.bed.set_background(heard.background);
        if heard.scale != before.scale {
            self.synth.release_held();
            self.composer.modulate(&heard, now);
        }
        if heard.layers != before.layers {
            self.synth.follow(&heard);
        }
    }

    /// True while anything may be heard.
    pub fn is_audible(&self) -> bool {
        self.settings.enabled || !self.gain.is_silent() || self.synth.is_sounding() || self.ringing
    }

    /// Adds the next `out.len() / channels` frames into `out`: the
    /// background, and the instruments through their chorus and hall.
    /// `ducking` while sounds play.
    pub fn render(&mut self, out: &mut [f32], channels: usize, ducking: bool) {
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
        let bed_target = part_gain(self.settings.background_level);
        if self.bed_gain.target() != bed_target {
            self.bed_gain.towards(bed_target, self.rate / 10);
        }
        let music_target = part_gain(self.settings.instruments_level);
        if self.music_level.target() != music_target {
            self.music_level.towards(music_target, self.rate / 10);
        }
        let (from, to) = (self.clock, self.clock + frames as u64);
        if self.settings.enabled {
            // A composer about to start begins its first piece now.
            let piece = if self.composer.is_started() {
                self.composer.origin()
            } else {
                from
            };
            let heard = self.course.settings(&self.settings, from, piece);
            self.follow(heard, from);
            self.composer.poll(&self.heard, from, to);
        }
        let music_gain = self.composer.music_gain();
        let mut tail = 0.0f32;
        for (i, frame) in out.chunks_exact_mut(channels).enumerate() {
            let now = from + i as u64;
            while let Some(event) = self.composer.due(now) {
                self.synth.play(event, &self.heard);
            }
            let gain = self.gain.next() * self.duck.next() * level;
            let bed_gain = self.bed_gain.next();
            let (mut left, mut right) = self.bed.next();
            left *= bed_gain;
            right *= bed_gain;
            let (sl, sr) = self
                .synth
                .next(music_gain * self.music_level.next() * MUSIC_TRIM);
            let (sl, sr) = self.chorus.process(sl, sr);
            // The hall rings on after the bed is switched off.
            let (hl, hr) = self
                .hall
                .process(sl * gain * HALL_SEND, sr * gain * HALL_SEND);
            tail = tail.max(hl.abs()).max(hr.abs());
            left = (left + sl) * gain + hl;
            right = (right + sr) * gain + hr;
            if channels == 1 {
                frame[0] += 0.5 * (left + right);
            } else {
                frame[0] += left;
                frame[1] += right;
            }
        }
        self.ringing = tail > HALL_QUIET;
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
    fn settings_saved_before_new_instruments_and_backgrounds_still_load() {
        // Six layers and `weather`, as saved before there were nine
        // instruments and backgrounds that are places.
        let layer = r#"{"enabled":true,"level":0.7,"density":"Occasional","register":"High"}"#;
        let off = r#"{"enabled":false}"#;
        let old = format!(
            r#"{{"enabled":true,"style":null,"weather":"Winter","layers":[{off},{layer},{off},{off},{off},{off}]}}"#
        );
        let settings: AmbienceSettings = serde_json::from_str(&old).unwrap();
        assert_eq!(settings.background, Background::Winter);
        assert_eq!(
            (settings.variety, settings.concentration, settings.chaos),
            (DEFAULT_VARIETY, DEFAULT_CONCENTRATION, 0.0),
            "saved before the character, in the middle and in the key"
        );
        assert_eq!(settings.background_level, DEFAULT_PART_LEVEL);
        assert_eq!(settings.instruments_level, DEFAULT_PART_LEVEL);
        let harp = settings.layer(Instrument::Harp);
        assert!(harp.enabled && harp.register == Register::High);
        assert_eq!(
            harp.density,
            Density::OCCASIONAL,
            "a step becomes its amount"
        );
        for instrument in [Instrument::Piano, Instrument::Violin, Instrument::Drums] {
            assert!(!settings.layer(instrument).enabled);
        }

        // More layers than this version knows are ignored.
        let ten = [layer; 10].join(",");
        let layers: Layers = serde_json::from_str(&format!("[{ten}]")).unwrap();
        assert!(layers.iter().all(|layer| layer.enabled));

        let mut settings = Style::Campfire.settings();
        settings.layers[Instrument::Harp].density = Density::new(0.7);
        let saved = serde_json::to_string(&settings).unwrap();
        assert!(saved.contains("\"background\":\"Fire\""));
        assert!(
            saved.contains("\"notes\":0.7"),
            "density saved as an amount"
        );
        assert_eq!(
            serde_json::from_str::<AmbienceSettings>(&saved).unwrap(),
            settings
        );
    }

    #[test]
    fn each_mood_reads_back_and_a_nudge_makes_the_character_its_own() {
        let mut settings = Style::RainStudy.settings();
        for style in Style::ALL {
            assert_eq!(style.settings().mood(), Some(Mood::Gentle), "{style:?}");
        }
        for mood in Mood::ALL {
            settings.set_mood(mood);
            assert_eq!(settings.mood(), Some(mood));
            assert_eq!(settings.clamped().mood(), Some(mood), "within range");
        }
        settings.chaos += 0.01;
        assert_eq!(settings.mood(), None);
    }

    #[test]
    fn the_same_music_ignores_playing_the_variation_and_the_style() {
        let rain = Style::RainStudy.settings();
        let mut other = AmbienceSettings {
            enabled: true,
            seed: 9,
            style: None,
            ..rain
        };
        assert!(rain.same_music(&other));
        other.journey = Journey::Evolve;
        assert!(rain.same_music(&other), "a way of playing it");
        other.chaos = 0.3;
        assert!(!rain.same_music(&other));
        assert!(!rain.same_music(&Style::Campfire.settings()));
    }

    #[test]
    fn a_surprise_is_a_tasteful_handful_and_the_same_seed_picks_the_same() {
        use std::collections::HashSet;
        let (mut kinds, mut backgrounds, mut instruments) =
            (HashSet::new(), HashSet::new(), HashSet::new());
        for seed in 1..500 {
            let settings = surprise(seed);
            assert_eq!(settings, surprise(seed));
            assert_eq!(settings.clamped(), settings, "within range");
            assert_eq!(settings.style, None);
            assert!(!settings.enabled);
            assert!(settings.mood().is_some(), "at a mood");
            let playing: Vec<Instrument> = settings.playing().collect();
            assert!((2..=4).contains(&playing.len()), "{playing:?}");
            assert!(
                playing
                    .iter()
                    .any(|i| matches!(i, Instrument::Drone | Instrument::Pad | Instrument::Bass)),
                "something beneath: {playing:?}"
            );
            assert_ne!(settings.scale.kind, ScaleKind::Mode(Mode::Locrian));
            kinds.insert(settings.scale.kind);
            backgrounds.insert(settings.background);
            instruments.extend(playing);
        }
        assert!(kinds.len() >= 9, "{kinds:?}");
        assert_eq!(backgrounds.len(), Background::ALL.len());
        assert_eq!(instruments.len(), Instrument::COUNT);
        assert_ne!(surprise(1), surprise(2));
    }

    #[test]
    fn the_background_has_its_own_level_beside_the_instruments() {
        let rate = 48_000;
        // The background alone, its level over the last second.
        let level = |background_level: f32| {
            let mut settings = Style::RainStudy.settings();
            settings.enabled = true;
            settings.background_level = background_level;
            for instrument in Instrument::ALL {
                settings.layers[instrument].enabled = false;
            }
            let mut ambience = Ambience::new(rate as u32, 4);
            ambience.set(settings);
            let mut out = vec![0.0; rate * 4 * 2];
            ambience.render(&mut out, 2, false);
            let last = &out[out.len() - rate * 2..];
            (last.iter().map(|s| s * s).sum::<f32>() / last.len() as f32).sqrt()
        };
        let made = level(DEFAULT_PART_LEVEL);
        let half = level(DEFAULT_PART_LEVEL / 2.0);
        let full = level(1.0);
        assert!(made > 0.0);
        assert!(
            (half / made - 0.25).abs() < 0.03,
            "a quarter as loud: {half} / {made}"
        );
        assert!(full > made * 1.8, "louder: {full} / {made}");
        assert_eq!(level(0.0), 0.0, "silent");
    }

    #[test]
    fn the_instruments_have_a_level_together_beside_the_background() {
        let rate = 48_000;
        // A drone alone, no background: its level over the last second.
        let level = |instruments_level: f32| {
            let mut settings = Style::Cathedral.settings();
            settings.enabled = true;
            settings.background = Background::None;
            settings.instruments_level = instruments_level;
            for instrument in Instrument::ALL {
                settings.layers[instrument].enabled = instrument == Instrument::Drone;
            }
            let mut ambience = Ambience::new(rate as u32, 5);
            ambience.set(settings);
            let mut out = vec![0.0; rate * 8 * 2];
            ambience.render(&mut out, 2, false);
            // Dry and hall together: the hall follows the instruments.
            let last = &out[out.len() - rate * 2..];
            (last.iter().map(|s| s * s).sum::<f32>() / last.len() as f32).sqrt()
        };
        let made = level(DEFAULT_PART_LEVEL);
        let half = level(DEFAULT_PART_LEVEL / 2.0);
        assert!(made > 0.0);
        assert!((half / made - 0.25).abs() < 0.03, "{half} / {made}");
        assert!(level(0.0) < 1e-8, "silent, but for the hall's last breath");
    }

    /// Prints how much louder each instrument of the pack in `OLIO_PACK`
    /// is than its synthesised voice, to set the pack's gains; run with
    /// `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn measure_recorded_against_synthesised() {
        use crate::instruments::Pack;
        let Ok(dir) = std::env::var("OLIO_PACK") else {
            return;
        };
        let pack = Pack::open(std::path::Path::new(&dir)).unwrap();
        let rate = 48_000;
        for instrument in pack.instruments() {
            let samples = std::sync::Arc::new(pack.load(instrument).unwrap().unwrap());
            let level = |recorded: bool| {
                let mut settings = Style::RainStudy.settings();
                settings.enabled = true;
                settings.background = Background::None;
                for other in Instrument::ALL {
                    settings.layers[other].enabled = other == instrument;
                }
                settings.layers[instrument].density = Density::LIVELY;
                let mut ambience = Ambience::new(rate as u32, 9);
                if recorded {
                    ambience.set_samples(instrument, Some(samples.clone()));
                }
                ambience.set(settings);
                let mut out = vec![0.0; rate * 60 * 2];
                ambience.render(&mut out, 2, false);
                (out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt()
            };
            let (synth, recorded) = (level(false), level(true));
            println!(
                "GAIN {instrument:?}: synth {synth:.5}, recorded {recorded:.5}, gain {:.3}",
                synth / recorded
            );
        }
    }

    #[test]
    fn the_drone_follows_its_level_and_stops_when_switched_off() {
        let mut settings = Style::Cathedral.settings();
        settings.enabled = true;
        for instrument in Instrument::ALL {
            settings.layers[instrument].enabled = instrument == Instrument::Drone;
        }
        let rate = 48_000;
        // The level over the last second of `seconds` more.
        let level = |ambience: &mut Ambience, seconds: usize| {
            let mut out = vec![0.0; rate * seconds * 2];
            ambience.render(&mut out, 2, false);
            let last = &out[out.len() - rate * 2..];
            (last.iter().map(|s| s * s).sum::<f32>() / last.len() as f32).sqrt()
        };
        let mut ambience = Ambience::new(rate as u32, 3);
        ambience.set(settings);
        let full = level(&mut ambience, 8);
        assert!(full > 0.0);
        settings.layers[Instrument::Drone].level = 0.25;
        ambience.set(settings);
        let quiet = level(&mut ambience, 4);
        assert!(
            quiet < full * 0.6 && quiet > full * 0.1,
            "{quiet} from {full}"
        );
        settings.layers[Instrument::Drone].enabled = false;
        ambience.set(settings);
        let off = level(&mut ambience, 6);
        assert!(off < full * 0.01, "{off}");
        settings.layers[Instrument::Drone].enabled = true;
        ambience.set(settings);
        let again = level(&mut ambience, 8);
        assert!(again > full * 0.2, "back: {again}");
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
            ambience.render(&mut out, 2, false);
            out
        };
        assert_eq!(render(), render());
    }

    #[test]
    fn every_instrument_alone_sounds_and_stays_under_the_ceiling() {
        for instrument in Instrument::ALL {
            let mut settings = Style::RainStudy.settings();
            settings.enabled = true;
            settings.background = Background::None;
            settings.level = 1.0;
            settings.presence = Presence::Declarative;
            for layer in &mut settings.layers {
                layer.enabled = false;
            }
            *settings.layer_mut(instrument) = Layer::on(1.0, Density::LIVELY);
            let mut ambience = Ambience::new(48_000, 3);
            ambience.set(settings);
            let mut out = vec![0.0; 48_000 * 2 * 40];
            ambience.render(&mut out, 2, false);
            let peak = out.iter().fold(0.0f32, |p, s| p.max(s.abs()));
            assert!(peak > 0.002, "{} is silent", instrument.name());
            assert!(peak < 0.6, "{} peaks at {peak}", instrument.name());
            assert!(out.iter().all(|s| s.is_finite()));
        }
    }
}

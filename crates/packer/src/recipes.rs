//! Which recordings make each instrument, and how they are trimmed.

use olio_audio::ambience::Instrument;

/// Recordings from one folder of a library.
pub struct Source {
    pub repo: &'static str,
    pub folder: &'static str,
    /// The `_`-separated parts naming each velocity layer, softest first;
    /// empty for one layer.
    pub layers: &'static [&'static str],
    /// A file is taken if its name contains one of these (all if empty)
    /// and none of `exclude`.
    pub any: &'static [&'static str],
    pub exclude: &'static [&'static str],
    /// Added to the octave in the names: these libraries call middle C
    /// C3, so 1 (checked by `olio-packer check`).
    pub octave: i32,
    /// Roots kept, MIDI, inclusive: where the library overlaps another.
    pub roots: (u8, u8),
    /// For recordings without a note in the name: the note they are taken
    /// as, and the notes they play.
    pub root: Option<(u8, u8, u8)>,
    pub pitched: bool,
}

impl Source {
    const fn new(repo: &'static str, folder: &'static str) -> Self {
        Self {
            repo,
            folder,
            layers: &[],
            any: &[],
            exclude: &[],
            octave: 1,
            roots: (0, 127),
            root: None,
            pitched: true,
        }
    }
}

/// One instrument of the pack.
pub struct Recipe {
    pub instrument: Instrument,
    /// Its folder in the pack.
    pub dir: &'static str,
    pub sources: &'static [Source],
    pub attack: f32,
    pub release: f32,
    /// The longest a note is kept, and how long the end fades; held
    /// instruments loop instead.
    pub seconds: f32,
    pub fade: f32,
    pub looped: bool,
    /// The gain against the synthesised voice: set so each instrument is
    /// about as loud recorded as synthesised (measured with the ignored
    /// test `measure_recorded_against_synthesised` in olio-audio); the
    /// pad's recording is a layer beneath its synthesised voice.
    pub gain: f32,
}

const VCSL: &str = "VCSL";
const VSCO: &str = "VSCO-2-CE";

pub const RECIPES: &[Recipe] = &[
    Recipe {
        instrument: Instrument::Piano,
        dir: "piano",
        sources: &[Source {
            layers: &["v1", "v2"],
            ..Source::new(VCSL, "Chordophones/Zithers/Grand Piano, Kawai/Sustains")
        }],
        attack: 0.001,
        release: 0.8,
        seconds: 11.0,
        fade: 3.0,
        looped: false,
        gain: 3.95,
    },
    Recipe {
        instrument: Instrument::Harp,
        dir: "harp",
        sources: &[Source {
            layers: &["mf1", "f1"],
            // Named with middle C as C4.
            octave: 0,
            ..Source::new(VCSL, "Chordophones/Composite Chordophones/Concert Harp")
        }],
        attack: 0.001,
        release: 0.5,
        seconds: 8.0,
        fade: 2.5,
        looped: false,
        gain: 0.46,
    },
    Recipe {
        instrument: Instrument::Bells,
        dir: "bells",
        sources: &[Source::new(
            VCSL,
            "Idiophones/Struck Idiophones/Hand Chimes",
        )],
        attack: 0.001,
        release: 1.0,
        seconds: 11.0,
        fade: 3.0,
        looped: false,
        gain: 0.6,
    },
    Recipe {
        instrument: Instrument::Violin,
        dir: "violin",
        sources: &[Source {
            layers: &["p", "f"],
            octave: 0,
            ..Source::new(VSCO, "Strings/Solo Violin/Pizz")
        }],
        attack: 0.001,
        release: 0.3,
        seconds: 3.0,
        fade: 1.0,
        looped: false,
        gain: 1.27,
    },
    Recipe {
        instrument: Instrument::Bass,
        dir: "bass",
        sources: &[Source {
            layers: &["v1", "v3"],
            // Both takes sound two octaves under their name.
            exclude: &["_C#2_"],
            ..Source::new(VSCO, "Strings/Solo Contrabass/Pizz")
        }],
        attack: 0.001,
        release: 0.4,
        seconds: 4.5,
        fade: 1.5,
        looped: false,
        gain: 0.48,
    },
    Recipe {
        instrument: Instrument::Strings,
        dir: "strings",
        // The cellos low, the violas in the middle, the violins high.
        sources: &[
            Source {
                layers: &["v1", "v3"],
                roots: (0, 54),
                ..Source::new(VSCO, "Strings/Cello Section/susvib")
            },
            Source {
                layers: &["v1", "v2"],
                roots: (55, 65),
                ..Source::new(VSCO, "Strings/Viola Section/susvib")
            },
            Source {
                layers: &["v1", "v2"],
                roots: (66, 127),
                ..Source::new(VSCO, "Strings/Violin Section/susVib")
            },
        ],
        attack: 0.25,
        release: 1.6,
        seconds: 0.0,
        fade: 0.0,
        looped: true,
        gain: 0.37,
    },
    Recipe {
        instrument: Instrument::Pad,
        dir: "pad",
        // Bowed vibraphone, beneath the synthesised pad.
        sources: &[Source::new(
            VCSL,
            "Idiophones/Struck Idiophones/Vibraphone/Bowed",
        )],
        attack: 0.6,
        release: 3.0,
        seconds: 0.0,
        fade: 0.0,
        looped: true,
        gain: 1.4,
    },
    Recipe {
        instrument: Instrument::Drums,
        dir: "drums",
        sources: &[
            // The large frame drum, struck, as the note the composer
            // gives it (C2 to B2), shifted a little.
            Source {
                layers: &["v2", "v3"],
                any: &["HDrumL_Hit_"],
                root: Some((41, 0, 119)),
                ..Source::new(VCSL, "Membranophones/Struck Membranophones/Frame Drum")
            },
            // Single strokes of the small shaker, as recorded.
            Source {
                any: &["_Down_", "_Up_"],
                root: Some((120, 120, 120)),
                pitched: false,
                ..Source::new(VCSL, "Idiophones/Struck Idiophones/Shaker, Small")
            },
        ],
        attack: 0.0005,
        release: 0.2,
        seconds: 2.0,
        fade: 0.6,
        looped: false,
        gain: 0.93,
    },
];

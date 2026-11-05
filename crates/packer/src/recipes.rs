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
    /// Every this many roots of a layer are kept (the first and the last
    /// always): 1 keeps all, 2 every other, to keep the pack small where
    /// a library records every note.
    pub every: usize,
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
            every: 1,
        }
    }
}

/// One voice of an instrument of the pack: the Generator's role, and the
/// recordings that play it.
pub struct Recipe {
    pub instrument: Instrument,
    /// Which voice of the role (0 is the role's own, played unless a layer
    /// asks for another), and what it is called.
    pub voice: u8,
    pub name: &'static str,
    /// Its folder in the pack.
    pub dir: &'static str,
    pub sources: &'static [Source],
    pub attack: f32,
    pub release: f32,
    /// The longest a note is kept, and how long the end fades; held
    /// instruments loop instead, in at most `seconds` (0: as long as it is
    /// steady) of their steady part, the join blended `loop_blend` seconds.
    pub seconds: f32,
    pub fade: f32,
    pub looped: bool,
    pub loop_blend: f32,
    /// How far, in velocity, neighbouring layers blend; 0 switches at the
    /// line. At most half the narrowest layer.
    pub blend: f32,
    /// The notes it plays at their own pitch; others are moved by octaves
    /// into it (see the pack format). `None`: all, stretched from the
    /// nearest recording.
    pub range: Option<(u8, u8)>,
    /// The gain against the synthesised voice: set so each instrument is
    /// about as loud recorded as synthesised (measured with the ignored
    /// test `measure_recorded_against_synthesised` in olio-audio); the
    /// pad's recording is a layer beneath its synthesised voice.
    pub gain: f32,
}

const VCSL: &str = "VCSL";
const VSCO: &str = "VSCO-2-CE";

pub const RECIPES: &[Recipe] = &[
    // ----- piano
    Recipe {
        instrument: Instrument::Piano,
        voice: 0,
        name: "Grand piano",
        dir: "piano",
        sources: &[Source {
            layers: &["v1", "v2", "v3"],
            ..Source::new(VCSL, "Chordophones/Zithers/Grand Piano, Kawai/Sustains")
        }],
        attack: 0.001,
        release: 0.8,
        seconds: 7.0,
        fade: 2.5,
        looped: false,
        loop_blend: 0.0,
        blend: 0.12,
        range: None,
        gain: 1.20,
    },
    // ----- harp
    Recipe {
        instrument: Instrument::Harp,
        voice: 0,
        name: "Concert harp",
        dir: "harp",
        sources: &[Source {
            layers: &["mf1", "f1"],
            // Named with middle C as C4.
            octave: 0,
            ..Source::new(VCSL, "Chordophones/Composite Chordophones/Concert Harp")
        }],
        attack: 0.001,
        release: 0.5,
        seconds: 6.5,
        fade: 2.0,
        looped: false,
        loop_blend: 0.0,
        blend: 0.12,
        range: None,
        gain: 0.39,
    },
    Recipe {
        instrument: Instrument::Harp,
        voice: 1,
        name: "Folk harp",
        dir: "folk-harp",
        sources: &[Source {
            layers: &["v2", "v3"],
            ..Source::new(VCSL, "Chordophones/Composite Chordophones/Folk Harp")
        }],
        attack: 0.001,
        release: 0.5,
        seconds: 5.5,
        fade: 2.0,
        looped: false,
        loop_blend: 0.0,
        blend: 0.12,
        range: None,
        gain: 0.42,
    },
    Recipe {
        instrument: Instrument::Harp,
        voice: 2,
        name: "Dan tranh",
        dir: "dan-tranh",
        // A Vietnamese zither, tuned to a pentatonic scale.
        sources: &[Source {
            layers: &["mf", "f", "ff"],
            ..Source::new(VCSL, "Chordophones/Zithers/Dan Tranh/Normal")
        }],
        attack: 0.001,
        release: 0.4,
        seconds: 5.5,
        fade: 2.0,
        looped: false,
        loop_blend: 0.0,
        blend: 0.08,
        range: None,
        gain: 0.64,
    },
    Recipe {
        instrument: Instrument::Harp,
        voice: 3,
        name: "Kalimba",
        dir: "kalimba",
        sources: &[Source::new(
            VCSL,
            "Idiophones/Plucked Idiophones/Kalimba, Tanzania",
        )],
        attack: 0.001,
        release: 0.4,
        seconds: 5.0,
        fade: 1.5,
        looped: false,
        loop_blend: 0.0,
        blend: 0.0,
        range: None,
        gain: 0.73,
    },
    // ----- bells
    Recipe {
        instrument: Instrument::Bells,
        voice: 0,
        name: "Hand chimes",
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
        loop_blend: 0.0,
        blend: 0.0,
        range: None,
        gain: 1.12,
    },
    Recipe {
        instrument: Instrument::Bells,
        voice: 1,
        name: "Glockenspiel",
        dir: "glockenspiel",
        sources: &[Source {
            layers: &["soft", "medium", "loud"],
            ..Source::new(VCSL, "Idiophones/Struck Idiophones/Glockenspiel")
        }],
        attack: 0.001,
        release: 0.8,
        seconds: 6.0,
        fade: 2.0,
        looped: false,
        loop_blend: 0.0,
        blend: 0.1,
        range: None,
        gain: 4.71,
    },
    Recipe {
        instrument: Instrument::Bells,
        voice: 2,
        name: "Vibraphone",
        dir: "vibraphone",
        sources: &[Source {
            layers: &["v1", "v2"],
            ..Source::new(VCSL, "Idiophones/Struck Idiophones/Vibraphone/Soft Mallets")
        }],
        attack: 0.001,
        release: 1.0,
        seconds: 7.0,
        fade: 2.5,
        looped: false,
        loop_blend: 0.0,
        blend: 0.12,
        range: None,
        gain: 0.38,
    },
    // ----- violin (plucked)
    Recipe {
        instrument: Instrument::Violin,
        voice: 0,
        name: "Pizzicato violin",
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
        loop_blend: 0.0,
        blend: 0.10,
        range: None,
        gain: 0.68,
    },
    // ----- bass
    Recipe {
        instrument: Instrument::Bass,
        voice: 0,
        name: "Pizzicato double bass",
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
        loop_blend: 0.0,
        blend: 0.10,
        range: None,
        gain: 0.46,
    },
    // ----- strings (and the winds that hold a note the same way)
    Recipe {
        instrument: Instrument::Strings,
        voice: 0,
        name: "String sections",
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
        seconds: 4.5,
        fade: 0.0,
        looped: true,
        loop_blend: 1.0,
        blend: 0.10,
        range: None,
        gain: 0.35,
    },
    Recipe {
        instrument: Instrument::Strings,
        voice: 1,
        name: "Flute",
        dir: "flute",
        sources: &[Source {
            layers: &["v1", "v3"],
            ..Source::new(VSCO, "Woodwinds/Flute/susNV")
        }],
        attack: 0.35,
        release: 1.2,
        seconds: 3.5,
        fade: 0.0,
        looped: true,
        loop_blend: 1.0,
        blend: 0.10,
        range: None,
        gain: 0.58,
    },
    Recipe {
        instrument: Instrument::Strings,
        voice: 2,
        name: "Clarinet",
        dir: "clarinet",
        sources: &[Source {
            layers: &["v1", "v3"],
            ..Source::new(VSCO, "Woodwinds/Clarinet/susLong")
        }],
        attack: 0.35,
        release: 1.2,
        seconds: 3.5,
        fade: 0.0,
        looped: true,
        loop_blend: 1.0,
        blend: 0.10,
        range: None,
        gain: 0.31,
    },
    Recipe {
        instrument: Instrument::Strings,
        voice: 3,
        name: "Horn",
        dir: "horn",
        sources: &[Source {
            layers: &["v1", "v2"],
            ..Source::new(VSCO, "Brass/F Horn/sus")
        }],
        attack: 0.45,
        release: 1.4,
        seconds: 3.5,
        fade: 0.0,
        looped: true,
        loop_blend: 1.0,
        blend: 0.10,
        range: None,
        gain: 0.42,
    },
    // ----- pad (held notes, beneath the synthesised pad)
    Recipe {
        instrument: Instrument::Pad,
        voice: 0,
        name: "Bowed vibraphone",
        dir: "pad",
        sources: &[Source::new(
            VCSL,
            "Idiophones/Struck Idiophones/Vibraphone/Bowed",
        )],
        attack: 0.6,
        release: 3.0,
        seconds: 0.0,
        fade: 0.0,
        looped: true,
        loop_blend: 1.0,
        blend: 0.0,
        range: None,
        gain: 0.29,
    },
    Recipe {
        instrument: Instrument::Pad,
        voice: 1,
        name: "Bowed psaltery",
        dir: "psaltery",
        sources: &[Source::new(
            VCSL,
            "Chordophones/Zithers/Psaltery, Bowed and Plucked/LongBow",
        )],
        attack: 0.7,
        release: 2.5,
        seconds: 6.0,
        fade: 0.0,
        looped: true,
        loop_blend: 1.2,
        blend: 0.0,
        range: Some((70, 92)),
        gain: 1.06,
    },
    Recipe {
        instrument: Instrument::Pad,
        voice: 2,
        name: "Wine glasses",
        dir: "glasses",
        // Four glasses, bowed: only an octave of notes (the library names
        // them D#4 to D5, with middle C as C3; the notes here are MIDI).
        sources: &[Source::new(
            VCSL,
            "Idiophones/Friction Idiophones/Wine Glasses/Sustains/Slow",
        )],
        attack: 1.0,
        release: 3.0,
        seconds: 7.0,
        fade: 0.0,
        looped: true,
        loop_blend: 1.5,
        blend: 0.0,
        range: Some((75, 86)),
        gain: 0.48,
    },
    Recipe {
        instrument: Instrument::Pad,
        voice: 3,
        name: "Organ",
        dir: "organ",
        sources: &[Source {
            every: 2,
            ..Source::new(
                VCSL,
                "Aerophones/Edge-blown Aerophones/Renaissance Organ/8'",
            )
        }],
        attack: 0.8,
        release: 2.5,
        seconds: 5.0,
        fade: 0.0,
        looped: true,
        loop_blend: 1.2,
        blend: 0.0,
        range: None,
        gain: 0.17,
    },
    // ----- drums
    Recipe {
        instrument: Instrument::Drums,
        voice: 0,
        name: "Hand drums",
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
        loop_blend: 0.0,
        blend: 0.0,
        range: None,
        gain: 0.92,
    },
];

//! Recorded instruments for the Generator: an instrument pack on disk (a
//! folder of audio files and a `pack.json` saying which note each is), and
//! the [`Multisample`]s loaded from it.
//!
//! Loading reads and decodes files, so it happens off the audio thread;
//! the mixer is handed a finished [`Multisample`] in an [`Arc`] and plays
//! it without allocating (`ambience/sampler.rs`). An instrument with no
//! samples keeps its synthesised voice.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ambience::Instrument;
use crate::clip::decode_file;

/// The pack format this build reads.
pub const PACK_VERSION: u32 = 1;
/// The manifest's name inside a pack.
pub const MANIFEST: &str = "pack.json";

/// A pack's manifest: what each instrument's files are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub instruments: Vec<InstrumentManifest>,
}

/// One instrument in a pack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstrumentManifest {
    pub instrument: Instrument,
    /// Seconds to rise at a note's start, and to fall once a held note is
    /// let go (struck notes play out).
    pub attack: f32,
    pub release: f32,
    /// The whole instrument's gain against its synthesised voice.
    pub gain: f32,
    pub zones: Vec<ZoneManifest>,
}

/// One recorded note and the notes and velocities it plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZoneManifest {
    /// Relative to the pack.
    pub file: String,
    /// The MIDI note recorded, and the lowest and highest it plays,
    /// shifted in pitch.
    pub root: u8,
    pub low: u8,
    pub high: u8,
    /// The velocities it plays, 0 to 1, low inclusive, high exclusive
    /// (1 inclusive).
    pub velocity: (f32, f32),
    /// Where a held note loops, in frames; the pack has made the join
    /// seamless.
    #[serde(default)]
    pub looped: Option<(u64, u64)>,
    #[serde(default = "one")]
    pub gain: f32,
    /// False for unpitched sounds (a shaker), played as recorded.
    #[serde(default = "yes")]
    pub pitched: bool,
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

/// A recorded note, ready to play: interleaved stereo 16-bit at `rate`.
pub struct Zone {
    pub(crate) data: Box<[i16]>,
    pub(crate) frames: usize,
    pub(crate) rate: u32,
    pub(crate) root: u8,
    pub(crate) low: u8,
    pub(crate) high: u8,
    pub(crate) velocity: (f32, f32),
    pub(crate) looped: Option<(usize, usize)>,
    pub(crate) gain: f32,
    pub(crate) pitched: bool,
}

/// An instrument's recorded notes, decoded into memory.
pub struct Multisample {
    pub(crate) instrument: Instrument,
    pub(crate) zones: Vec<Zone>,
    pub(crate) attack: f32,
    pub(crate) release: f32,
    pub(crate) gain: f32,
}

impl Multisample {
    pub fn instrument(&self) -> Instrument {
        self.instrument
    }

    pub fn zones(&self) -> usize {
        self.zones.len()
    }

    /// Memory the samples take.
    pub fn bytes(&self) -> usize {
        self.zones.iter().map(|zone| zone.data.len() * 2).sum()
    }
}

/// An instrument pack on disk.
#[derive(Clone, Debug)]
pub struct Pack {
    dir: PathBuf,
    manifest: Manifest,
}

impl Pack {
    /// Reads the manifest in `dir`.
    pub fn open(dir: &Path) -> Result<Self, String> {
        let path = dir.join(MANIFEST);
        let text =
            std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        let manifest: Manifest =
            serde_json::from_str(&text).map_err(|err| format!("{}: {err}", path.display()))?;
        if manifest.version != PACK_VERSION {
            return Err(format!(
                "{}: version {}, but this Olio reads version {PACK_VERSION}",
                path.display(),
                manifest.version
            ));
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            manifest,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The instruments the pack has.
    pub fn instruments(&self) -> impl Iterator<Item = Instrument> + '_ {
        self.manifest
            .instruments
            .iter()
            .map(|instrument| instrument.instrument)
    }

    /// Decodes an instrument's notes; `None` if the pack has none for it.
    pub fn load(&self, instrument: Instrument) -> Result<Option<Multisample>, String> {
        let Some(entry) = self
            .manifest
            .instruments
            .iter()
            .find(|entry| entry.instrument == instrument)
        else {
            return Ok(None);
        };
        let mut zones = Vec::with_capacity(entry.zones.len());
        for zone in &entry.zones {
            let path = self.dir.join(&zone.file);
            let clip = decode_file(&path).map_err(|err| format!("{}: {err}", path.display()))?;
            let channels = usize::from(clip.channels().max(1));
            let frames = clip.frames();
            let to_i16 = |sample: f32| (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            let mut data = Vec::with_capacity(frames * 2);
            for frame in clip.samples().chunks_exact(channels) {
                let left = frame[0];
                let right = if channels > 1 { frame[1] } else { frame[0] };
                data.push(to_i16(left));
                data.push(to_i16(right));
            }
            let looped = zone.looped.and_then(|(start, end)| {
                let (start, end) = (start as usize, (end as usize).min(frames));
                (start + 4 < end).then_some((start, end))
            });
            zones.push(Zone {
                data: data.into_boxed_slice(),
                frames,
                rate: clip.rate(),
                root: zone.root,
                low: zone.low,
                high: zone.high,
                velocity: zone.velocity,
                looped,
                gain: zone.gain,
                pitched: zone.pitched,
            });
        }
        Ok(Some(Multisample {
            instrument,
            zones,
            attack: entry.attack,
            release: entry.release,
            gain: entry.gain,
        }))
    }
}

/// The recorded instruments the Generator plays, one slot per
/// [`Instrument`]; empty slots keep their synthesised voice.
#[derive(Clone, Default)]
pub struct Recorded(pub [Option<Arc<Multisample>>; Instrument::COUNT]);

impl Recorded {
    pub fn get(&self, instrument: Instrument) -> Option<&Arc<Multisample>> {
        self.0[instrument.index()].as_ref()
    }

    pub fn set(&mut self, instrument: Instrument, samples: Option<Arc<Multisample>>) {
        self.0[instrument.index()] = samples;
    }

    /// Memory all of them take.
    pub fn bytes(&self) -> usize {
        self.0.iter().flatten().map(|set| set.bytes()).sum()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{Clip, WavFormat};

    /// Writes a pack to `dir` whose harp is sine waves recorded at A3 and
    /// A5 (one soft and one loud each), whose strings are one long A3
    /// with a loop, and whose drums are a shaker.
    pub(crate) fn write_test_pack(dir: &Path) {
        let rate = 44_100;
        let tone = |hz: f32, seconds: f32, level: f32, decay: bool| {
            let frames = (seconds * rate as f32) as usize;
            let samples: Vec<f32> = (0..frames)
                .flat_map(|i| {
                    let t = i as f32 / rate as f32;
                    let envelope = if decay { (-t * 2.0).exp() } else { 1.0 };
                    let s = (t * hz * std::f32::consts::TAU).sin() * level * envelope;
                    [s, s]
                })
                .collect();
            Clip::new(rate, 2, samples)
        };
        std::fs::create_dir_all(dir.join("harp")).unwrap();
        std::fs::create_dir_all(dir.join("strings")).unwrap();
        std::fs::create_dir_all(dir.join("drums")).unwrap();
        let write = |clip: Clip, file: &str| {
            clip.write_wav(&dir.join(file), WavFormat::Int16).unwrap();
        };
        write(tone(220.0, 2.0, 0.3, true), "harp/A3_p.wav");
        write(tone(220.0, 2.0, 0.6, true), "harp/A3_f.wav");
        write(tone(880.0, 2.0, 0.5, true), "harp/A5.wav");
        write(tone(220.0, 1.0, 0.5, false), "strings/A3.wav");
        write(tone(3000.0, 0.2, 0.4, true), "drums/shaker.wav");
        let zone = |file: &str, root: u8, low: u8, high: u8, velocity: (f32, f32)| ZoneManifest {
            file: file.to_owned(),
            root,
            low,
            high,
            velocity,
            looped: None,
            gain: 1.0,
            pitched: true,
        };
        let manifest = Manifest {
            version: PACK_VERSION,
            instruments: vec![
                InstrumentManifest {
                    instrument: Instrument::Harp,
                    attack: 0.002,
                    release: 0.3,
                    gain: 1.0,
                    zones: vec![
                        zone("harp/A3_p.wav", 57, 0, 69, (0.0, 0.5)),
                        zone("harp/A3_f.wav", 57, 0, 69, (0.5, 1.0)),
                        zone("harp/A5.wav", 81, 70, 127, (0.0, 1.0)),
                    ],
                },
                InstrumentManifest {
                    instrument: Instrument::Strings,
                    attack: 0.1,
                    release: 0.5,
                    gain: 1.0,
                    zones: vec![ZoneManifest {
                        // Whole cycles of 220 Hz: a seamless loop.
                        looped: Some((22_050 - 4_410, 22_050 + 4_410 * 3)),
                        ..zone("strings/A3.wav", 57, 0, 127, (0.0, 1.0))
                    }],
                },
                InstrumentManifest {
                    instrument: Instrument::Drums,
                    attack: 0.0,
                    release: 0.05,
                    gain: 1.0,
                    zones: vec![ZoneManifest {
                        pitched: false,
                        ..zone("drums/shaker.wav", 120, 120, 120, (0.0, 1.0))
                    }],
                },
            ],
        };
        std::fs::write(
            dir.join(MANIFEST),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn the_mixer_plays_recordings_and_hands_old_ones_back_to_be_freed() {
        use crate::Engine;
        use crate::ambience::{AmbienceSettings, Background, Density, Layer, Style};
        let dir = tempfile::tempdir().unwrap();
        write_test_pack(dir.path());
        let pack = Pack::open(dir.path()).unwrap();
        let harp = Arc::new(pack.load(Instrument::Harp).unwrap().unwrap());
        let (mut engine, mut mixer) = Engine::offline(48_000);
        engine.set_instrument(Instrument::Harp, Some(harp.clone()));
        let mut settings = AmbienceSettings {
            enabled: true,
            background: Background::None,
            ..Style::RainStudy.settings()
        };
        for instrument in Instrument::ALL {
            settings.layers[instrument].enabled = false;
        }
        settings.layers[Instrument::Harp] = Layer::on(0.8, Density::LIVELY);
        engine.set_ambience(settings);
        let mut out = vec![0.0; 48_000 * 2];
        let mut peak = 0.0f32;
        for _ in 0..8 {
            mixer.render(&mut out, 2);
            peak = out.iter().fold(peak, |peak, s| peak.max(s.abs()));
        }
        assert!(peak > 0.0, "the recorded harp plays");
        assert_eq!(Arc::strong_count(&harp), 2, "ours and the mixer's");

        engine.set_instrument(Instrument::Harp, None);
        mixer.render(&mut out, 2);
        engine.poll();
        assert_eq!(Arc::strong_count(&harp), 1, "handed back and freed here");
    }

    #[test]
    fn a_pack_loads_its_instruments_and_leaves_the_rest_synthesised() {
        let dir = tempfile::tempdir().unwrap();
        write_test_pack(dir.path());
        let pack = Pack::open(dir.path()).unwrap();
        let mut instruments: Vec<Instrument> = pack.instruments().collect();
        instruments.sort_by_key(|i| i.index());
        assert_eq!(
            instruments,
            [Instrument::Harp, Instrument::Strings, Instrument::Drums]
        );
        let harp = pack.load(Instrument::Harp).unwrap().unwrap();
        assert_eq!(harp.zones(), 3);
        assert_eq!(harp.zones[0].frames, 88_200);
        assert_eq!(harp.bytes(), (88_200 * 2 * 2) * 3);
        assert!(pack.load(Instrument::Piano).unwrap().is_none());
        let strings = pack.load(Instrument::Strings).unwrap().unwrap();
        assert_eq!(strings.zones[0].looped, Some((17_640, 35_280)));

        // A pack from a newer Olio is refused with a reason.
        let manifest = dir.path().join(MANIFEST);
        let newer = std::fs::read_to_string(&manifest)
            .unwrap()
            .replace("\"version\": 1", "\"version\": 99");
        std::fs::write(&manifest, newer).unwrap();
        assert!(Pack::open(dir.path()).unwrap_err().contains("version 99"));
    }
}

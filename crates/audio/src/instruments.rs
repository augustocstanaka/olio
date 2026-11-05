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

/// The pack format this build writes, and the oldest it still reads.
///
/// Version 2 added to each instrument its voice (a pack can hold several
/// recordings of one role, say a harp, a folk harp and a kalimba for the
/// Generator's harp), its name, licence and credit, a velocity crossfade,
/// and the notes it plays well. A version 1 pack has one voice of each.
pub const PACK_VERSION: u32 = 2;
pub const OLDEST_PACK_VERSION: u32 = 1;
/// The manifest's name inside a pack.
pub const MANIFEST: &str = "pack.json";

/// A pack's manifest: what each instrument's files are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub instruments: Vec<InstrumentManifest>,
}

/// One instrument in a pack: one voice of a role.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstrumentManifest {
    pub instrument: Instrument,
    /// Which voice of the role this is: 0 is the role's own, the others
    /// are chosen by a layer's `voice`.
    #[serde(default)]
    pub voice: u8,
    /// What the voice is called ("Folk harp"); empty for the role's name.
    #[serde(default)]
    pub name: String,
    /// The recordings' licence (an SPDX name) and who to credit; both
    /// required in a pack Olio builds, empty in a version 1 pack.
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub credit: String,
    /// How far, in velocity, neighbouring layers blend into each other
    /// either side of the line between them: 0 switches at the line.
    #[serde(default)]
    pub blend: f32,
    /// The notes it plays at their own pitch; one outside is moved by
    /// octaves to the nearest inside, rather than stretched far from any
    /// recording. `None` plays every note from the nearest recording.
    #[serde(default)]
    pub range: Option<(u8, u8)>,
    /// Seconds to rise at a note's start, and to fall once a held note is
    /// let go (struck notes play out).
    pub attack: f32,
    pub release: f32,
    /// The whole instrument's gain against its synthesised voice.
    pub gain: f32,
    pub zones: Vec<ZoneManifest>,
}

impl InstrumentManifest {
    /// A voice 0 of `instrument` with no name, licence or blend, as a
    /// version 1 pack had.
    pub fn new(
        instrument: Instrument,
        attack: f32,
        release: f32,
        gain: f32,
        zones: Vec<ZoneManifest>,
    ) -> Self {
        Self {
            instrument,
            voice: 0,
            name: String::new(),
            license: String::new(),
            credit: String::new(),
            blend: 0.0,
            range: None,
            attack,
            release,
            gain,
            zones,
        }
    }
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
    /// How far the recording is from its root, in cents, as measured;
    /// played corrected.
    #[serde(default)]
    pub tune: f32,
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
    pub(crate) tune: f32,
}

/// An instrument's recorded notes, decoded into memory.
pub struct Multisample {
    pub(crate) instrument: Instrument,
    pub(crate) voice: u8,
    pub(crate) name: String,
    pub(crate) zones: Vec<Zone>,
    pub(crate) attack: f32,
    pub(crate) release: f32,
    pub(crate) gain: f32,
    pub(crate) blend: f32,
    pub(crate) range: Option<(u8, u8)>,
}

impl Multisample {
    pub fn instrument(&self) -> Instrument {
        self.instrument
    }

    /// Which voice of its role these recordings are.
    pub fn voice(&self) -> u8 {
        self.voice
    }

    /// What the voice is called; empty for the role's own name.
    pub fn name(&self) -> &str {
        &self.name
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
        if !(OLDEST_PACK_VERSION..=PACK_VERSION).contains(&manifest.version) {
            return Err(format!(
                "{}: version {}, but this Olio reads versions {OLDEST_PACK_VERSION} to \
                 {PACK_VERSION}",
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

    /// The instruments the pack has, each once however many voices.
    pub fn instruments(&self) -> impl Iterator<Item = Instrument> + '_ {
        let mut seen = [false; Instrument::COUNT];
        self.manifest.instruments.iter().filter_map(move |entry| {
            let new = !seen[entry.instrument.index()];
            seen[entry.instrument.index()] = true;
            new.then_some(entry.instrument)
        })
    }

    /// The voices the pack has of an instrument, by number, each with its
    /// name (empty for the role's own).
    pub fn voices(&self, instrument: Instrument) -> impl Iterator<Item = (u8, &str)> + '_ {
        self.manifest
            .instruments
            .iter()
            .filter(move |entry| entry.instrument == instrument)
            .map(|entry| (entry.voice, entry.name.as_str()))
    }

    /// The voice to play when `wanted` is asked for: that one if the pack
    /// has it, else the role's own (0), else the first it has; `None` if
    /// the pack has no recording of the instrument.
    pub fn voice_for(&self, instrument: Instrument, wanted: u8) -> Option<u8> {
        let mut voices = self.voices(instrument).map(|(voice, _)| voice);
        let first = voices.next()?;
        let has = |voice: u8| self.voices(instrument).any(|(v, _)| v == voice);
        [wanted, 0].into_iter().find(|v| has(*v)).or(Some(first))
    }

    /// Decodes a voice of an instrument; `None` if the pack has none.
    pub fn load(&self, instrument: Instrument, voice: u8) -> Result<Option<Multisample>, String> {
        let Some(entry) = self
            .manifest
            .instruments
            .iter()
            .find(|entry| entry.instrument == instrument && entry.voice == voice)
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
                tune: zone.tune,
            });
        }
        // Neighbouring layers can blend no further than halfway into each
        // other, or three would sound at once.
        let narrowest = zones
            .iter()
            .map(|zone| zone.velocity.1 - zone.velocity.0)
            .filter(|width| *width > 0.0)
            .fold(1.0f32, f32::min);
        Ok(Some(Multisample {
            instrument,
            voice,
            name: entry.name.clone(),
            zones,
            attack: entry.attack,
            release: entry.release,
            gain: entry.gain,
            blend: entry.blend.clamp(0.0, narrowest / 2.0),
            range: entry.range,
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

    /// The voice loaded for an instrument, if any.
    pub fn voice(&self, instrument: Instrument) -> Option<u8> {
        self.get(instrument).map(|samples| samples.voice())
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
            tune: 0.0,
        };
        let manifest = Manifest {
            version: PACK_VERSION,
            instruments: vec![
                InstrumentManifest::new(
                    Instrument::Harp,
                    0.002,
                    0.3,
                    1.0,
                    vec![
                        zone("harp/A3_p.wav", 57, 0, 69, (0.0, 0.5)),
                        zone("harp/A3_f.wav", 57, 0, 69, (0.5, 1.0)),
                        zone("harp/A5.wav", 81, 70, 127, (0.0, 1.0)),
                    ],
                ),
                InstrumentManifest::new(
                    Instrument::Strings,
                    0.1,
                    0.5,
                    1.0,
                    vec![ZoneManifest {
                        // Whole cycles of 220 Hz: a seamless loop.
                        looped: Some((22_050 - 4_410, 22_050 + 4_410 * 3)),
                        ..zone("strings/A3.wav", 57, 0, 127, (0.0, 1.0))
                    }],
                ),
                InstrumentManifest::new(
                    Instrument::Drums,
                    0.0,
                    0.05,
                    1.0,
                    vec![ZoneManifest {
                        pitched: false,
                        ..zone("drums/shaker.wav", 120, 120, 120, (0.0, 1.0))
                    }],
                ),
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
        let harp = Arc::new(pack.load(Instrument::Harp, 0).unwrap().unwrap());
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

    /// A pack of two voices of the harp, and a version 1 pack's bare
    /// manifest text to read beside it.
    fn write_voices_pack(dir: &Path) {
        write_test_pack(dir);
        let text = std::fs::read_to_string(dir.join(MANIFEST)).unwrap();
        let mut manifest: Manifest = serde_json::from_str(&text).unwrap();
        let mut folk = manifest.instruments[0].clone();
        folk.voice = 1;
        folk.name = "Folk harp".to_owned();
        folk.license = "CC0-1.0".to_owned();
        folk.credit = "Somebody".to_owned();
        folk.blend = 0.9;
        folk.range = Some((57, 81));
        manifest.instruments.insert(1, folk);
        std::fs::write(
            dir.join(MANIFEST),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn a_pack_holds_several_voices_of_a_role_and_falls_back_to_the_roles_own() {
        let dir = tempfile::tempdir().unwrap();
        write_voices_pack(dir.path());
        let pack = Pack::open(dir.path()).unwrap();
        let voices: Vec<(u8, &str)> = pack.voices(Instrument::Harp).collect();
        assert_eq!(voices, [(0, ""), (1, "Folk harp")]);
        assert_eq!(pack.voices(Instrument::Piano).count(), 0);
        let mut instruments: Vec<Instrument> = pack.instruments().collect();
        instruments.sort_by_key(|i| i.index());
        assert_eq!(
            instruments,
            [Instrument::Harp, Instrument::Strings, Instrument::Drums],
            "each instrument once, however many voices"
        );
        // The voice asked for, else the role's own, else nothing.
        assert_eq!(pack.voice_for(Instrument::Harp, 1), Some(1));
        assert_eq!(pack.voice_for(Instrument::Harp, 5), Some(0));
        assert_eq!(pack.voice_for(Instrument::Strings, 1), Some(0));
        assert_eq!(pack.voice_for(Instrument::Piano, 0), None);

        let own = pack.load(Instrument::Harp, 0).unwrap().unwrap();
        let folk = pack.load(Instrument::Harp, 1).unwrap().unwrap();
        assert_eq!((own.voice(), own.name()), (0, ""));
        assert_eq!((folk.voice(), folk.name()), (1, "Folk harp"));
        assert_eq!(folk.range, Some((57, 81)));
        assert!(pack.load(Instrument::Harp, 2).unwrap().is_none());
        let mut recorded = Recorded::default();
        assert_eq!(recorded.voice(Instrument::Harp), None);
        recorded.set(Instrument::Harp, Some(Arc::new(folk)));
        assert_eq!(recorded.voice(Instrument::Harp), Some(1));
    }

    #[test]
    fn layers_blend_no_further_than_halfway_into_each_other() {
        let dir = tempfile::tempdir().unwrap();
        write_voices_pack(dir.path());
        let pack = Pack::open(dir.path()).unwrap();
        // The harp has two layers, each half wide: a blend of 0.9 asked
        // for, a quarter allowed.
        let folk = pack.load(Instrument::Harp, 1).unwrap().unwrap();
        assert!((folk.blend - 0.25).abs() < 1e-6, "{}", folk.blend);
        let own = pack.load(Instrument::Harp, 0).unwrap().unwrap();
        assert_eq!(own.blend, 0.0);
    }

    #[test]
    fn a_version_1_pack_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        write_test_pack(dir.path());
        // Written as version 1 was: no voice, name, licence, credit, blend
        // or range.
        let text = std::fs::read_to_string(dir.path().join(MANIFEST)).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["version"] = 1.into();
        for entry in value["instruments"].as_array_mut().unwrap() {
            let entry = entry.as_object_mut().unwrap();
            for field in ["voice", "name", "license", "credit", "blend", "range"] {
                entry.remove(field);
            }
        }
        std::fs::write(dir.path().join(MANIFEST), value.to_string()).unwrap();
        let pack = Pack::open(dir.path()).unwrap();
        assert_eq!(pack.manifest().version, 1);
        let harp = pack.load(Instrument::Harp, 0).unwrap().unwrap();
        assert_eq!(
            (harp.voice(), harp.name(), harp.blend, harp.range),
            (0, "", 0.0, None)
        );
        assert_eq!(pack.voice_for(Instrument::Harp, 3), Some(0));
        assert_eq!(pack.manifest().instruments[0].license, "");
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
        let harp = pack.load(Instrument::Harp, 0).unwrap().unwrap();
        assert_eq!(harp.zones(), 3);
        assert_eq!(harp.zones[0].frames, 88_200);
        assert_eq!(harp.bytes(), (88_200 * 2 * 2) * 3);
        assert!(pack.load(Instrument::Piano, 0).unwrap().is_none());
        let strings = pack.load(Instrument::Strings, 0).unwrap().unwrap();
        assert_eq!(strings.zones[0].looped, Some((17_640, 35_280)));

        // A pack from a newer Olio is refused with a reason.
        let manifest = dir.path().join(MANIFEST);
        let newer = std::fs::read_to_string(&manifest)
            .unwrap()
            .replace(&format!("\"version\": {PACK_VERSION}"), "\"version\": 99");
        std::fs::write(&manifest, newer).unwrap();
        assert!(Pack::open(dir.path()).unwrap_err().contains("version 99"));
    }
}

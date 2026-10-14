//! Recorded instruments, played: a fixed pool of voices reading the notes
//! of [`Multisample`]s, shifted in pitch with cubic interpolation, held
//! notes looping. Nothing here allocates or frees.
//!
//! An instrument's samples arrive in an [`Arc`]. When they are replaced
//! (or taken away) the voices still playing the old ones fade out
//! quickly, and the old samples wait in a retiring slot until they have,
//! then go back to the app to be freed off the audio thread
//! ([`Sampler::take_released`]).

use std::sync::Arc;

use super::Instrument;
use super::composer::Event;
use super::dsp::pan_gains;
use crate::instruments::{Multisample, Zone};

/// Voices for every recorded instrument together.
const VOICES: usize = 40;
/// Old sample sets waiting for their voices to fade.
const RETIRING: usize = 4;
/// How fast a voice fades when its samples are replaced or it is stolen.
const QUICK_FADE_SECONDS: f32 = 0.05;
/// A voice this quiet has ended.
const SILENT: f32 = 1e-4;

/// Where a voice's samples are: an instrument's current set, or a
/// retiring one.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot {
    Current(usize),
    Retiring(usize),
}

#[derive(Clone, Copy)]
struct Voice {
    active: bool,
    slot: Slot,
    zone: usize,
    /// In the zone's frames, and how far each output frame moves it.
    position: f64,
    step: f64,
    gain: [f32; 2],
    env: f32,
    attack: f32,
    release: f32,
    /// Output frames left before a held note is let go; `None` plays out.
    hold: Option<u64>,
    releasing: bool,
    /// When it started, to steal the oldest first.
    started: u64,
}

impl Voice {
    const IDLE: Self = Self {
        active: false,
        slot: Slot::Current(0),
        zone: 0,
        position: 0.0,
        step: 1.0,
        gain: [0.0; 2],
        env: 0.0,
        attack: 1.0,
        release: 0.0,
        hold: None,
        releasing: false,
        started: 0,
    };
}

pub(super) struct Sampler {
    rate: u32,
    sets: [Option<Arc<Multisample>>; Instrument::COUNT],
    retiring: [Option<Arc<Multisample>>; RETIRING],
    /// Sets whose voices have faded, for the app to take and free.
    released: [Option<Arc<Multisample>>; RETIRING],
    voices: [Voice; VOICES],
    round_robin: [u32; Instrument::COUNT],
    started: u64,
}

/// One sample of a channel, cubic-interpolated between frames.
#[inline]
fn cubic(data: &[i16], frames: usize, at: usize, frac: f32, channel: usize) -> f32 {
    let get = |frame: usize| f32::from(data[frame.min(frames - 1) * 2 + channel]) / 32768.0;
    let y0 = get(at.saturating_sub(1));
    let (y1, y2, y3) = (get(at), get(at + 1), get(at + 2));
    // Catmull-Rom.
    let a = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
    let b = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c = -0.5 * y0 + 0.5 * y2;
    ((a * frac + b) * frac + c) * frac + y1
}

impl Sampler {
    pub(super) fn new(rate: u32) -> Self {
        Self {
            rate,
            sets: Default::default(),
            retiring: Default::default(),
            released: Default::default(),
            voices: [Voice::IDLE; VOICES],
            round_robin: [0; Instrument::COUNT],
            started: 0,
        }
    }

    /// True if the instrument plays recorded notes.
    pub(super) fn has(&self, instrument: Instrument) -> bool {
        self.sets[instrument.index()].is_some()
    }

    pub(super) fn is_sounding(&self) -> bool {
        self.voices.iter().any(|voice| voice.active)
    }

    /// Takes on an instrument's samples (or none): voices playing the old
    /// ones fade out, and the old ones are released once they have.
    pub(super) fn set(&mut self, instrument: Instrument, samples: Option<Arc<Multisample>>) {
        let index = instrument.index();
        let Some(old) = std::mem::replace(&mut self.sets[index], samples) else {
            return;
        };
        let quick = self.quick_fade();
        let playing = self
            .voices
            .iter()
            .any(|voice| voice.active && voice.slot == Slot::Current(index));
        let free = self.retiring.iter().position(Option::is_none);
        match (playing, free) {
            (true, Some(slot)) => {
                self.retiring[slot] = Some(old);
                for voice in &mut self.voices {
                    if voice.active && voice.slot == Slot::Current(index) {
                        voice.slot = Slot::Retiring(slot);
                        voice.releasing = true;
                        voice.release = voice.release.min(quick);
                    }
                }
            }
            _ => {
                // Nothing playing them, or nowhere to wait: stop at once.
                for voice in &mut self.voices {
                    if voice.slot == Slot::Current(index) {
                        voice.active = false;
                    }
                }
                self.release(old);
            }
        }
    }

    /// A set the app may now free, if any.
    pub(super) fn take_released(&mut self) -> Option<Arc<Multisample>> {
        self.released.iter_mut().find_map(Option::take)
    }

    fn release(&mut self, set: Arc<Multisample>) {
        if let Some(free) = self.released.iter_mut().find(|slot| slot.is_none()) {
            *free = Some(set);
        } else {
            // The app has not taken the others yet; keep it retiring.
            if let Some(free) = self.retiring.iter_mut().find(|slot| slot.is_none()) {
                *free = Some(set);
            } else {
                // Every slot full, the app not taking them: a leak rather
                // than freeing on the audio thread.
                std::mem::forget(set);
            }
        }
    }

    fn quick_fade(&self) -> f32 {
        (-6.9 / (QUICK_FADE_SECONDS * self.rate as f32)).exp()
    }

    /// Plays a note on a recorded instrument at `velocity` (already scaled
    /// by its layer's level).
    pub(super) fn play(&mut self, event: Event, velocity: f32) {
        let note = event.note;
        let index = note.instrument.index();
        let Some(set) = &self.sets[index] else {
            return;
        };
        let raw = event.note.velocity.clamp(0.0, 1.0);
        let Some(zone_index) = pick(set, note.midi, raw, self.round_robin[index]) else {
            return;
        };
        self.round_robin[index] = self.round_robin[index].wrapping_add(1);
        let zone = &set.zones[zone_index];
        let semitones = if zone.pitched {
            f32::from(note.midi) - f32::from(zone.root)
        } else {
            0.0
        };
        let step =
            f64::from(zone.rate) / f64::from(self.rate) * 2f64.powf(f64::from(semitones) / 12.0);
        let (left, right) = pan_gains(note.pan);
        let level = velocity * zone.gain * set.gain;
        let attack_frames = set.attack.max(0.0005) * self.rate as f32;
        let attack = 1.0 - (-1.0 / attack_frames).exp();
        let release = (-6.9 / (set.release.max(0.01) * self.rate as f32)).exp();
        let hold = (note.duration > 0).then_some(note.duration);
        let voice = Voice {
            active: true,
            slot: Slot::Current(index),
            zone: zone_index,
            position: 0.0,
            step,
            gain: [level * left, level * right],
            env: 0.0,
            attack,
            release,
            hold,
            releasing: false,
            started: self.started,
        };
        self.started += 1;
        let free = self.voices.iter().position(|voice| !voice.active);
        let at = free.unwrap_or_else(|| {
            // The oldest, which is the quietest more often than not.
            self.voices
                .iter()
                .enumerate()
                .min_by_key(|(_, voice)| voice.started)
                .map_or(0, |(at, _)| at)
        });
        self.voices[at] = voice;
    }

    /// The next frame of every recorded voice, at `gain`.
    #[inline]
    pub(super) fn next(&mut self, gain: f32) -> (f32, f32) {
        let (mut l, mut r) = (0.0, 0.0);
        for voice in &mut self.voices {
            if !voice.active {
                continue;
            }
            let set = match voice.slot {
                Slot::Current(index) => self.sets[index].as_deref(),
                Slot::Retiring(slot) => self.retiring[slot].as_deref(),
            };
            let Some(zone) = set.and_then(|set| set.zones.get(voice.zone)) else {
                voice.active = false;
                continue;
            };
            let Some((left, right)) = voice_next(voice, zone) else {
                voice.active = false;
                continue;
            };
            l += left;
            r += right;
        }
        // Retiring sets whose voices have all gone are released.
        for slot in 0..RETIRING {
            if self.retiring[slot].is_some()
                && !self
                    .voices
                    .iter()
                    .any(|voice| voice.active && voice.slot == Slot::Retiring(slot))
                && let Some(set) = self.retiring[slot].take()
            {
                self.release(set);
            }
        }
        (l * gain, r * gain)
    }
}

/// The zone for a note: among those whose notes and velocities take it,
/// one in turn; failing that, the nearest by note.
fn pick(set: &Multisample, midi: u8, velocity: f32, turn: u32) -> Option<usize> {
    let fits = |zone: &Zone| {
        midi >= zone.low
            && midi <= zone.high
            && velocity >= zone.velocity.0
            && (velocity < zone.velocity.1 || zone.velocity.1 >= 1.0)
    };
    let count = set.zones.iter().filter(|zone| fits(zone)).count();
    if count > 0 {
        let choice = turn as usize % count;
        return set
            .zones
            .iter()
            .enumerate()
            .filter(|(_, zone)| fits(zone))
            .nth(choice)
            .map(|(index, _)| index);
    }
    set.zones
        .iter()
        .enumerate()
        .min_by_key(|(_, zone)| (i32::from(zone.root) - i32::from(midi)).unsigned_abs())
        .map(|(index, _)| index)
}

/// Moves a voice on one frame; `None` once it has ended.
#[inline]
fn voice_next(voice: &mut Voice, zone: &Zone) -> Option<(f32, f32)> {
    if voice.releasing {
        voice.env *= voice.release;
        if voice.env < SILENT {
            return None;
        }
    } else {
        voice.env += (1.0 - voice.env) * voice.attack;
        if let Some(hold) = &mut voice.hold {
            if *hold == 0 {
                voice.releasing = true;
            } else {
                *hold -= 1;
            }
        }
    }
    if let Some((start, end)) = zone.looped
        && voice.position >= end as f64
    {
        voice.position -= (end - start) as f64;
    }
    let at = voice.position as usize;
    if at + 1 >= zone.frames {
        return None;
    }
    let frac = (voice.position - at as f64) as f32;
    let left = cubic(&zone.data, zone.frames, at, frac, 0);
    let right = cubic(&zone.data, zone.frames, at, frac, 1);
    voice.position += voice.step;
    let env = voice.env;
    Some((left * voice.gain[0] * env, right * voice.gain[1] * env))
}

#[cfg(test)]
mod tests {
    use super::super::Note;
    use super::*;
    use crate::instruments::Pack;
    use crate::instruments::tests::write_test_pack;

    fn sampler_with(instrument: Instrument) -> (Sampler, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        write_test_pack(dir.path());
        let pack = Pack::open(dir.path()).unwrap();
        let mut sampler = Sampler::new(48_000);
        let set = pack.load(instrument).unwrap().unwrap();
        sampler.set(instrument, Some(Arc::new(set)));
        (sampler, dir)
    }

    fn event(instrument: Instrument, midi: u8, velocity: f32, duration: u64) -> Event {
        Event {
            at: 0,
            note: Note {
                instrument,
                midi,
                velocity,
                pan: 0.0,
                duration,
            },
        }
    }

    fn crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|p| (p[0] < 0.0) != (p[1] < 0.0))
            .count()
    }

    #[test]
    fn a_recorded_note_plays_at_its_pitch_from_the_nearest_sample() {
        let (mut sampler, _dir) = sampler_with(Instrument::Harp);
        // A4, an octave above the A3 recording: twice its frequency.
        sampler.play(event(Instrument::Harp, 69, 0.3, 0), 0.3);
        let out: Vec<f32> = (0..48_000).map(|_| sampler.next(1.0).0).collect();
        let cycles = crossings(&out[4_800..14_400]) as f32 / 2.0 / 0.2;
        assert!((cycles - 440.0).abs() < 6.0, "{cycles} Hz");
        assert!(sampler.is_sounding());
        // The recording plays out, then the voice ends.
        for _ in 0..48_000 * 2 {
            sampler.next(1.0);
        }
        assert!(!sampler.is_sounding());
    }

    #[test]
    fn velocity_picks_the_layer_and_round_robin_takes_turns() {
        let (sampler, _dir) = sampler_with(Instrument::Harp);
        let set = sampler.sets[Instrument::Harp.index()].as_ref().unwrap();
        assert_eq!(pick(set, 57, 0.2, 0), Some(0), "soft");
        assert_eq!(pick(set, 57, 0.9, 0), Some(1), "loud");
        assert_eq!(pick(set, 81, 1.0, 0), Some(2), "high, at full velocity");
        assert_eq!(pick(set, 127, 0.5, 7), Some(2));
    }

    #[test]
    fn a_held_note_loops_past_its_recording_and_fades_when_let_go() {
        let (mut sampler, _dir) = sampler_with(Instrument::Strings);
        // Held three seconds: three times the one-second recording.
        sampler.play(event(Instrument::Strings, 57, 0.8, 48_000 * 3), 0.8);
        let out: Vec<f32> = (0..48_000 * 4).map(|_| sampler.next(1.0).0).collect();
        let level = |from: usize, to: usize| {
            (out[from..to].iter().map(|s| s * s).sum::<f32>() / (to - from) as f32).sqrt()
        };
        let held = level(48_000 * 2, 48_000 * 2 + 9_600);
        assert!(held > 0.1, "still sounding at two seconds: {held}");
        assert!(
            level(48_000 * 3 + 36_000, 48_000 * 4) < held * 0.05,
            "let go"
        );
    }

    #[test]
    fn replaced_samples_fade_out_then_go_back_to_be_freed() {
        let (mut sampler, _dir) = sampler_with(Instrument::Harp);
        sampler.play(event(Instrument::Harp, 57, 0.6, 0), 0.6);
        for _ in 0..4_800 {
            sampler.next(1.0);
        }
        sampler.set(Instrument::Harp, None);
        assert!(!sampler.has(Instrument::Harp));
        assert!(sampler.take_released().is_none(), "still fading");
        for _ in 0..48_000 / 2 {
            sampler.next(1.0);
        }
        assert!(!sampler.is_sounding());
        let released = sampler.take_released().expect("handed back");
        assert_eq!(released.instrument(), Instrument::Harp);
        assert!(sampler.take_released().is_none());
    }

    #[test]
    fn an_unpitched_sound_plays_as_recorded_whatever_the_note() {
        let (mut sampler, _dir) = sampler_with(Instrument::Drums);
        sampler.play(event(Instrument::Drums, 120, 0.5, 0), 0.5);
        let out: Vec<f32> = (0..2_400).map(|_| sampler.next(1.0).0).collect();
        let hz = crossings(&out) as f32 / 2.0 / 0.05;
        assert!((hz - 3000.0).abs() < 120.0, "{hz} Hz, as recorded");
    }
}

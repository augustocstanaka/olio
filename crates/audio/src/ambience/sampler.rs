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
use super::Rng;
use super::composer::Event;
use super::dsp::{coefficient, pan_gains};
use super::synth::Stems;
use crate::instruments::{Multisample, Zone};

/// Voices for every recorded instrument together, and more where a voice
/// taken for a new note fades out quickly rather than stopping dead.
const VOICES: usize = 40;
const FADING: usize = 8;
/// A recorded note is tuned a few cents either way, so the same note
/// played twice is never quite the same; a held one starts a moment into
/// its recording; and a softer note sounds darker, as a softer stroke
/// does: its tone opens from this, at the softest, to fully open.
const DETUNE_CENTS: f32 = 4.0;
const START_JITTER_SECONDS: f32 = 0.004;
const SOFTEST_TONE_HZ: f32 = 2500.0;
/// Old sample sets waiting for their voices to fade.
const RETIRING: usize = 4;
/// How fast a voice fades when its samples are replaced or it is stolen.
const QUICK_FADE_SECONDS: f32 = 0.05;
/// A voice this quiet has ended.
const SILENT: f32 = 1e-4;
/// How long a carried note takes to reach its new level.
const LEVEL_GLIDE_SECONDS: f32 = 1.2;

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
    /// The layer it plays in, and its note.
    instrument: usize,
    midi: u8,
    /// The pad voicing that struck or last carried it
    /// ([`super::composer::Note::voicing`]); the gain it moves to when
    /// carried on at another velocity, and how fast.
    voicing: u32,
    gain_target: [f32; 2],
    glide: f32,
    /// Its tone, darker as it is played softer: a low-pass per side.
    tone: f32,
    lowpass: [f32; 2],
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
        instrument: 0,
        midi: 0,
        voicing: 0,
        gain_target: [0.0; 2],
        glide: 0.0,
        tone: 1.0,
        lowpass: [0.0; 2],
    };
}

pub(super) struct Sampler {
    rate: u32,
    sets: [Option<Arc<Multisample>>; Instrument::COUNT],
    retiring: [Option<Arc<Multisample>>; RETIRING],
    /// Sets whose voices have faded, for the app to take and free.
    released: [Option<Arc<Multisample>>; RETIRING],
    /// The voices notes are played on, then those fading after being
    /// taken for a new note.
    voices: [Voice; VOICES + FADING],
    rng: Rng,
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
    pub(super) fn new(rate: u32, seed: u64) -> Self {
        Self {
            rate,
            sets: Default::default(),
            retiring: Default::default(),
            released: Default::default(),
            voices: [Voice::IDLE; VOICES + FADING],
            rng: Rng::new(seed),
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
        // The same recordings again: nothing changes. (Dropping this second
        // reference frees nothing; the first is still held.)
        if let (Some(new), Some(old)) = (&samples, &self.sets[index])
            && Arc::ptr_eq(new, old)
        {
            return;
        }
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

    /// Plays a note, or carries on a held one ([`super::composer::Note::from`]):
    /// the same note on a looped recording is held longer, its level
    /// moving to the new one, and counts as struck now (so it is not the
    /// first stolen); otherwise it is played afresh and the held one let
    /// go, since a recording cannot glide and an unlooped one runs out.
    /// A voice struck or carried by this chord already is not carried
    /// again.
    pub(super) fn play_or_carry(&mut self, event: Event, velocity: f32) {
        let note = event.note;
        if let Some(from) = note.from {
            let index = note.instrument.index();
            let held = self.voices.iter().position(|voice| {
                voice.active
                    && !voice.releasing
                    && voice.instrument == index
                    && voice.midi == from
                    && voice.hold.is_some()
                    && voice.voicing != note.voicing
            });
            if let Some(at) = held {
                let looped = match self.voices[at].slot {
                    Slot::Current(index) => self.sets[index].as_deref().and_then(|set| {
                        let zone = set.zones.get(self.voices[at].zone)?;
                        Some((zone.looped.is_some(), zone.gain * set.gain))
                    }),
                    Slot::Retiring(_) => None,
                };
                let voice = &mut self.voices[at];
                if note.duration == 0 {
                    // Let go, nothing struck.
                    voice.hold = Some(0);
                    return;
                }
                match looped {
                    Some((true, gain)) if from == note.midi => {
                        let (left, right) = pan_gains(note.pan);
                        let level = velocity * gain;
                        voice.gain_target = [level * left, level * right];
                        voice.hold = voice.hold.map(|left| left.max(note.duration));
                        voice.voicing = note.voicing;
                        voice.started = self.started;
                        self.started += 1;
                        return;
                    }
                    _ => voice.hold = Some(0),
                }
            }
        }
        if note.from.is_some() && note.duration == 0 {
            return;
        }
        self.play(event, velocity);
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
            f32::from(note.midi) - f32::from(zone.root) - zone.tune / 100.0
        } else {
            0.0
        };
        let cents = if zone.pitched {
            self.rng.range(-DETUNE_CENTS, DETUNE_CENTS)
        } else {
            0.0
        };
        let semitones = semitones + cents / 100.0;
        let step =
            f64::from(zone.rate) / f64::from(self.rate) * 2f64.powf(f64::from(semitones) / 12.0);
        let hold = (note.duration > 0).then_some(note.duration);
        let start = if hold.is_some() {
            f64::from(self.rng.range(0.0, START_JITTER_SECONDS) * zone.rate as f32)
        } else {
            0.0
        };
        let tone_hz = SOFTEST_TONE_HZ * 2f32.powf(3.0 * raw);
        let (left, right) = pan_gains(note.pan);
        let level = velocity * zone.gain * set.gain;
        let attack_frames = set.attack.max(0.0005) * self.rate as f32;
        let attack = 1.0 - (-1.0 / attack_frames).exp();
        let release = (-6.9 / (set.release.max(0.01) * self.rate as f32)).exp();
        let voice = Voice {
            active: true,
            slot: Slot::Current(index),
            zone: zone_index,
            position: start,
            step,
            gain: [level * left, level * right],
            env: 0.0,
            attack,
            release,
            hold,
            releasing: false,
            started: self.started,
            instrument: index,
            midi: note.midi,
            voicing: note.voicing,
            gain_target: [level * left, level * right],
            glide: 1.0 - (-4.0 / (LEVEL_GLIDE_SECONDS * self.rate as f32)).exp(),
            tone: coefficient(tone_hz, self.rate),
            lowpass: [0.0; 2],
        };
        self.started += 1;
        let free = self.voices[..VOICES].iter().position(|voice| !voice.active);
        let at = free.unwrap_or_else(|| {
            // The oldest, which is the quietest more often than not; it
            // fades out on a spare voice, the most nearly silent of them
            // if none is free.
            let oldest = self.voices[..VOICES]
                .iter()
                .enumerate()
                .min_by_key(|(_, voice)| voice.started)
                .map_or(0, |(at, _)| at);
            let spare = VOICES
                + self.voices[VOICES..]
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let loudness = |v: &Voice| if v.active { v.env } else { -1.0 };
                        loudness(a).total_cmp(&loudness(b))
                    })
                    .map_or(0, |(at, _)| at);
            let quick = self.quick_fade();
            let mut fading = self.voices[oldest];
            fading.releasing = true;
            fading.hold = None;
            fading.release = fading.release.min(quick);
            self.voices[spare] = fading;
            oldest
        });
        self.voices[at] = voice;
    }

    /// Adds the next frame of every recorded voice into its layer's stem.
    #[inline]
    pub(super) fn next(&mut self, stems: &mut Stems) {
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
            let stem = &mut stems[voice.instrument];
            stem[0] += left;
            stem[1] += right;
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
    let mut left = cubic(&zone.data, zone.frames, at, frac, 0);
    let mut right = cubic(&zone.data, zone.frames, at, frac, 1);
    for (side, sample) in [&mut left, &mut right].into_iter().enumerate() {
        voice.lowpass[side] += (*sample - voice.lowpass[side]) * voice.tone;
        *sample = voice.lowpass[side];
    }
    voice.position += voice.step;
    for (gain, target) in voice.gain.iter_mut().zip(voice.gain_target) {
        *gain += (target - *gain) * voice.glide;
    }
    let env = voice.env;
    Some((left * voice.gain[0] * env, right * voice.gain[1] * env))
}

#[cfg(test)]
mod tests {
    use super::super::Note;
    use super::*;
    use crate::instruments::Pack;

    /// The next frame's left side, every layer together.
    fn left(sampler: &mut Sampler) -> f32 {
        let mut stems = [[0.0; 2]; Instrument::COUNT];
        sampler.next(&mut stems);
        stems.iter().map(|stem| stem[0]).sum()
    }
    use crate::instruments::tests::write_test_pack;

    fn sampler_with(instrument: Instrument) -> (Sampler, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        write_test_pack(dir.path());
        let pack = Pack::open(dir.path()).unwrap();
        let mut sampler = Sampler::new(48_000, 1);
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
                from: None,
                voicing: 0,
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
        let out: Vec<f32> = (0..48_000).map(|_| left(&mut sampler)).collect();
        let cycles = crossings(&out[4_800..14_400]) as f32 / 2.0 / 0.2;
        assert!((cycles - 440.0).abs() < 6.0, "{cycles} Hz");
        assert!(sampler.is_sounding());
        // The recording plays out, then the voice ends.
        for _ in 0..48_000 * 2 {
            left(&mut sampler);
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
        let out: Vec<f32> = (0..48_000 * 4).map(|_| left(&mut sampler)).collect();
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
    fn a_carried_note_holds_on_where_it_loops_and_is_struck_again_where_not() {
        let carried = |instrument: Instrument, midi: u8| {
            let (mut sampler, dir) = sampler_with(instrument);
            sampler.play_or_carry(event(instrument, midi, 0.6, 48_000 * 3), 0.6);
            // The next chord, 1.6 s on (the harp's two-second recording
            // still sounding).
            for _ in 0..76_800 {
                left(&mut sampler);
            }
            let started = sampler.voices.iter().find(|v| v.active).unwrap().started;
            let mut again = event(instrument, midi, 0.3, 48_000 * 4);
            again.at = 76_800;
            again.note.from = Some(midi);
            again.note.voicing = 1;
            sampler.play_or_carry(again, 0.3);
            (sampler, started, dir)
        };
        // Looped strings: one voice, held longer, its level falling to the
        // new one and counted as struck now.
        let (mut sampler, started, _dir) = carried(Instrument::Strings, 57);
        let playing: Vec<&Voice> = sampler.voices.iter().filter(|v| v.active).collect();
        assert_eq!(playing.len(), 1);
        assert!(playing[0].hold.unwrap() > 48_000 * 3);
        assert!(playing[0].started > started, "not the first stolen");
        let before = playing[0].gain[0];
        for _ in 0..48_000 * 2 {
            left(&mut sampler);
        }
        let after = sampler.voices.iter().find(|v| v.active).unwrap().gain[0];
        assert!((after / before - 0.5).abs() < 0.01, "{before} {after}");
        // An unlooped harp note would run out: struck again, the old let go.
        let (sampler, _, _dir) = carried(Instrument::Harp, 57);
        let fresh = sampler
            .voices
            .iter()
            .filter(|v| v.active && v.hold == Some(48_000 * 4))
            .count();
        assert_eq!(fresh, 1);
    }

    #[test]
    fn a_voice_taken_for_a_new_note_fades_out_rather_than_stopping() {
        let (mut sampler, _dir) = sampler_with(Instrument::Strings);
        for i in 0..VOICES {
            sampler.play(
                event(Instrument::Strings, 45 + (i % 24) as u8, 0.6, 48_000 * 10),
                0.6,
            );
        }
        for _ in 0..4_800 {
            left(&mut sampler);
        }
        let oldest = sampler.voices[..VOICES]
            .iter()
            .map(|v| v.started)
            .min()
            .unwrap();
        let taken = sampler.voices[..VOICES]
            .iter()
            .find(|v| v.started == oldest)
            .unwrap()
            .midi;
        sampler.play(event(Instrument::Strings, 80, 0.6, 48_000 * 10), 0.6);
        let fading: Vec<&Voice> = sampler.voices[VOICES..]
            .iter()
            .filter(|v| v.active)
            .collect();
        assert_eq!(fading.len(), 1);
        assert!(fading[0].releasing && fading[0].midi == taken);
        // Fading, not stopped: still heard 10 ms on, gone by 200 ms.
        for _ in 0..480 {
            left(&mut sampler);
        }
        assert!(
            sampler.voices[VOICES..]
                .iter()
                .any(|v| v.active && v.env > 0.01)
        );
        for _ in 0..9_600 {
            left(&mut sampler);
        }
        assert!(sampler.voices[VOICES..].iter().all(|v| !v.active));
        assert_eq!(
            sampler.voices[..VOICES].iter().filter(|v| v.active).count(),
            VOICES
        );
    }

    #[test]
    fn the_same_note_twice_is_never_quite_the_same_and_softer_is_darker() {
        let (mut sampler, _dir) = sampler_with(Instrument::Strings);
        sampler.play(event(Instrument::Strings, 57, 0.2, 48_000), 0.2);
        sampler.play(event(Instrument::Strings, 57, 0.9, 48_000), 0.9);
        let (soft, loud) = (sampler.voices[0], sampler.voices[1]);
        let exact = 44_100.0 / 48_000.0;
        let cents = |step: f64| 1200.0 * (step / exact).log2();
        assert!(soft.step != loud.step, "detuned apart");
        for voice in [soft, loud] {
            assert!(cents(voice.step).abs() <= f64::from(DETUNE_CENTS) + 0.01);
            assert!(
                voice.position > 0.0
                    && voice.position <= f64::from(START_JITTER_SECONDS * 44_100.0),
                "held: a moment in"
            );
        }
        assert!(soft.tone < loud.tone, "softer, darker");
        // A struck note starts at its start.
        let (mut sampler, _dir) = sampler_with(Instrument::Harp);
        sampler.play(event(Instrument::Harp, 57, 0.5, 0), 0.5);
        assert_eq!(sampler.voices[0].position, 0.0);
    }

    #[test]
    fn replaced_samples_fade_out_then_go_back_to_be_freed() {
        let (mut sampler, _dir) = sampler_with(Instrument::Harp);
        sampler.play(event(Instrument::Harp, 57, 0.6, 0), 0.6);
        for _ in 0..4_800 {
            left(&mut sampler);
        }
        sampler.set(Instrument::Harp, None);
        assert!(!sampler.has(Instrument::Harp));
        assert!(sampler.take_released().is_none(), "still fading");
        for _ in 0..48_000 / 2 {
            left(&mut sampler);
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
        let out: Vec<f32> = (0..2_400).map(|_| left(&mut sampler)).collect();
        let hz = crossings(&out) as f32 / 2.0 / 0.05;
        assert!((hz - 3000.0).abs() < 120.0, "{hz} Hz, as recorded");
    }
}

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
const VOICES: usize = 48;
const FADING: usize = 12;
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
    /// How much of the note it is, 1 unless the note's velocity fell
    /// between two layers that blend.
    weight: f32,
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
        weight: 1.0,
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

/// One sample of a channel, cubic-interpolated between frames; in a loop,
/// the frames after its end are those after its start, as the pack baked
/// them to join.
#[inline]
fn cubic(
    data: &[i16],
    frames: usize,
    looped: Option<(usize, usize)>,
    at: usize,
    frac: f32,
    channel: usize,
) -> f32 {
    let get = |frame: usize| {
        let frame = match looped {
            Some((start, end)) if frame >= end => start + (frame - end) % (end - start),
            _ => frame.min(frames - 1),
        };
        f32::from(data[frame * 2 + channel]) / 32768.0
    };
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

    /// The most semitones any sounding voice is pitched away from its
    /// recording, for tests.
    #[cfg(test)]
    pub(super) fn greatest_stretch(&self) -> f32 {
        self.voices
            .iter()
            .filter(|voice| voice.active)
            .filter_map(|voice| match voice.slot {
                Slot::Current(set) => self.sets[set]
                    .as_deref()
                    .and_then(|set| set.zones.get(voice.zone))
                    .filter(|zone| zone.pitched)
                    .map(|zone| {
                        let ratio = voice.step * f64::from(self.rate) / f64::from(zone.rate);
                        (12.0 * ratio.log2()).abs() as f32
                    }),
                Slot::Retiring(_) => None,
            })
            .fold(0.0, f32::max)
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
    /// A note may sound on two voices (two layers blending), and all are
    /// carried or let go together. A voice struck or carried by this
    /// voicing already is not carried again.
    pub(super) fn play_or_carry(&mut self, event: Event, velocity: f32) {
        let note = event.note;
        if let Some(from) = note.from {
            let index = note.instrument.index();
            let mut held = [usize::MAX; 4];
            let mut count = 0;
            for (at, voice) in self.voices.iter().enumerate() {
                if voice.active
                    && !voice.releasing
                    && voice.instrument == index
                    && voice.midi == from
                    && voice.hold.is_some()
                    && voice.voicing != note.voicing
                    && count < held.len()
                {
                    held[count] = at;
                    count += 1;
                }
            }
            if count > 0 {
                let held = &held[..count];
                let looped = held.iter().all(|at| {
                    let voice = &self.voices[*at];
                    match voice.slot {
                        Slot::Current(set) => self.sets[set]
                            .as_deref()
                            .and_then(|set| set.zones.get(voice.zone))
                            .is_some_and(|zone| zone.looped.is_some()),
                        Slot::Retiring(_) => false,
                    }
                });
                let (left, right) = pan_gains(note.pan);
                let hold_on = looped && from == note.midi && note.duration > 0;
                for &at in held {
                    let current = self.voices[at].slot;
                    let gain = match current {
                        Slot::Current(set) => self.sets[set].as_deref().and_then(|set| {
                            let zone = set.zones.get(self.voices[at].zone)?;
                            Some(zone.gain * set.gain)
                        }),
                        Slot::Retiring(_) => None,
                    };
                    let voice = &mut self.voices[at];
                    if hold_on && let Some(gain) = gain {
                        let level = velocity * gain * voice.weight;
                        voice.gain_target = [level * left, level * right];
                        voice.hold = voice.hold.map(|left| left.max(note.duration));
                        voice.voicing = note.voicing;
                        voice.started = self.started;
                        self.started += 1;
                    } else {
                        voice.hold = Some(0);
                    }
                }
                if hold_on {
                    return;
                }
            }
            if note.duration == 0 {
                // Let go, nothing struck.
                return;
            }
        }
        self.play(event, velocity);
    }

    /// Plays a note on a recorded instrument at `velocity` (already scaled
    /// by its layer's level): a voice for each layer its velocity falls in,
    /// two where it falls between layers that blend.
    pub(super) fn play(&mut self, event: Event, velocity: f32) {
        let note = event.note;
        let index = note.instrument.index();
        let Some(set) = &self.sets[index] else {
            return;
        };
        let raw = event.note.velocity.clamp(0.0, 1.0);
        let midi = fold(note.midi, set.range);
        let chosen = picks(set, midi, raw, self.round_robin[index]);
        if chosen.count == 0 {
            return;
        }
        self.round_robin[index] = self.round_robin[index].wrapping_add(1);
        let hold = (note.duration > 0).then_some(note.duration);
        let mut voices = [None; 2];
        for (slot, &(zone_index, weight)) in chosen.zones[..chosen.count].iter().enumerate() {
            let zone = &set.zones[zone_index];
            let semitones = if zone.pitched {
                f32::from(midi) - f32::from(zone.root) - zone.tune / 100.0
            } else {
                0.0
            };
            let cents = if zone.pitched {
                self.rng.range(-DETUNE_CENTS, DETUNE_CENTS)
            } else {
                0.0
            };
            let semitones = semitones + cents / 100.0;
            let step = f64::from(zone.rate) / f64::from(self.rate)
                * 2f64.powf(f64::from(semitones) / 12.0);
            let start = if hold.is_some() {
                f64::from(self.rng.range(0.0, START_JITTER_SECONDS) * zone.rate as f32)
            } else {
                0.0
            };
            let tone_hz = SOFTEST_TONE_HZ * 2f32.powf(3.0 * raw);
            let (left, right) = pan_gains(note.pan);
            let level = velocity * zone.gain * set.gain * weight;
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
                weight,
            };
            self.started += 1;
            voices[slot] = Some(voice);
        }
        for voice in voices.into_iter().flatten() {
            self.place(voice);
        }
    }

    /// Puts a voice on a free one; a full pool takes the oldest, which
    /// fades out on a spare voice.
    fn place(&mut self, voice: Voice) {
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

/// The zones a note plays, and how much of it each does.
struct Picks {
    zones: [(usize, f32); 2],
    count: usize,
}

/// How much of a layer a velocity is: 1 inside it, falling to 0 over
/// `blend` either side of its edges, as the square root of a cross-fade
/// so that two neighbours together keep the power (the lowest layer has
/// nothing below it to blend with, nor the highest above it). With no
/// blend, 1 inside and 0 outside.
fn layer_weight((low, high): (f32, f32), velocity: f32, blend: f32) -> f32 {
    use std::f32::consts::FRAC_PI_2;
    if blend <= 0.0 {
        let inside = velocity >= low && (velocity < high || high >= 1.0);
        return f32::from(inside);
    }
    let rise = if low <= 0.0 {
        1.0
    } else {
        (((velocity - (low - blend)) / (2.0 * blend)).clamp(0.0, 1.0) * FRAC_PI_2).sin()
    };
    let fall = if high >= 1.0 {
        1.0
    } else {
        (((velocity - (high - blend)) / (2.0 * blend)).clamp(0.0, 1.0) * FRAC_PI_2).cos()
    };
    rise * fall
}

/// A note moved by octaves into `range`, if it has one and the note is
/// outside it; where no octave of it fits (a range under an octave wide
/// that lacks its pitch class), the octave nearest the range.
fn fold(midi: u8, range: Option<(u8, u8)>) -> u8 {
    let Some((low, high)) = range else {
        return midi;
    };
    let (low, high) = (i32::from(low), i32::from(high));
    let outside = |note: i32| (low - note).max(note - high).max(0);
    (-10..=10)
        .map(|octave| i32::from(midi) + 12 * octave)
        .filter(|note| (0..=127).contains(note))
        .min_by_key(|note| (outside(*note), (note - i32::from(midi)).abs()))
        .unwrap_or(i32::from(midi)) as u8
}

/// How many semitones further the recording of the layer a velocity calls
/// for may be than the nearest of any layer before the nearest is used.
const FALLBACK_MARGIN: u32 = 5;

/// The zones for a note: for each layer whose velocities take it, one of
/// the zones of that layer for that note, in turn (round robin), with
/// the layer's weight. Where only one layer has a recording near the note
/// (the packer stops a layer's last recordings a fifth beyond their
/// roots) that one plays at whatever velocity, at full weight, since
/// there is nothing to blend with. Where none has, the nearest recording
/// of a layer the velocity calls for, else the nearest of all.
fn picks(set: &Multisample, midi: u8, velocity: f32, turn: u32) -> Picks {
    let mut picks = Picks {
        zones: [(0, 0.0); 2],
        count: 0,
    };
    let covers = |zone: &Zone| midi >= zone.low && midi <= zone.high;
    let first = |index: usize, zone: &Zone| {
        covers(zone)
            && !set.zones[..index]
                .iter()
                .any(|other| covers(other) && other.velocity == zone.velocity)
    };
    let layers = set
        .zones
        .iter()
        .enumerate()
        .filter(|(index, zone)| first(*index, zone))
        .count();
    for (index, zone) in set.zones.iter().enumerate() {
        if picks.count == picks.zones.len() || !first(index, zone) {
            continue;
        }
        let weight = if layers == 1 {
            1.0
        } else {
            layer_weight(zone.velocity, velocity, set.blend)
        };
        if weight <= 0.0 {
            continue;
        }
        let same = |other: &&Zone| covers(other) && other.velocity == zone.velocity;
        let takes = set.zones.iter().filter(|other| same(other)).count();
        let choice = turn as usize % takes;
        if let Some((chosen, _)) = set
            .zones
            .iter()
            .enumerate()
            .filter(|(_, other)| same(other))
            .nth(choice)
        {
            picks.zones[picks.count] = (chosen, weight);
            picks.count += 1;
        }
    }
    if picks.count == 0 {
        let distance = |zone: &Zone| (i32::from(zone.root) - i32::from(midi)).unsigned_abs();
        let wanted = |zone: &Zone| layer_weight(zone.velocity, velocity, set.blend) > 0.0;
        let nearest_of = |called: bool| {
            set.zones
                .iter()
                .enumerate()
                .filter(|(_, zone)| !called || wanted(zone))
                .min_by_key(|(_, zone)| distance(zone))
        };
        // The layer the velocity calls for, unless another layer's
        // recording is much nearer (a stretch is worse than the timbre).
        let nearest = match (nearest_of(true), nearest_of(false)) {
            (Some(called), Some(any))
                if distance(&set.zones[called.0])
                    > distance(&set.zones[any.0]) + FALLBACK_MARGIN =>
            {
                Some(any)
            }
            (Some(called), _) => Some(called),
            (None, any) => any,
        };
        if let Some((nearest, _)) = nearest {
            picks.zones[0] = (nearest, 1.0);
            picks.count = 1;
        }
    }
    picks
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
        // Round the loop, however far past its end a step took it.
        voice.position = start as f64 + (voice.position - end as f64) % (end - start) as f64;
    }
    let at = voice.position as usize;
    // A struck note ends with its recording; a held one goes round.
    if zone.looped.is_none() && at + 1 >= zone.frames {
        return None;
    }
    let frac = (voice.position - at as f64) as f32;
    let mut left = cubic(&zone.data, zone.frames, zone.looped, at, frac, 0);
    let mut right = cubic(&zone.data, zone.frames, zone.looped, at, frac, 1);
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
        let set = pack.load(instrument, 0).unwrap().unwrap();
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
        let first = |midi, velocity, turn| {
            let picks = picks(set, midi, velocity, turn);
            assert_eq!(picks.count, 1);
            picks.zones[0].0
        };
        assert_eq!(first(57, 0.2, 0), 0, "soft");
        assert_eq!(first(57, 0.9, 0), 1, "loud");
        assert_eq!(first(81, 1.0, 0), 2, "high, at full velocity");
        assert_eq!(first(127, 0.5, 7), 2);
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

    /// A set of one sine zone per layer, each layer a velocity range (and
    /// `copies` zones of it, as round robins), blending by `blend`.
    fn layered(layers: &[(f32, f32)], copies: usize, blend: f32, looped: bool) -> Multisample {
        let rate = 44_100;
        let mut zones = Vec::new();
        for (layer, &velocity) in layers.iter().enumerate() {
            for copy in 0..copies {
                let hz = 220.0 + 20.0 * (layer * copies + copy) as f32;
                let data: Vec<i16> = (0..rate * 2)
                    .flat_map(|i| {
                        let t = i as f32 / rate as f32;
                        let s = (t * hz * std::f32::consts::TAU).sin() * 0.5;
                        let s = (s * 32767.0) as i16;
                        [s, s]
                    })
                    .collect();
                zones.push(Zone {
                    data: data.into_boxed_slice(),
                    frames: rate * 2,
                    rate: rate as u32,
                    root: 57,
                    low: 0,
                    high: 127,
                    velocity,
                    looped: looped.then_some((rate / 2, rate * 3 / 2)),
                    gain: 1.0,
                    pitched: true,
                    tune: 0.0,
                });
            }
        }
        Multisample {
            instrument: Instrument::Harp,
            voice: 0,
            name: String::new(),
            zones,
            attack: 0.001,
            release: 0.3,
            gain: 1.0,
            blend,
            range: None,
        }
    }

    const FOUR: [(f32, f32); 4] = [(0.0, 0.25), (0.25, 0.5), (0.5, 0.75), (0.75, 1.0)];

    #[test]
    fn neighbouring_layers_blend_with_equal_power() {
        let set = layered(&FOUR, 1, 0.1, false);
        for step in 0..=1000 {
            let velocity = step as f32 / 1000.0;
            let chosen = picks(&set, 57, velocity, 0);
            let power: f32 = chosen.zones[..chosen.count]
                .iter()
                .map(|(_, w)| w * w)
                .sum();
            assert!(
                (power - 1.0).abs() < 1e-4,
                "velocity {velocity}: power {power}"
            );
            assert!(chosen.count <= 2);
        }
        // Inside a layer, away from its edges: one voice at full weight.
        let chosen = picks(&set, 57, 0.375, 0);
        assert_eq!((chosen.count, chosen.zones[0]), (1, (1, 1.0)));
        // On a line between two: both, half the power each.
        let chosen = picks(&set, 57, 0.5, 0);
        assert_eq!(chosen.count, 2);
        assert_eq!((chosen.zones[0].0, chosen.zones[1].0), (1, 2));
        for (_, weight) in &chosen.zones[..2] {
            assert!((weight * weight - 0.5).abs() < 1e-4);
        }
        // Softer than the softest and louder than the loudest: the end layers.
        assert_eq!(picks(&set, 57, 0.0, 0).zones[0], (0, 1.0));
        assert_eq!(picks(&set, 57, 1.0, 0).zones[0], (3, 1.0));
        // Without a blend the lines switch, as before.
        let hard = layered(&FOUR, 1, 0.0, false);
        let chosen = picks(&hard, 57, 0.5, 0);
        assert_eq!((chosen.count, chosen.zones[0]), (1, (2, 1.0)));
        let chosen = picks(&hard, 57, 0.4999, 0);
        assert_eq!(chosen.zones[0], (1, 1.0));
    }

    /// A set of two layers (soft 0 to 0.5, loud 0.5 to 1) with a blend,
    /// the soft one covering notes 40 to 70 and the loud 50 to 80, roots
    /// at the middle of each.
    fn uneven_layers() -> Multisample {
        let mut set = layered(&FOUR[..0], 1, 0.1, false);
        let zone = |velocity: (f32, f32), low: u8, high: u8, root: u8| {
            let mut z = layered(&FOUR[..1], 1, 0.0, false).zones.remove(0);
            z.velocity = velocity;
            z.low = low;
            z.high = high;
            z.root = root;
            z
        };
        set.zones = vec![zone((0.0, 0.5), 40, 70, 55), zone((0.5, 1.0), 50, 80, 65)];
        set
    }

    #[test]
    fn a_note_only_one_layer_covers_plays_it_at_full_weight_at_any_velocity() {
        let set = uneven_layers();
        // 45 is only in the soft layer, 75 only in the loud: at every
        // velocity, one voice at weight 1, never a dip inside the blend.
        for step in 0..=100 {
            let velocity = step as f32 / 100.0;
            for (midi, zone) in [(45u8, 0usize), (75, 1)] {
                let chosen = picks(&set, midi, velocity, 0);
                assert_eq!(chosen.count, 1, "note {midi} at {velocity}");
                assert_eq!(chosen.zones[0], (zone, 1.0), "note {midi} at {velocity}");
            }
        }
        // Where both cover (50 to 70) they blend as before.
        let chosen = picks(&set, 60, 0.5, 0);
        assert_eq!(chosen.count, 2);
        assert_eq!(picks(&set, 60, 0.2, 0).zones[0], (0, 1.0));
        assert_eq!(picks(&set, 60, 0.9, 0).zones[0], (1, 1.0));
    }

    #[test]
    fn a_note_no_layer_covers_comes_from_a_layer_the_velocity_calls_for() {
        // Roots 55 (soft) and 65 (loud), covering 40 to 70 and 50 to 80.
        let set = uneven_layers();
        // 30: below both. The loud recording is 35 away, the soft one 25:
        // much nearer, so the soft one, at any velocity.
        assert_eq!(picks(&set, 30, 0.9, 0).zones[0], (0, 1.0));
        // 90: 25 from the loud, 35 from the soft: the loud, even at piano.
        assert_eq!(picks(&set, 90, 0.2, 0).zones[0], (1, 1.0));
        assert_eq!(picks(&set, 90, 0.9, 0).zones[0], (1, 1.0));
        // With roots close (62 and 65) and a note at 100 (38 and 35 away)
        // the layer the velocity calls for plays: soft at piano, loud at
        // forte, neither stretching more than the other by much.
        let mut close = uneven_layers();
        close.zones[0].root = 62;
        assert_eq!(picks(&close, 100, 0.2, 0).zones[0], (0, 1.0));
        assert_eq!(picks(&close, 100, 0.9, 0).zones[0], (1, 1.0));
        // A set with one layer is as it was: the nearest.
        let mut one = uneven_layers();
        one.zones.truncate(1);
        assert_eq!(picks(&one, 100, 0.9, 0).zones[0], (0, 1.0));
    }

    #[test]
    fn a_note_between_two_layers_sounds_both_and_round_robins_take_turns_within_each() {
        let set = layered(&FOUR[..2], 2, 0.1, false);
        // Zones 0 and 1 are the soft layer's two takes, 2 and 3 the next.
        let at = |turn| {
            let chosen = picks(&set, 57, 0.25, turn);
            (chosen.zones[0].0, chosen.zones[1].0)
        };
        assert_eq!(
            (at(0), at(1), at(2), at(3)),
            ((0, 2), (1, 3), (0, 2), (1, 3))
        );

        let mut sampler = Sampler::new(48_000, 1);
        sampler.set(Instrument::Harp, Some(Arc::new(set)));
        sampler.play(event(Instrument::Harp, 57, 0.3, 0), 0.8);
        let sounding: Vec<&Voice> = sampler.voices.iter().filter(|v| v.active).collect();
        assert_eq!(sounding.len(), 2, "two layers, two voices");
        let power: f32 = sounding.iter().map(|v| v.weight * v.weight).sum();
        assert!((power - 1.0).abs() < 1e-4);
        // Level follows weight: the louder weight, the louder voice.
        let (a, b) = (sounding[0], sounding[1]);
        assert_eq!(a.gain[0] > b.gain[0], a.weight > b.weight);
        // A note inside a layer is one voice.
        let mut sampler = Sampler::new(48_000, 1);
        sampler.set(
            Instrument::Harp,
            Some(Arc::new(layered(&FOUR, 1, 0.1, false))),
        );
        sampler.play(event(Instrument::Harp, 57, 0.1, 0), 0.8);
        assert_eq!(sampler.voices.iter().filter(|v| v.active).count(), 1);
    }

    #[test]
    fn a_note_outside_a_voices_range_moves_by_octaves_into_it() {
        assert_eq!(fold(60, None), 60);
        assert_eq!(fold(60, Some((55, 80))), 60, "inside: as played");
        assert_eq!(fold(36, Some((55, 80))), 60);
        assert_eq!(fold(100, Some((55, 80))), 76);
        // A range of an octave less a semitone or wider holds every note.
        for midi in 0..=127u8 {
            let folded = fold(midi, Some((63, 74)));
            assert!((63..=74).contains(&folded), "{midi} -> {folded}");
            assert_eq!(folded % 12, midi % 12, "the same pitch class");
        }
        // Narrower, lacking the pitch class: the octave nearest the range.
        assert_eq!(fold(30, Some((60, 64))), 66);
        assert_eq!(fold(53, Some((60, 64))), 65);
        assert_eq!(fold(62, Some((60, 64))), 62);

        let mut set = layered(&FOUR[..1], 1, 0.0, false);
        set.range = Some((55, 80));
        let mut sampler = Sampler::new(44_100, 1);
        sampler.set(Instrument::Harp, Some(Arc::new(set)));
        // Played at 36, heard an octave and a half up at 60: three octaves
        // would be 36 below the recording's root; 60 is 3 above.
        sampler.play(event(Instrument::Harp, 36, 0.5, 0), 0.8);
        let step = sampler.voices.iter().find(|v| v.active).unwrap().step;
        let cents = 1200.0 * step.log2();
        assert!(
            (cents - 300.0).abs() < f64::from(DETUNE_CENTS) + 0.1,
            "{cents}"
        );
        // The note it was asked for is still the one carried on.
        assert_eq!(sampler.voices.iter().find(|v| v.active).unwrap().midi, 36);
    }

    #[test]
    fn a_blended_held_note_carries_and_lets_go_on_both_its_voices() {
        let mut sampler = Sampler::new(48_000, 1);
        sampler.set(
            Instrument::Harp,
            Some(Arc::new(layered(&FOUR[..2], 1, 0.1, true))),
        );
        let mut first = event(Instrument::Harp, 57, 0.25, 48_000 * 5);
        first.note.voicing = 1;
        sampler.play_or_carry(first, 0.8);
        assert_eq!(sampler.voices.iter().filter(|v| v.active).count(), 2);
        let mut again = first;
        again.at = 48_000 * 2;
        again.note.from = Some(57);
        again.note.voicing = 2;
        again.note.duration = 48_000 * 9;
        sampler.play_or_carry(again, 0.4);
        let voices: Vec<&Voice> = sampler.voices.iter().filter(|v| v.active).collect();
        assert_eq!(voices.len(), 2, "carried, nothing struck");
        for voice in &voices {
            assert!(voice.hold.unwrap() > 48_000 * 8, "held longer");
            assert_eq!(voice.voicing, 2);
        }
        // Each keeps its weight in the new level.
        let ratio = voices[0].gain_target[0] / voices[1].gain_target[0];
        assert!((ratio - voices[0].weight / voices[1].weight).abs() < 1e-3);
        // Let go: both.
        let mut gone = again;
        gone.note.from = Some(57);
        gone.note.voicing = 3;
        gone.note.duration = 0;
        sampler.play_or_carry(gone, 0.0);
        assert!(
            sampler
                .voices
                .iter()
                .filter(|v| v.active)
                .all(|v| v.hold == Some(0))
        );
        assert_eq!(
            sampler.voices.iter().filter(|v| v.active).count(),
            2,
            "nothing struck"
        );
    }

    #[test]
    fn a_loop_that_ends_with_its_recording_goes_round_and_round_unbroken() {
        // As the packer bakes them: the data stops where the loop does.
        // 441 Hz is a hundred frames a cycle: a loop of 176 whole cycles,
        // so a join that continues the tone leaves no jump.
        let rate = 44_100usize;
        let hz = 441.0f32;
        let frames = 22_000;
        let data: Vec<i16> = (0..frames)
            .flat_map(|i| {
                let s = ((i as f32 * hz / rate as f32 * std::f32::consts::TAU).sin()
                    * 0.5
                    * 32767.0) as i16;
                [s, s]
            })
            .collect();
        let mut set = layered(&FOUR[..1], 1, 0.0, false);
        set.zones[0] = Zone {
            data: data.into_boxed_slice(),
            frames,
            rate: rate as u32,
            root: 57,
            low: 0,
            high: 127,
            velocity: (0.0, 1.0),
            looped: Some((4_400, frames)),
            gain: 1.0,
            pitched: true,
            tune: 0.0,
        };
        set.attack = 0.001;
        for (name, midi) in [
            ("at the root", 57u8),
            ("an octave down", 45),
            ("a fifth up", 64),
        ] {
            let mut sampler = Sampler::new(rate as u32, 1);
            sampler.set(Instrument::Harp, Some(Arc::new(set_clone(&set))));
            // Held ten seconds: twenty-five passes round the loop.
            sampler.play(event(Instrument::Harp, midi, 0.8, rate as u64 * 10), 0.8);
            let out: Vec<f32> = (0..rate * 9).map(|_| left(&mut sampler)).collect();
            let rms = |from: usize, to: usize| {
                (out[from..to].iter().map(|s| s * s).sum::<f32>() / (to - from) as f32).sqrt()
            };
            let (early, late) = (rms(rate, rate * 2), rms(rate * 8, rate * 9));
            assert!(
                late > early * 0.9 && late < early * 1.1,
                "{name}: {early} {late}"
            );
            // No click anywhere: the biggest step is about a sine's own.
            let step = out
                .windows(2)
                .map(|p| (p[1] - p[0]).abs())
                .fold(0.0f32, f32::max);
            let own = rms(rate, rate * 2)
                * 1.5
                * (std::f32::consts::TAU * hz * 2f32.powf((f32::from(midi) - 57.0) / 12.0)
                    / rate as f32);
            assert!(
                step < own * 1.3 + 1e-4,
                "{name}: a step of {step}, a sine's own is {own}"
            );
        }
    }

    /// A copy of a set, for a test to play from more than one sampler.
    fn set_clone(set: &Multisample) -> Multisample {
        Multisample {
            instrument: set.instrument,
            voice: set.voice,
            name: set.name.clone(),
            zones: set
                .zones
                .iter()
                .map(|zone| Zone {
                    data: zone.data.clone(),
                    ..*zone
                })
                .collect(),
            attack: set.attack,
            release: set.release,
            gain: set.gain,
            blend: set.blend,
            range: set.range,
        }
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

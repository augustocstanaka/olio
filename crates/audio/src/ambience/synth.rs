//! The instruments. Each is a small pool of voices, all allocated when the
//! synth is built; rendering a frame allocates nothing.

use std::f32::consts::{FRAC_PI_4, TAU};

use super::composer::Event;
use super::{AmbienceSettings, Instrument, Rng};
use crate::mixer::Ramp;

/// Equal-power panning.
#[inline]
fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    (angle.cos(), angle.sin())
}

fn hz(midi: u8) -> f32 {
    440.0 * 2f32.powf((f32::from(midi) - 69.0) / 12.0)
}

/// A one-pole low-pass coefficient for `hz` at `rate`.
fn coefficient(hz: f32, rate: u32) -> f32 {
    let x = (-TAU * hz / rate as f32).exp();
    (1.0 - x).clamp(0.0, 1.0)
}

// ----- plucked ---------------------------------------------------------

/// A plucked string: a delay line fed back through a gentle low-pass
/// (Karplus-Strong), read with a fractional delay so it is in tune.
pub(super) struct Plucked {
    line: Vec<f32>,
    /// Frames of delay for the pitch, fractional.
    length: f32,
    write: usize,
    /// Feedback, applied every frame, so very close to 1: 0.99998 loses
    /// 60 dB in about seven seconds at 48 kHz.
    damp: f32,
    /// The loop filter: more of it, the duller the string.
    lowpass: f32,
    tone: f32,
    gain: Ramp,
    pan: f32,
    /// Frames left before the string is let go of.
    life: u64,
    pub(super) active: bool,
}

impl Plucked {
    pub(super) fn new(max_frames: usize) -> Self {
        Self {
            line: vec![0.0; max_frames + 2],
            length: 100.0,
            write: 0,
            damp: 0.9999,
            lowpass: 0.0,
            tone: 0.5,
            gain: Ramp::at(0.0),
            pan: 0.0,
            life: 0,
            active: false,
        }
    }

    pub(super) fn pluck(
        &mut self,
        rng: &mut Rng,
        rate: u32,
        midi: u8,
        velocity: f32,
        pan: f32,
        instrument: Instrument,
    ) {
        let length = (rate as f32 / hz(midi)).clamp(2.0, (self.line.len() - 2) as f32);
        let (damp, tone, seconds, brightness) = match instrument {
            // Long, warm and round.
            Instrument::Bass => (0.99998, 0.25, 9.0, 0.35),
            // Shorter and brighter, with a little ring.
            _ => (0.99996, 0.55, 5.0, 0.85),
        };
        self.damp = damp;
        self.tone = tone;
        // The loop's low-pass delays the signal a little too; take that off
        // the line so the string is in tune.
        self.length = (length - (1.0 - tone) / tone).max(2.0);
        self.lowpass = 0.0;
        self.pan = pan;
        self.life = (seconds * rate as f32) as u64;
        self.active = true;
        self.gain = Ramp::at(velocity * 0.5);
        // Excite the whole line with noise, softened for the bass so the
        // attack is a thump rather than a click.
        let fill = (length.ceil() as usize + 1).min(self.line.len());
        let mut smooth = 0.0f32;
        let mut sum = 0.0f32;
        for sample in &mut self.line[..fill] {
            let noise = rng.signed();
            smooth += (noise - smooth) * brightness;
            *sample = smooth;
            sum += smooth;
        }
        // Without its average the noise would leave a steady offset in
        // the loop that outlives the note.
        let mean = sum / fill as f32;
        for sample in &mut self.line[..fill] {
            *sample -= mean;
        }
        for sample in &mut self.line[fill..] {
            *sample = 0.0;
        }
        // Write on from the end of the excitation, so the read point, a
        // period behind, runs through it.
        self.write = fill % self.line.len();
    }

    /// The next frame as (left, right).
    #[inline]
    pub(super) fn next(&mut self) -> (f32, f32) {
        let len = self.line.len();
        let read = self.write as f32 - self.length + len as f32;
        let index = read.floor();
        let frac = read - index;
        let i0 = index as usize % len;
        let i1 = (i0 + 1) % len;
        let delayed = self.line[i0] * (1.0 - frac) + self.line[i1] * frac;
        self.lowpass += (delayed - self.lowpass) * self.tone;
        let fed = self.lowpass * self.damp;
        self.line[self.write] = fed;
        self.write = (self.write + 1) % len;
        if self.life > 0 {
            self.life -= 1;
            if self.life == 0 {
                self.gain.towards(0.0, 2400);
            }
        } else if self.gain.is_silent() {
            self.active = false;
        }
        let out = fed * self.gain.next();
        let (l, r) = pan_gains(self.pan);
        (out * l, out * r)
    }
}

// ----- bells -------------------------------------------------------------

/// A struck bell: four partials in bell-like ratios, the high ones dying
/// first.
#[derive(Clone, Copy)]
struct Bell {
    phases: [f32; 4],
    steps: [f32; 4],
    amps: [f32; 4],
    decays: [f32; 4],
    attack: f32,
    pan: f32,
    active: bool,
}

const BELL_RATIOS: [f32; 4] = [1.0, 2.0, 2.756, 5.404];
const BELL_AMPS: [f32; 4] = [1.0, 0.5, 0.35, 0.18];
const BELL_SECONDS: [f32; 4] = [3.2, 2.0, 1.3, 0.6];

impl Bell {
    fn new() -> Self {
        Self {
            phases: [0.0; 4],
            steps: [0.0; 4],
            amps: [0.0; 4],
            decays: [1.0; 4],
            attack: 0.0,
            pan: 0.0,
            active: false,
        }
    }

    fn strike(&mut self, rate: u32, midi: u8, velocity: f32, pan: f32) {
        let base = hz(midi);
        for i in 0..4 {
            self.phases[i] = 0.0;
            self.steps[i] = base * BELL_RATIOS[i] / rate as f32;
            self.amps[i] = BELL_AMPS[i] * velocity * 0.35;
            // Each partial's own decay: 60 dB over its seconds.
            self.decays[i] = (-6.9 / (BELL_SECONDS[i] * rate as f32)).exp();
        }
        self.attack = 0.0;
        self.pan = pan;
        self.active = true;
    }

    #[inline]
    fn next(&mut self) -> (f32, f32) {
        let mut out = 0.0;
        let mut loudest = 0.0f32;
        for i in 0..4 {
            out += (self.phases[i] * TAU).sin() * self.amps[i];
            self.phases[i] = (self.phases[i] + self.steps[i]).fract();
            self.amps[i] *= self.decays[i];
            loudest = loudest.max(self.amps[i]);
        }
        // A soft few milliseconds in, so the strike does not click.
        self.attack = (self.attack + 0.005).min(1.0);
        out *= self.attack;
        if loudest < 1e-4 {
            self.active = false;
        }
        let (l, r) = pan_gains(self.pan);
        (out * l, out * r)
    }
}

// ----- sustained ---------------------------------------------------------

/// A saw wave with the step smoothed (PolyBLEP), so it does not alias.
#[inline]
fn saw(phase: f32, step: f32) -> f32 {
    let mut value = 2.0 * phase - 1.0;
    if phase < step {
        let t = phase / step;
        value -= t + t - t * t - 1.0;
    } else if phase > 1.0 - step {
        let t = (phase - 1.0) / step;
        value -= t * t + t + t + 1.0;
    }
    value
}

#[inline]
fn triangle(phase: f32) -> f32 {
    4.0 * (phase - 0.5).abs() - 1.0
}

#[derive(Clone, Copy, PartialEq)]
enum Timbre {
    Strings,
    Pad,
}

/// A bowed or blown note: three detuned oscillators through a low-pass,
/// with a slow rise and a slow fall.
#[derive(Clone, Copy)]
struct Sustained {
    timbre: Timbre,
    phases: [f32; 3],
    steps: [f32; 3],
    vibrato: f32,
    vibrato_step: f32,
    env: f32,
    attack: f32,
    release: f32,
    /// Frames of holding left before the release.
    hold: u64,
    releasing: bool,
    lowpass: f32,
    cutoff: f32,
    lfo: f32,
    lfo_step: f32,
    level: f32,
    pan: f32,
    active: bool,
}

impl Sustained {
    fn new(timbre: Timbre) -> Self {
        Self {
            timbre,
            phases: [0.0; 3],
            steps: [0.0; 3],
            vibrato: 0.0,
            vibrato_step: 0.0,
            env: 0.0,
            attack: 0.0,
            release: 0.0,
            hold: 0,
            releasing: false,
            lowpass: 0.0,
            cutoff: 0.0,
            lfo: 0.0,
            lfo_step: 0.0,
            level: 0.0,
            pan: 0.0,
            active: false,
        }
    }

    fn bow(&mut self, rng: &mut Rng, rate: u32, midi: u8, velocity: f32, pan: f32, hold: u64) {
        let base = hz(midi) / rate as f32;
        let detune = match self.timbre {
            Timbre::Strings => 0.004,
            Timbre::Pad => 0.006,
        };
        self.steps = [base * (1.0 - detune), base, base * (1.0 + detune)];
        for phase in &mut self.phases {
            *phase = rng.unit();
        }
        let (attack, release, cutoff, level) = match self.timbre {
            Timbre::Strings => (1.3, 2.5, 1400.0, 0.22),
            Timbre::Pad => (3.5, 6.0, 520.0, 0.2),
        };
        self.attack = 1.0 - (-1.0 / (attack * rate as f32)).exp();
        // 60 dB down over `release` seconds.
        self.release = (-6.9 / (release * rate as f32)).exp();
        self.env = 0.0;
        self.hold = hold.max((attack * rate as f32) as u64);
        self.releasing = false;
        self.cutoff = coefficient(cutoff, rate);
        self.vibrato_step = 5.2 / rate as f32;
        self.lfo_step = 0.09 / rate as f32;
        self.level = level * velocity;
        self.pan = pan;
        self.active = true;
    }

    #[inline]
    fn next(&mut self) -> (f32, f32) {
        // Envelope: rise towards 1, hold, then fall.
        if self.releasing {
            self.env *= self.release;
            if self.env < 1e-4 {
                self.active = false;
            }
        } else {
            self.env += (1.0 - self.env) * self.attack;
            if self.hold > 0 {
                self.hold -= 1;
            } else {
                self.releasing = true;
            }
        }
        self.vibrato = (self.vibrato + self.vibrato_step).fract();
        self.lfo = (self.lfo + self.lfo_step).fract();
        let bend = 1.0 + 0.0025 * (self.vibrato * TAU).sin();
        let mut out = 0.0;
        for i in 0..3 {
            let step = self.steps[i] * bend;
            out += match self.timbre {
                Timbre::Strings => saw(self.phases[i], step),
                Timbre::Pad => triangle(self.phases[i]),
            };
            self.phases[i] = (self.phases[i] + step).fract();
        }
        // The filter opens a little with the envelope and breathes with
        // the LFO.
        let breath = 0.7 + 0.3 * self.env + 0.15 * (self.lfo * TAU).sin();
        self.lowpass += (out / 3.0 - self.lowpass) * (self.cutoff * breath).clamp(0.0, 1.0);
        let out = self.lowpass * self.env * self.level;
        let (l, r) = pan_gains(self.pan);
        (out * l, out * r)
    }
}

// ----- drone -------------------------------------------------------------

/// The root, held: a few partials and the fifth above, swelling in and out
/// slowly. Two of them, so a change of key crosses over.
#[derive(Clone, Copy)]
struct DroneVoice {
    phases: [f32; 4],
    steps: [f32; 4],
    env: f32,
    target: f32,
    rate_in: f32,
    rate_out: f32,
    breath: f32,
    breath_step: f32,
    active: bool,
}

impl DroneVoice {
    fn new() -> Self {
        Self {
            phases: [0.0; 4],
            steps: [0.0; 4],
            env: 0.0,
            target: 0.0,
            rate_in: 0.0,
            rate_out: 0.0,
            breath: 0.0,
            breath_step: 0.0,
            active: false,
        }
    }

    fn tune(&mut self, rate: u32, midi: u8) {
        let base = hz(midi) / rate as f32;
        self.steps = [base, base * 2.0, base * 3.0, base * 1.5];
        self.rate_in = 1.0 - (-1.0 / (4.0 * rate as f32)).exp();
        self.rate_out = (-4.0 / (4.0 * rate as f32)).exp();
        self.breath_step = 0.045 / rate as f32;
        self.target = 1.0;
        self.active = true;
    }

    fn release(&mut self) {
        self.target = 0.0;
    }

    #[inline]
    fn next(&mut self) -> f32 {
        if self.target > 0.0 {
            self.env += (1.0 - self.env) * self.rate_in;
        } else {
            self.env *= self.rate_out;
            if self.env < 1e-4 {
                self.active = false;
                return 0.0;
            }
        }
        self.breath = (self.breath + self.breath_step).fract();
        let swell = 0.75 + 0.25 * (self.breath * TAU).sin();
        let amps = [1.0, 0.45, 0.18, 0.3];
        let mut out = 0.0;
        for ((phase, step), amp) in self.phases.iter_mut().zip(self.steps).zip(amps) {
            out += (*phase * TAU).sin() * amp;
            *phase = (*phase + step).fract();
        }
        out * self.env * swell * 0.16
    }
}

// ----- the synth ----------------------------------------------------------

const BASS_STRINGS: usize = 3;
const HARP_STRINGS: usize = 8;
const BELLS: usize = 6;
const STRINGS: usize = 4;
const PAD_VOICES: usize = 6;
/// The lowest plucked note, for the delay lines' length.
const LOWEST_HZ: f32 = 24.0;

pub(super) struct Synth {
    rate: u32,
    rng: Rng,
    bass: Vec<Plucked>,
    harp: Vec<Plucked>,
    bells: [Bell; BELLS],
    strings: [Sustained; STRINGS],
    pad: [Sustained; PAD_VOICES],
    drone: [DroneVoice; 2],
    drone_live: usize,
}

impl Synth {
    pub(super) fn new(rate: u32, seed: u64) -> Self {
        let max_frames = (rate as f32 / LOWEST_HZ).ceil() as usize + 2;
        Self {
            rate,
            rng: Rng::new(seed),
            bass: (0..BASS_STRINGS)
                .map(|_| Plucked::new(max_frames))
                .collect(),
            harp: (0..HARP_STRINGS)
                .map(|_| Plucked::new(max_frames / 4))
                .collect(),
            bells: [Bell::new(); BELLS],
            strings: [Sustained::new(Timbre::Strings); STRINGS],
            pad: [Sustained::new(Timbre::Pad); PAD_VOICES],
            drone: [DroneVoice::new(); 2],
            drone_live: 0,
        }
    }

    pub(super) fn is_sounding(&self) -> bool {
        self.bass.iter().chain(&self.harp).any(|s| s.active)
            || self.bells.iter().any(|b| b.active)
            || self.strings.iter().chain(&self.pad).any(|s| s.active)
            || self.drone.iter().any(|d| d.active)
    }

    /// Starts a note. A full pool takes the voice nearest its end.
    pub(super) fn play(&mut self, event: Event, settings: &AmbienceSettings) {
        let note = event.note;
        let layer = settings.layer(note.instrument);
        if !layer.enabled {
            return;
        }
        let velocity = (note.velocity * layer.level).clamp(0.0, 1.0);
        match note.instrument {
            Instrument::Bass | Instrument::Harp => {
                let pool = if note.instrument == Instrument::Bass {
                    &mut self.bass
                } else {
                    &mut self.harp
                };
                let index = pool
                    .iter()
                    .position(|s| !s.active)
                    .unwrap_or_else(|| least(pool.iter().map(|s| s.life)));
                pool[index].pluck(
                    &mut self.rng,
                    self.rate,
                    note.midi,
                    velocity,
                    note.pan,
                    note.instrument,
                );
            }
            Instrument::Bells => {
                let index = self
                    .bells
                    .iter()
                    .position(|b| !b.active)
                    .unwrap_or_else(|| least(self.bells.iter().map(|b| (b.amps[0] * 1e6) as u64)));
                self.bells[index].strike(self.rate, note.midi, velocity, note.pan);
            }
            Instrument::Strings | Instrument::Pad => {
                let pool: &mut [Sustained] = if note.instrument == Instrument::Strings {
                    &mut self.strings
                } else {
                    &mut self.pad
                };
                let index = pool
                    .iter()
                    .position(|s| !s.active)
                    .unwrap_or_else(|| least(pool.iter().map(|s| s.hold)));
                pool[index].bow(
                    &mut self.rng,
                    self.rate,
                    note.midi,
                    velocity,
                    note.pan,
                    note.duration,
                );
            }
            Instrument::Drone => {
                self.drone[self.drone_live].release();
                self.drone_live = 1 - self.drone_live;
                self.drone[self.drone_live].tune(self.rate, note.midi);
            }
        }
    }

    /// The next frame of every instrument, at `gain`.
    #[inline]
    pub(super) fn next(&mut self, gain: f32) -> (f32, f32) {
        let (mut l, mut r) = (0.0, 0.0);
        for string in self.bass.iter_mut().chain(self.harp.iter_mut()) {
            if string.active {
                let (sl, sr) = string.next();
                l += sl;
                r += sr;
            }
        }
        for bell in &mut self.bells {
            if bell.active {
                let (bl, br) = bell.next();
                l += bl;
                r += br;
            }
        }
        for voice in self.strings.iter_mut().chain(self.pad.iter_mut()) {
            if voice.active {
                let (vl, vr) = voice.next();
                l += vl;
                r += vr;
            }
        }
        for drone in &mut self.drone {
            if drone.active {
                let d = drone.next();
                l += d;
                r += d;
            }
        }
        (l * gain, r * gain)
    }
}

/// The index of the smallest value.
fn least(values: impl Iterator<Item = u64>) -> usize {
    values
        .enumerate()
        .min_by_key(|(_, value)| *value)
        .map_or(0, |(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cycles_in(samples: &[f32]) -> f32 {
        let crossings = samples
            .windows(2)
            .filter(|p| (p[0] < 0.0) != (p[1] < 0.0))
            .count();
        crossings as f32 / 2.0
    }

    #[test]
    fn plucked_strings_ring_in_tune_and_die_away() {
        let rate = 48_000;
        let mut rng = Rng::new(5);
        let mut string = Plucked::new(rate as usize / 20);
        string.pluck(&mut rng, rate, 69, 1.0, 0.0, Instrument::Harp);
        let samples: Vec<f32> = (0..rate as usize / 2).map(|_| string.next().0).collect();
        let tail = &samples[samples.len() - rate as usize / 10..];
        let expected = 440.0 / 10.0;
        let cycles = cycles_in(tail);
        assert!(
            (cycles - expected).abs() <= expected * 0.03,
            "{cycles} vs {expected}"
        );
        let early = samples[..4800].iter().map(|s| s.abs()).fold(0.0, f32::max);
        let late = tail.iter().map(|s| s.abs()).fold(0.0, f32::max);
        assert!(late < early * 0.7 && late > 0.0, "{early} then {late}");
    }

    #[test]
    fn bells_strike_then_fade_and_strings_rise_hold_and_fall() {
        let rate = 48_000;
        let mut bell = Bell::new();
        bell.strike(rate, 84, 1.0, 0.0);
        let bell_out: Vec<f32> = (0..rate as usize * 5).map(|_| bell.next().0).collect();
        let peak_at = |from: usize, to: usize| {
            bell_out[from..to]
                .iter()
                .map(|s| s.abs())
                .fold(0.0, f32::max)
        };
        assert!(peak_at(0, 4800) > peak_at(rate as usize, rate as usize + 4800));
        assert!(!bell.active, "gone after five seconds");

        let mut rng = Rng::new(2);
        let mut string = Sustained::new(Timbre::Strings);
        string.bow(&mut rng, rate, 60, 1.0, 0.0, rate as u64 * 2);
        let out: Vec<f32> = (0..rate as usize * 6).map(|_| string.next().0).collect();
        let level = |second: usize| {
            let slice = &out[second * rate as usize..(second + 1) * rate as usize];
            (slice.iter().map(|s| s * s).sum::<f32>() / slice.len() as f32).sqrt()
        };
        assert!(level(0) < level(1), "rising");
        assert!(level(1) > level(4), "fallen after the hold");
        assert!(!string.active, "silent by the end");
        assert!(out.iter().all(|s| s.abs() < 0.5));
    }

    #[test]
    fn the_drone_crosses_over_to_a_new_root() {
        let rate = 48_000;
        let mut synth = Synth::new(rate, 1);
        let settings = {
            let mut s = super::super::Style::Cathedral.settings();
            s.enabled = true;
            s
        };
        let event = |midi| Event {
            at: 0,
            note: super::super::Note {
                instrument: Instrument::Drone,
                midi,
                velocity: 1.0,
                pan: 0.0,
                duration: 0,
            },
        };
        synth.play(event(31), &settings);
        for _ in 0..rate * 6 {
            synth.next(1.0);
        }
        synth.play(event(38), &settings);
        for _ in 0..rate * 2 {
            synth.next(1.0);
        }
        assert!(
            synth.drone.iter().all(|d| d.active),
            "both sound while crossing"
        );
        for _ in 0..rate * 8 {
            synth.next(1.0);
        }
        assert_eq!(synth.drone.iter().filter(|d| d.active).count(), 1);
    }
}

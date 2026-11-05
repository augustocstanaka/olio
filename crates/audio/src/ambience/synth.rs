//! The instruments. Each is a small pool of voices, all allocated when the
//! synth is built; rendering a frame allocates nothing.

use std::f32::consts::TAU;
use std::sync::Arc;

use super::composer::{Event, SHAKER};
use super::dsp::{BandPass, OnePole, coefficient, decay, hz, pan_gains};
use super::sampler::Sampler;
use super::{AmbienceSettings, Instrument, Rng};
use crate::instruments::Multisample;
use crate::mixer::Ramp;

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
            // Pizzicato: bright and short, the string stopped by the finger.
            Instrument::Violin => (0.99975, 0.6, 1.5, 0.95),
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

/// Oscillators in a bowed or blown note: half lean left, half right.
const UNISON: usize = 6;
/// A held note carried on to another at most this far away glides there,
/// over about this long; further, it is let go and the new one bowed.
const GLIDE_SEMITONES: i32 = 2;
const GLIDE_SECONDS: f32 = 1.2;

/// A bowed or blown note: six detuned oscillators, spread across the
/// stereo field, through a low-pass on each side, with a slow rise and a
/// slow fall.
#[derive(Clone, Copy)]
struct Sustained {
    timbre: Timbre,
    phases: [f32; UNISON],
    steps: [f32; UNISON],
    vibrato: f32,
    vibrato_step: f32,
    env: f32,
    attack: f32,
    release: f32,
    /// Frames of holding left before the release.
    hold: u64,
    releasing: bool,
    /// The note it plays, and how far its pitch still is from it while it
    /// glides there (a ratio, settling to 1).
    midi: u8,
    glide: f32,
    glide_rate: f32,
    /// The pad voicing that last struck or carried it
    /// ([`super::Note::voicing`]): a voice that voicing has touched
    /// already is not carried again.
    voicing: u32,
    /// The level it moves to when carried on at another velocity.
    level_target: f32,
    lowpass: [f32; 2],
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
            phases: [0.0; UNISON],
            steps: [0.0; UNISON],
            vibrato: 0.0,
            vibrato_step: 0.0,
            env: 0.0,
            attack: 0.0,
            release: 0.0,
            hold: 0,
            releasing: false,
            midi: 0,
            glide: 1.0,
            glide_rate: 0.0,
            voicing: 0,
            level_target: 0.0,
            lowpass: [0.0; 2],
            cutoff: 0.0,
            lfo: 0.0,
            lfo_step: 0.0,
            level: 0.0,
            pan: 0.0,
            active: false,
        }
    }

    /// Tunes the oscillators to `midi`: the widest of them sharp and
    /// flat, the rest evenly between.
    fn tune(&mut self, rate: u32, midi: u8) {
        let base = hz(midi) / rate as f32;
        let detune = match self.timbre {
            Timbre::Strings => 0.005,
            Timbre::Pad => 0.011,
        };
        for (i, step) in self.steps.iter_mut().enumerate() {
            let offset = i as f32 / (UNISON - 1) as f32 * 2.0 - 1.0;
            *step = base * (1.0 + detune * offset);
        }
        self.midi = midi;
    }

    /// Its level at `velocity`.
    fn level_at(&self, velocity: f32) -> f32 {
        let level = match self.timbre {
            Timbre::Strings => 0.22,
            Timbre::Pad => 0.2,
        };
        level * velocity
    }

    #[allow(clippy::too_many_arguments)]
    fn bow(
        &mut self,
        rng: &mut Rng,
        rate: u32,
        midi: u8,
        velocity: f32,
        pan: f32,
        hold: u64,
        voicing: u32,
    ) {
        self.tune(rate, midi);
        for phase in &mut self.phases {
            *phase = rng.unit();
        }
        let (attack, release, cutoff) = match self.timbre {
            Timbre::Strings => (1.3, 2.5, 1400.0),
            Timbre::Pad => (3.5, 6.0, 520.0),
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
        self.level = self.level_at(velocity);
        self.level_target = self.level;
        self.pan = pan;
        self.voicing = voicing;
        self.glide = 1.0;
        // Nearly there (98 %) after GLIDE_SECONDS.
        self.glide_rate = 1.0 - (-4.0 / (GLIDE_SECONDS * rate as f32)).exp();
        self.active = true;
    }

    /// True if a note of `voicing` carrying on from `from` may carry this
    /// voice on: it sounds that note, held, and no note of the same
    /// voicing has struck or carried it already.
    fn carries(&self, from: u8, voicing: u32) -> bool {
        self.active && !self.releasing && self.midi == from && self.voicing != voicing
    }

    /// Carries a held note on to `midi` at `velocity` for `hold` more
    /// frames: the same note held longer, or one up to a tone away glided
    /// to, its level moving to the new one. False, and the note let go,
    /// when it is further.
    fn carry(&mut self, rate: u32, midi: u8, velocity: f32, hold: u64, voicing: u32) -> bool {
        let semitones = i32::from(midi) - i32::from(self.midi);
        if semitones.abs() > GLIDE_SEMITONES {
            self.hold = 0;
            self.releasing = true;
            return false;
        }
        if semitones != 0 {
            // Tuned there afresh, so glides never add up to an error;
            // heard where it was, then gliding.
            self.glide /= 2f32.powf(semitones as f32 / 12.0);
            self.tune(rate, midi);
        }
        self.level_target = self.level_at(velocity);
        self.hold = self.hold.max(hold);
        self.voicing = voicing;
        true
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
        self.glide += (1.0 - self.glide) * self.glide_rate;
        self.level += (self.level_target - self.level) * self.glide_rate;
        let bend = (1.0 + 0.0025 * (self.vibrato * TAU).sin()) * self.glide;
        // Alternate oscillators to each side, so the note is wide.
        let mut sides = [0.0f32; 2];
        for i in 0..UNISON {
            let step = self.steps[i] * bend;
            sides[i % 2] += match self.timbre {
                Timbre::Strings => saw(self.phases[i], step),
                Timbre::Pad => triangle(self.phases[i]),
            };
            self.phases[i] = (self.phases[i] + step).fract();
        }
        // The filters open a little with the envelope and breathe with
        // the LFO.
        let breath = 0.7 + 0.3 * self.env + 0.15 * (self.lfo * TAU).sin();
        let opening = (self.cutoff * breath).clamp(0.0, 1.0);
        let per_side = (UNISON / 2) as f32;
        for (lowpass, side) in self.lowpass.iter_mut().zip(sides) {
            *lowpass += (side / per_side - *lowpass) * opening;
        }
        let amp = self.env * self.level;
        let (l, r) = pan_gains(self.pan);
        (self.lowpass[0] * amp * l, self.lowpass[1] * amp * r)
    }
}

// ----- drone -------------------------------------------------------------

/// A drone's partials: the root, its octave and twelfth, the fifth, a
/// copy a little sharp that beats slowly against the root, and the octave
/// below, soft.
const DRONE_PARTIALS: usize = 6;
const DRONE_AMPS: [f32; DRONE_PARTIALS] = [1.0, 0.45, 0.18, 0.3, 0.5, 0.25];
/// Where each partial leans, -1 left to 1 right: the root and the octave
/// below in the middle, the rest to either side, so the drone is wide and
/// its slow beating moves across it. The two sides together are the drone
/// heard in the middle.
const DRONE_SPREAD: [f32; DRONE_PARTIALS] = [0.0, -0.4, 0.4, 0.3, -0.5, 0.0];

/// The root, held: a few partials and the fifth above, swelling in and out
/// slowly. Two of them, so a change of key crosses over.
#[derive(Clone, Copy)]
struct DroneVoice {
    phases: [f32; DRONE_PARTIALS],
    steps: [f32; DRONE_PARTIALS],
    /// The layer's level, followed smoothly so a slider never clicks.
    level: f32,
    level_target: f32,
    level_step: f32,
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
            phases: [0.0; DRONE_PARTIALS],
            steps: [0.0; DRONE_PARTIALS],
            level: 0.0,
            level_target: 0.0,
            level_step: 0.0,
            env: 0.0,
            target: 0.0,
            rate_in: 0.0,
            rate_out: 0.0,
            breath: 0.0,
            breath_step: 0.0,
            active: false,
        }
    }

    fn tune(&mut self, rate: u32, midi: u8, level: f32) {
        if !self.active {
            self.level = level;
        }
        self.level_target = level;
        self.level_step = 1.0 - (-1.0 / (0.5 * rate as f32)).exp();
        let base = hz(midi) / rate as f32;
        self.steps = [
            base,
            base * 2.0,
            base * 3.0,
            base * 1.5,
            base * 1.0035,
            base * 0.5,
        ];
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
    fn next(&mut self) -> (f32, f32) {
        if self.target > 0.0 {
            self.env += (1.0 - self.env) * self.rate_in;
        } else {
            self.env *= self.rate_out;
            if self.env < 1e-4 {
                self.active = false;
                return (0.0, 0.0);
            }
        }
        self.breath = (self.breath + self.breath_step).fract();
        self.level += (self.level_target - self.level) * self.level_step;
        let swell = 0.75 + 0.25 * (self.breath * TAU).sin();
        let (mut left, mut right) = (0.0, 0.0);
        let partials = self.phases.iter_mut().zip(self.steps);
        for ((phase, step), (amp, spread)) in partials.zip(DRONE_AMPS.into_iter().zip(DRONE_SPREAD))
        {
            let partial = (*phase * TAU).sin() * amp;
            left += partial * (1.0 - spread);
            right += partial * (1.0 + spread);
            *phase = (*phase + step).fract();
        }
        // At the styles' usual level, about 0.6, as loud as before the
        // drone followed its level.
        let gain = self.env * swell * 0.24 * self.level;
        (left * gain, right * gain)
    }
}

// ----- piano -------------------------------------------------------------

/// Five partials, slightly stretched as a piano string's are, plus a second
/// string a hair sharp so the note beats gently.
const PIANO_PARTIALS: usize = 6;
const PIANO_AMPS: [f32; PIANO_PARTIALS] = [1.0, 0.45, 0.25, 0.12, 0.06, 0.7];
const PIANO_SECONDS: [f32; PIANO_PARTIALS] = [6.0, 4.0, 2.5, 1.5, 0.9, 5.0];
/// How out of tune the partials are: piano strings are stiff.
const PIANO_STIFFNESS: f32 = 0.0004;

/// A struck piano note: partials dying at their own rates, the high ones
/// first, and a soft knock of the hammer.
#[derive(Clone, Copy)]
struct Piano {
    phases: [f32; PIANO_PARTIALS],
    steps: [f32; PIANO_PARTIALS],
    amps: [f32; PIANO_PARTIALS],
    decays: [f32; PIANO_PARTIALS],
    knock: f32,
    knock_decay: f32,
    knock_tone: f32,
    knock_filter: OnePole,
    attack: f32,
    attack_step: f32,
    pan: f32,
    active: bool,
}

impl Piano {
    fn new() -> Self {
        Self {
            phases: [0.0; PIANO_PARTIALS],
            steps: [0.0; PIANO_PARTIALS],
            amps: [0.0; PIANO_PARTIALS],
            decays: [1.0; PIANO_PARTIALS],
            knock: 0.0,
            knock_decay: 0.0,
            knock_tone: 0.0,
            knock_filter: OnePole::default(),
            attack: 0.0,
            attack_step: 0.0,
            pan: 0.0,
            active: false,
        }
    }

    fn strike(&mut self, rate: u32, midi: u8, velocity: f32, pan: f32) {
        let base = hz(midi);
        // High notes die sooner.
        let shorten = (1.0 - (f32::from(midi) - 48.0) / 60.0).clamp(0.4, 1.2);
        for i in 0..PIANO_PARTIALS {
            let (n, beat) = if i == PIANO_PARTIALS - 1 {
                (1.0, 1.0015)
            } else {
                (i as f32 + 1.0, 1.0)
            };
            let stretch = (1.0 + PIANO_STIFFNESS * n * n).sqrt();
            self.steps[i] = (base * n * stretch * beat / rate as f32).min(0.45);
            self.phases[i] = 0.0;
            // Played harder, the upper partials come out more.
            let brightness = if (2..PIANO_PARTIALS - 1).contains(&i) {
                0.4 + 0.6 * velocity
            } else {
                1.0
            };
            // Louder than the rest: a struck note gains least from the hall.
            self.amps[i] = PIANO_AMPS[i] * velocity * brightness * 0.17;
            self.decays[i] = decay(PIANO_SECONDS[i] * shorten, rate);
        }
        self.knock = velocity * 0.17;
        self.knock_decay = decay(0.03, rate);
        self.knock_tone = coefficient(1200.0 + 2000.0 * velocity, rate);
        self.attack = 0.0;
        self.attack_step = 1.0 / (0.002 * rate as f32);
        self.pan = pan;
        self.active = true;
    }

    #[inline]
    fn next(&mut self, rng: &mut Rng) -> (f32, f32) {
        let mut out = 0.0;
        let mut loudest = 0.0f32;
        for i in 0..PIANO_PARTIALS {
            out += (self.phases[i] * TAU).sin() * self.amps[i];
            self.phases[i] = (self.phases[i] + self.steps[i]).fract();
            self.amps[i] *= self.decays[i];
            loudest = loudest.max(self.amps[i]);
        }
        if self.knock > 1e-5 {
            out += self.knock_filter.low(rng.signed(), self.knock_tone) * self.knock;
            self.knock *= self.knock_decay;
        }
        self.attack = (self.attack + self.attack_step).min(1.0);
        out *= self.attack;
        if loudest < 1e-4 && self.knock <= 1e-5 {
            self.active = false;
        }
        let (l, r) = pan_gains(self.pan);
        (out * l, out * r)
    }
}

// ----- drums -------------------------------------------------------------

/// A soft hand drum. A frame drum is a sine whose pitch drops quickly to
/// the note, with a short low slap of the hand; a shaker is a moment of
/// high, band-passed noise.
#[derive(Clone, Copy)]
struct Drum {
    shaker: bool,
    phase: f32,
    step: f32,
    settle: f32,
    glide: f32,
    body: f32,
    body_decay: f32,
    slap: f32,
    slap_decay: f32,
    slap_tone: f32,
    slap_filter: OnePole,
    grains: BandPass,
    rise: f32,
    rise_step: f32,
    pan: f32,
    active: bool,
}

impl Drum {
    fn new() -> Self {
        Self {
            shaker: false,
            phase: 0.0,
            step: 0.0,
            settle: 0.0,
            glide: 0.0,
            body: 0.0,
            body_decay: 0.0,
            slap: 0.0,
            slap_decay: 0.0,
            slap_tone: 0.0,
            slap_filter: OnePole::default(),
            grains: BandPass::default(),
            rise: 0.0,
            rise_step: 0.0,
            pan: 0.0,
            active: false,
        }
    }

    fn hit(&mut self, rate: u32, midi: u8, velocity: f32, pan: f32) {
        self.shaker = midi >= SHAKER;
        self.pan = pan;
        self.active = true;
        if self.shaker {
            self.grains.tune(7000.0, 1.2, rate);
            self.body = 0.0;
            self.slap = velocity * 0.45;
            self.slap_decay = decay(0.08, rate);
            self.rise = 0.0;
            self.rise_step = 1.0 / (0.006 * rate as f32);
        } else {
            self.settle = hz(midi) / rate as f32;
            self.step = self.settle * 1.6;
            self.glide = decay(0.08, rate);
            self.phase = 0.0;
            self.body = velocity * 0.4;
            self.body_decay = decay(0.7, rate);
            self.slap = velocity * 0.2;
            self.slap_decay = decay(0.025, rate);
            self.slap_tone = coefficient(900.0, rate);
            self.rise = 1.0;
        }
    }

    #[inline]
    fn next(&mut self, rng: &mut Rng) -> (f32, f32) {
        let out = if self.shaker {
            self.rise = (self.rise + self.rise_step).min(1.0);
            let sound = self.grains.next(rng.signed()) * self.slap * self.rise;
            self.slap *= self.slap_decay;
            if self.slap < 1e-4 {
                self.active = false;
            }
            sound
        } else {
            self.step = self.settle + (self.step - self.settle) * self.glide;
            self.phase = (self.phase + self.step).fract();
            let tone = (self.phase * TAU).sin() * self.body;
            let slap = self.slap_filter.low(rng.signed(), self.slap_tone) * self.slap;
            self.body *= self.body_decay;
            self.slap *= self.slap_decay;
            if self.body < 1e-4 && self.slap < 1e-4 {
                self.active = false;
            }
            tone + slap
        };
        let (l, r) = pan_gains(self.pan);
        (out * l, out * r)
    }
}

// ----- the synth ----------------------------------------------------------

const BASS_STRINGS: usize = 4;
const HARP_STRINGS: usize = 8;
const VIOLIN_STRINGS: usize = 4;
const BELLS: usize = 6;
const STRINGS: usize = 6;
const PAD_VOICES: usize = 16;
const PIANO_VOICES: usize = 10;
const DRUMS: usize = 4;

/// One stereo frame per layer, by [`Instrument::index`], so each can be
/// placed in the room on its own.
pub(super) type Stems = [[f32; 2]; Instrument::COUNT];
/// The lowest plucked note, for the delay lines' length.
const LOWEST_HZ: f32 = 24.0;

pub(super) struct Synth {
    rate: u32,
    rng: Rng,
    /// Instruments played from recordings, where there are some.
    sampler: Sampler,
    bass: Vec<Plucked>,
    harp: Vec<Plucked>,
    violin: Vec<Plucked>,
    bells: [Bell; BELLS],
    strings: [Sustained; STRINGS],
    pad: [Sustained; PAD_VOICES],
    drone: [DroneVoice; 2],
    drone_live: usize,
    piano: [Piano; PIANO_VOICES],
    drums: [Drum; DRUMS],
}

impl Synth {
    pub(super) fn new(rate: u32, seed: u64) -> Self {
        let max_frames = (rate as f32 / LOWEST_HZ).ceil() as usize + 2;
        Self {
            rate,
            rng: Rng::new(seed),
            sampler: Sampler::new(rate, seed.rotate_left(41)),
            bass: (0..BASS_STRINGS)
                .map(|_| Plucked::new(max_frames))
                .collect(),
            harp: (0..HARP_STRINGS)
                .map(|_| Plucked::new(max_frames / 4))
                .collect(),
            violin: (0..VIOLIN_STRINGS)
                .map(|_| Plucked::new(max_frames / 4))
                .collect(),
            bells: [Bell::new(); BELLS],
            strings: [Sustained::new(Timbre::Strings); STRINGS],
            pad: [Sustained::new(Timbre::Pad); PAD_VOICES],
            drone: [DroneVoice::new(); 2],
            drone_live: 0,
            piano: [Piano::new(); PIANO_VOICES],
            drums: [Drum::new(); DRUMS],
        }
    }

    /// Takes on an instrument's recordings, or goes back to its
    /// synthesised voice with `None`.
    pub(super) fn set_samples(
        &mut self,
        instrument: Instrument,
        samples: Option<Arc<Multisample>>,
    ) {
        self.sampler.set(instrument, samples);
    }

    /// Recordings no longer played, for the app to free.
    pub(super) fn take_released(&mut self) -> Option<Arc<Multisample>> {
        self.sampler.take_released()
    }

    pub(super) fn is_sounding(&self) -> bool {
        self.sampler.is_sounding()
            || self
                .bass
                .iter()
                .chain(&self.harp)
                .chain(&self.violin)
                .any(|s| s.active)
            || self.bells.iter().any(|b| b.active)
            || self.strings.iter().chain(&self.pad).any(|s| s.active)
            || self.drone.iter().any(|d| d.active)
            || self.piano.iter().any(|p| p.active)
            || self.drums.iter().any(|d| d.active)
    }

    /// Lets every held note go: the strings and pad fade from here, as at
    /// the end of their hold. For a change of key.
    pub(super) fn release_held(&mut self) {
        for voice in self.strings.iter_mut().chain(self.pad.iter_mut()) {
            voice.hold = 0;
        }
    }

    /// Follows the layers between notes: the drone takes its level, and
    /// fades out when its layer is off. Other instruments take theirs with
    /// each note.
    pub(super) fn follow(&mut self, settings: &AmbienceSettings) {
        let layer = settings.layer(Instrument::Drone);
        for drone in &mut self.drone {
            if !layer.enabled {
                drone.release();
            } else if drone.target > 0.0 {
                drone.level_target = layer.level;
            }
        }
    }

    /// Starts a note. A full pool takes the voice nearest its end.
    pub(super) fn play(&mut self, event: Event, settings: &AmbienceSettings) {
        let note = event.note;
        let layer = settings.layer(note.instrument);
        if !layer.enabled {
            return;
        }
        let velocity = (note.velocity * layer.level).clamp(0.0, 1.0);
        if self.sampler.has(note.instrument) {
            self.sampler.play_or_carry(event, velocity);
            // The pad keeps its synthesised voice beneath the recording.
            if note.instrument != Instrument::Pad {
                return;
            }
        }
        match note.instrument {
            Instrument::Bass | Instrument::Harp | Instrument::Violin => {
                let pool = match note.instrument {
                    Instrument::Bass => &mut self.bass,
                    Instrument::Violin => &mut self.violin,
                    _ => &mut self.harp,
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
                // A held note carried on, where it still sounds.
                let rate = self.rate;
                if let Some(from) = note.from
                    && note.duration == 0
                {
                    // Let go, nothing struck.
                    if let Some(voice) = pool.iter_mut().find(|v| v.carries(from, note.voicing)) {
                        voice.hold = 0;
                        voice.releasing = true;
                    }
                    return;
                }
                if let Some(from) = note.from
                    && let Some(voice) = pool.iter_mut().find(|v| v.carries(from, note.voicing))
                    && voice.carry(rate, note.midi, velocity, note.duration, note.voicing)
                {
                    return;
                }
                // A full pool takes the quietest voice already let go, else
                // the held one nearest its end: never one just bowed.
                let index = pool.iter().position(|s| !s.active).unwrap_or_else(|| {
                    least(pool.iter().map(|s| {
                        if s.releasing {
                            (s.env * s.level * 1e9) as u64
                        } else {
                            u64::MAX / 2 + s.hold
                        }
                    }))
                });
                pool[index].bow(
                    &mut self.rng,
                    self.rate,
                    note.midi,
                    velocity,
                    note.pan,
                    note.duration,
                    note.voicing,
                );
            }
            Instrument::Drone => {
                self.drone[self.drone_live].release();
                self.drone_live = 1 - self.drone_live;
                self.drone[self.drone_live].tune(self.rate, note.midi, velocity);
            }
            Instrument::Piano => {
                let index = self
                    .piano
                    .iter()
                    .position(|p| !p.active)
                    .unwrap_or_else(|| least(self.piano.iter().map(|p| (p.amps[0] * 1e6) as u64)));
                self.piano[index].strike(self.rate, note.midi, velocity, note.pan);
            }
            Instrument::Drums => {
                let index = self
                    .drums
                    .iter()
                    .position(|d| !d.active)
                    .unwrap_or_else(|| {
                        least(self.drums.iter().map(|d| ((d.body + d.slap) * 1e6) as u64))
                    });
                self.drums[index].hit(self.rate, note.midi, velocity, note.pan);
            }
        }
    }

    /// Adds the next frame of every instrument into its layer's stem,
    /// at `gain`.
    #[inline]
    pub(super) fn next(&mut self, gain: f32, stems: &mut Stems) {
        self.sampler.next(stems);
        let pools = [
            (Instrument::Bass, &mut self.bass),
            (Instrument::Harp, &mut self.harp),
            (Instrument::Violin, &mut self.violin),
        ];
        for (instrument, pool) in pools {
            let stem = &mut stems[instrument.index()];
            for string in pool.iter_mut().filter(|s| s.active) {
                let (l, r) = string.next();
                stem[0] += l;
                stem[1] += r;
            }
        }
        let stem = &mut stems[Instrument::Bells.index()];
        for bell in self.bells.iter_mut().filter(|b| b.active) {
            let (l, r) = bell.next();
            stem[0] += l;
            stem[1] += r;
        }
        for (instrument, pool) in [
            (Instrument::Strings, &mut self.strings[..]),
            (Instrument::Pad, &mut self.pad[..]),
        ] {
            let stem = &mut stems[instrument.index()];
            for voice in pool.iter_mut().filter(|v| v.active) {
                let (l, r) = voice.next();
                stem[0] += l;
                stem[1] += r;
            }
        }
        let stem = &mut stems[Instrument::Drone.index()];
        for drone in self.drone.iter_mut().filter(|d| d.active) {
            let (l, r) = drone.next();
            stem[0] += l;
            stem[1] += r;
        }
        let stem = &mut stems[Instrument::Piano.index()];
        for piano in self.piano.iter_mut().filter(|p| p.active) {
            let (l, r) = piano.next(&mut self.rng);
            stem[0] += l;
            stem[1] += r;
        }
        let stem = &mut stems[Instrument::Drums.index()];
        for drum in self.drums.iter_mut().filter(|d| d.active) {
            let (l, r) = drum.next(&mut self.rng);
            stem[0] += l;
            stem[1] += r;
        }
        for stem in stems.iter_mut() {
            stem[0] *= gain;
            stem[1] *= gain;
        }
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
        string.bow(&mut rng, rate, 60, 1.0, 0.0, rate as u64 * 2, 0);
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

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().map(|s| s.abs()).fold(0.0, f32::max)
    }

    #[test]
    fn held_notes_let_go_at_once_for_a_change_of_key() {
        use super::super::{Note, Style};
        let rate = 48_000;
        let mut synth = Synth::new(rate, 1);
        let settings = AmbienceSettings {
            enabled: true,
            ..Style::Affirmation.settings()
        };
        let note = Note {
            instrument: Instrument::Pad,
            midi: 60,
            velocity: 0.8,
            pan: 0.0,
            duration: rate as u64 * 20,
            from: None,
            voicing: 0,
        };
        synth.play(Event { at: 0, note }, &settings);
        for _ in 0..rate {
            synth.next(1.0, &mut [[0.0; 2]; Instrument::COUNT]);
        }
        assert!(
            synth
                .pad
                .iter()
                .any(|voice| voice.active && !voice.releasing)
        );
        synth.release_held();
        // Well before its twenty seconds, gone within its release.
        for _ in 0..rate * 8 {
            synth.next(1.0, &mut [[0.0; 2]; Instrument::COUNT]);
        }
        assert!(!synth.pad.iter().any(|voice| voice.active));
    }

    #[test]
    fn a_piano_note_rings_in_tune_and_dies_away() {
        let rate = 48_000;
        let mut rng = Rng::new(1);
        let mut piano = Piano::new();
        piano.strike(rate, 69, 0.8, 0.0);
        let out: Vec<f32> = (0..rate as usize * 8)
            .map(|_| piano.next(&mut rng).0)
            .collect();
        // Once the upper partials have gone, the fundamental is left.
        let tail = &out[rate as usize * 2..rate as usize * 2 + rate as usize / 10];
        let cycles = cycles_in(tail);
        assert!((cycles - 44.0).abs() <= 1.5, "{cycles} cycles in 100 ms");
        assert!(peak(&out[..4800]) > peak(&out[rate as usize * 3..]) * 4.0);
        assert!(peak(&out) < 0.5);
        assert!(!piano.active, "silent after eight seconds");
    }

    #[test]
    fn pizzicato_and_drums_are_short() {
        let rate = 48_000;
        let mut rng = Rng::new(3);
        let mut string = Plucked::new(rate as usize / 24);
        string.pluck(&mut rng, rate, 76, 1.0, 0.0, Instrument::Violin);
        let out: Vec<f32> = (0..rate as usize * 2).map(|_| string.next().0).collect();
        assert!(peak(&out[..2400]) > 0.05);
        assert!(!string.active, "a pizzicato is over within two seconds");

        let mut drum = Drum::new();
        drum.hit(rate, 40, 1.0, 0.0);
        let out: Vec<f32> = (0..rate as usize * 2)
            .map(|_| drum.next(&mut rng).0)
            .collect();
        let start = peak(&out[..2400]);
        assert!(start > 0.1, "a frame drum thumps: {start}");
        assert!(peak(&out[rate as usize / 2..]) < start * 0.1);
        assert!(!drum.active);

        let mut shaker = Drum::new();
        shaker.hit(rate, SHAKER, 1.0, 0.0);
        let out: Vec<f32> = (0..rate as usize / 2)
            .map(|_| shaker.next(&mut rng).0)
            .collect();
        assert!(peak(&out[..4800]) > 0.02);
        assert!(!shaker.active, "a shake is over in half a second");
    }

    #[test]
    fn a_carried_pad_note_holds_on_glides_a_step_and_lets_go_further() {
        use super::super::{Note, Style};
        let rate = 48_000;
        let mut synth = Synth::new(rate, 1);
        let settings = AmbienceSettings {
            enabled: true,
            ..Style::Affirmation.settings()
        };
        let pad = |midi, from, seconds: u64| Event {
            at: rate as u64 * seconds,
            note: Note {
                instrument: Instrument::Pad,
                midi,
                velocity: 0.8,
                pan: 0.0,
                duration: rate as u64 * 4,
                from,
                voicing: seconds as u32 + 1,
            },
        };
        let run = |synth: &mut Synth, seconds: f32| {
            for _ in 0..(seconds * rate as f32) as usize {
                synth.next(1.0, &mut [[0.0; 2]; Instrument::COUNT]);
            }
        };
        let sounding = |synth: &Synth| {
            synth
                .pad
                .iter()
                .filter(|v| v.active && !v.releasing)
                .count()
        };
        synth.play(pad(60, None, 0), &settings);
        run(&mut synth, 2.0);
        // Held on: still one voice, held longer.
        synth.play(pad(60, Some(60), 2), &settings);
        assert_eq!(sounding(&synth), 1);
        run(&mut synth, 3.0);
        assert_eq!(sounding(&synth), 1, "held past its first four seconds");
        // A tone up: the same voice glides there.
        synth.play(pad(62, Some(60), 5), &settings);
        assert_eq!(sounding(&synth), 1);
        let voice = synth.pad.iter().find(|v| v.active).unwrap();
        assert!(voice.glide < 0.95, "heard where it was at first");
        run(&mut synth, 3.0);
        let voice = synth.pad.iter().find(|v| v.active && !v.releasing).unwrap();
        assert_eq!(voice.midi, 62);
        assert!((voice.glide - 1.0).abs() < 1e-3, "and there now");
        let fresh = hz(62) / rate as f32;
        let middle = voice.steps.iter().sum::<f32>() / UNISON as f32;
        assert!(
            (middle / fresh - 1.0).abs() < 1e-4,
            "in tune: {middle} {fresh}"
        );
        // A fifth away: let go, and the new note bowed.
        synth.play(pad(69, Some(62), 8), &settings);
        assert_eq!(sounding(&synth), 1);
        assert!(
            synth
                .pad
                .iter()
                .any(|v| v.active && v.releasing && v.midi == 62)
        );
        assert!(
            synth
                .pad
                .iter()
                .any(|v| v.active && !v.releasing && v.midi == 69)
        );
    }

    #[test]
    fn a_chord_carries_each_voice_once_even_when_one_glides_onto_anothers_note() {
        use super::super::{Note, Style};
        let rate = 48_000;
        let mut synth = Synth::new(rate, 1);
        let settings = AmbienceSettings {
            enabled: true,
            ..Style::Affirmation.settings()
        };
        let pad = |midi, from, at: f32| Event {
            at: (at * rate as f32) as u64,
            note: Note {
                instrument: Instrument::Pad,
                midi,
                velocity: 0.8,
                pan: 0.0,
                duration: rate as u64 * 6,
                from,
                // A voicing per chord: the notes within a second are one.
                voicing: at as u32 + 1,
            },
        };
        synth.play(pad(65, None, 0.0), &settings);
        synth.play(pad(67, None, 0.35), &settings);
        for _ in 0..rate * 3 {
            synth.next(1.0, &mut [[0.0; 2]; Instrument::COUNT]);
        }
        // The next chord: 65 glides to 67, then the voice that was on 67
        // glides to 69. The first must not be taken for the second.
        synth.play(pad(67, Some(65), 3.0), &settings);
        synth.play(pad(69, Some(67), 3.35), &settings);
        let mut held: Vec<u8> = synth
            .pad
            .iter()
            .filter(|v| v.active && !v.releasing)
            .map(|v| v.midi)
            .collect();
        held.sort_unstable();
        assert_eq!(held, [67, 69], "both carried, neither let go");
        assert!(
            synth.pad.iter().filter(|v| v.active).count() == 2,
            "nothing struck afresh"
        );
    }

    #[test]
    fn once_a_held_over_chord_has_sounded_only_its_notes_are_held() {
        // Composer and synth together, the pad alone, for eight minutes.
        use super::super::composer::Composer;
        use super::super::{Density, Layer, Style};
        let rate = 48_000;
        for (style, energy) in [
            (Style::RainStudy, None),
            (Style::Cathedral, None),
            (Style::NightDrift, None),
            (Style::Affirmation, Some(0.6)),
        ] {
            let mut settings = style.settings();
            settings.enabled = true;
            if let Some(energy) = energy {
                settings.energy = energy;
            }
            for layer in &mut settings.layers {
                layer.enabled = false;
            }
            *settings.layer_mut(Instrument::Pad) = Layer::on(0.7, Density::LIVELY);
            let mut composer = Composer::new(rate, 7);
            let mut synth = Synth::new(rate, 1);
            // The voicing sounding now: its number, when it began, its
            // notes, and whether it was held over from the last.
            let mut current: Option<(u32, u64, Vec<u8>, bool)> = None;
            // What was held (each voice's note and frames of hold left)
            // once a voicing had sounded, with its notes and whether it was
            // held over; judged when the next voicing shows it was not cut
            // short (a change of chord can drop the end of a voicing, its
            // notes then held until the next leads them on, within 1.5 s).
            type Snapshot = (u64, Vec<u8>, bool, Vec<(u8, u64)>);
            let mut snapshot: Option<Snapshot> = None;
            let (mut at, end) = (0u64, rate as u64 * 60 * 8);
            // A voicing's notes all begin within 1.4 s.
            let settled = rate as u64 * 3 / 2;
            let mut checked = 0;
            while at < end {
                composer.poll(&settings, at, at + 512);
                if composer.take_ended() {
                    synth.release_held();
                }
                for now in at..at + 512 {
                    while let Some(event) = composer.due(now) {
                        let note = event.note;
                        if current.as_ref().is_none_or(|(v, ..)| *v != note.voicing) {
                            if let Some((began, notes, held_over, held)) = snapshot.take()
                                && now > began + 2 * settled
                            {
                                for (midi, left) in held {
                                    // Under a held-over voicing nothing else
                                    // is held; under any, nothing older for
                                    // long.
                                    let long = left >= 2 * rate as u64;
                                    assert!(
                                        notes.contains(&midi) || !(held_over || long),
                                        "{}: {midi} held {} s more under {notes:?}",
                                        style.name(),
                                        left / rate as u64
                                    );
                                }
                                checked += 1;
                            }
                            current = Some((note.voicing, now, Vec::new(), note.from.is_some()));
                        }
                        if let Some((.., notes, _)) = &mut current {
                            notes.push(note.midi);
                        }
                        synth.play(event, &settings);
                    }
                    synth.next(1.0, &mut [[0.0; 2]; Instrument::COUNT]);
                    if let Some((_, began, notes, held_over)) = &current
                        && now == began + settled
                    {
                        let held = synth
                            .pad
                            .iter()
                            .filter(|v| v.active && !v.releasing)
                            .map(|v| (v.midi, v.hold))
                            .collect();
                        snapshot = Some((*began, notes.clone(), *held_over, held));
                    }
                }
                at += 512;
            }
            assert!(checked > 5, "{}: {checked} chords checked", style.name());
        }
    }

    #[test]
    fn a_chord_into_a_full_pool_keeps_every_note() {
        use super::super::{Note, Style};
        let rate = 48_000;
        let mut synth = Synth::new(rate, 1);
        let settings = AmbienceSettings {
            enabled: true,
            ..Style::Affirmation.settings()
        };
        let strings = |midi| Event {
            at: 0,
            note: Note {
                instrument: Instrument::Strings,
                midi,
                velocity: 0.7,
                pan: 0.0,
                duration: rate as u64 * 30,
                from: None,
                voicing: 0,
            },
        };
        for midi in 50..50 + STRINGS as u8 {
            synth.play(strings(midi), &settings);
        }
        for _ in 0..rate * 3 {
            synth.next(1.0, &mut [[0.0; 2]; Instrument::COUNT]);
        }
        for midi in [70, 74, 77] {
            synth.play(strings(midi), &settings);
        }
        let held: Vec<u8> = synth
            .strings
            .iter()
            .filter(|v| v.active)
            .map(|v| v.midi)
            .collect();
        for midi in [70, 74, 77] {
            assert!(held.contains(&midi), "{midi} lost: {held:?}");
        }
    }

    #[test]
    fn a_carried_note_moves_to_its_new_level() {
        use super::super::{Note, Style};
        let rate = 48_000;
        let mut synth = Synth::new(rate, 1);
        let settings = AmbienceSettings {
            enabled: true,
            ..Style::Affirmation.settings()
        };
        let pad = |velocity, from, at: u64| Event {
            at: rate as u64 * at,
            note: Note {
                instrument: Instrument::Pad,
                midi: 60,
                velocity,
                pan: 0.0,
                duration: rate as u64 * 6,
                from,
                voicing: at as u32 + 1,
            },
        };
        synth.play(pad(0.8, None, 0), &settings);
        for _ in 0..rate * 3 {
            synth.next(1.0, &mut [[0.0; 2]; Instrument::COUNT]);
        }
        let before = synth.pad.iter().find(|v| v.active).unwrap().level;
        synth.play(pad(0.4, Some(60), 3), &settings);
        for _ in 0..rate * 2 {
            synth.next(1.0, &mut [[0.0; 2]; Instrument::COUNT]);
        }
        let after = synth.pad.iter().find(|v| v.active).unwrap().level;
        assert!((after / before - 0.5).abs() < 0.01, "{before} {after}");
    }

    #[test]
    fn each_layer_sounds_in_its_own_stem_and_the_drone_is_wide() {
        use super::super::{Note, Style};
        let rate = 48_000;
        let mut synth = Synth::new(rate, 1);
        let mut settings = Style::Cathedral.settings();
        settings.enabled = true;
        for layer in &mut settings.layers {
            layer.enabled = true;
        }
        for instrument in [Instrument::Drone, Instrument::Bells] {
            synth.play(
                Event {
                    at: 0,
                    note: Note {
                        instrument,
                        midi: if instrument == Instrument::Drone {
                            38
                        } else {
                            84
                        },
                        velocity: 0.8,
                        pan: 0.0,
                        duration: 0,
                        from: None,
                        voicing: 0,
                    },
                },
                &settings,
            );
        }
        let mut energy = [0.0f32; Instrument::COUNT];
        let mut side = 0.0f32;
        for _ in 0..rate {
            let mut stems = [[0.0; 2]; Instrument::COUNT];
            synth.next(1.0, &mut stems);
            for (sum, stem) in energy.iter_mut().zip(stems) {
                *sum += stem[0] * stem[0] + stem[1] * stem[1];
            }
            let drone = stems[Instrument::Drone.index()];
            side += (drone[0] - drone[1]).abs();
        }
        for instrument in Instrument::ALL {
            let heard = energy[instrument.index()] > 1e-3;
            let played = matches!(instrument, Instrument::Drone | Instrument::Bells);
            assert_eq!(heard, played, "{instrument:?}");
        }
        assert!(side > 1.0, "the drone's two sides differ: {side}");
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
                from: None,
                voicing: 0,
            },
        };
        synth.play(event(31), &settings);
        for _ in 0..rate * 6 {
            synth.next(1.0, &mut [[0.0; 2]; Instrument::COUNT]);
        }
        synth.play(event(38), &settings);
        for _ in 0..rate * 2 {
            synth.next(1.0, &mut [[0.0; 2]; Instrument::COUNT]);
        }
        assert!(
            synth.drone.iter().all(|d| d.active),
            "both sound while crossing"
        );
        for _ in 0..rate * 8 {
            synth.next(1.0, &mut [[0.0; 2]; Instrument::COUNT]);
        }
        assert_eq!(synth.drone.iter().filter(|d| d.active).count(), 1);
    }
}

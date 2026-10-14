//! The ambience's own space: an ensemble chorus that thickens and widens
//! the instruments, and a long, softly moving hall they sit in. The tiles
//! keep the mixer's smaller room; the Generator's music wants more air.
//!
//! The hall is a feedback delay network: eight delay lines of unrelated
//! lengths, mixed back into each other through a Householder matrix (every
//! line feeds every other, losing nothing), each damped above a few
//! kilohertz and slowly swaying in length so the tail shimmers rather than
//! rings. Everything is allocated in `new`; processing allocates nothing.

use std::f32::consts::TAU;

use super::dsp::{OnePole, coefficient};

/// The hall's delay lines in milliseconds: unrelated, so their echoes
/// never line up.
const LINES_MS: [f32; 8] = [37.3, 43.9, 51.1, 58.7, 66.1, 73.3, 81.7, 89.9];
/// How long the tail takes to fall 60 dB, and how much brighter the start
/// is than the end.
const DECAY_SECONDS: f32 = 5.2;
const DAMPING_HZ: f32 = 4200.0;
/// The gap before the hall answers.
const PRE_DELAY_MS: f32 = 28.0;
/// How far, in milliseconds, and how slowly the lines sway.
const SWAY_MS: f32 = 0.35;
const SWAY_HZ: [f32; 4] = [0.11, 0.13, 0.17, 0.19];
/// Diffusers before the network smear an attack into a cloud.
const DIFFUSERS_MS: [f32; 4] = [4.7, 3.6, 2.9, 1.9];
const DIFFUSION: f32 = 0.6;

/// A delay line read at a fractional position.
struct Line {
    buffer: Vec<f32>,
    write: usize,
}

impl Line {
    fn new(frames: usize) -> Self {
        Self {
            buffer: vec![0.0; frames.max(4)],
            write: 0,
        }
    }

    #[inline]
    fn push(&mut self, input: f32) {
        self.buffer[self.write] = input;
        self.write = (self.write + 1) % self.buffer.len();
    }

    /// The sample `delay` frames ago (at least 1, at most the length
    /// less 2), interpolated.
    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let len = self.buffer.len();
        let delay = delay.clamp(1.0, (len - 2) as f32);
        let whole = delay.floor();
        let frac = delay - whole;
        let back = whole as usize;
        let a = self.buffer[(self.write + len - back) % len];
        let b = self.buffer[(self.write + len - back - 1) % len];
        a + (b - a) * frac
    }
}

/// A Schroeder all-pass: passes every frequency, smears it in time.
struct Diffuser {
    line: Line,
    delay: f32,
}

impl Diffuser {
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.line.read(self.delay);
        let fed = input + delayed * DIFFUSION;
        self.line.push(fed);
        delayed - fed * DIFFUSION
    }
}

pub(super) struct Hall {
    rate: f32,
    pre_delay: [Line; 2],
    diffusers: [[Diffuser; 4]; 2],
    lines: [Line; 8],
    lengths: [f32; 8],
    feedback: [f32; 8],
    damping: [OnePole; 8],
    damping_coefficient: f32,
    sway: [f32; 4],
    sway_step: [f32; 4],
}

impl Hall {
    pub(super) fn new(rate: u32) -> Self {
        let rate_f = rate as f32;
        let frames = |ms: f32| (ms * rate_f / 1000.0) as usize;
        let sway_frames = SWAY_MS * rate_f / 1000.0;
        let lengths = LINES_MS.map(|ms| ms * rate_f / 1000.0);
        let diffusers = |scale: f32| {
            DIFFUSERS_MS.map(|ms| Diffuser {
                line: Line::new(frames(ms * scale) + 4),
                delay: ms * scale * rate_f / 1000.0,
            })
        };
        Self {
            rate: rate_f,
            pre_delay: [
                Line::new(frames(PRE_DELAY_MS) + 4),
                Line::new(frames(PRE_DELAY_MS) + 4),
            ],
            // The right side's diffusers a little longer, for width.
            diffusers: [diffusers(1.0), diffusers(1.13)],
            lines: lengths.map(|length| Line::new(length as usize + sway_frames as usize + 4)),
            lengths,
            feedback: lengths.map(|length| 10f32.powf(-3.0 * length / (DECAY_SECONDS * rate_f))),
            damping: [OnePole::default(); 8],
            damping_coefficient: coefficient(DAMPING_HZ, rate),
            sway: [0.0, 0.25, 0.5, 0.75],
            sway_step: SWAY_HZ.map(|hz| hz / rate_f),
        }
    }

    /// The hall's answer to one stereo frame sent into it.
    #[inline]
    pub(super) fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let pre = PRE_DELAY_MS * self.rate / 1000.0;
        self.pre_delay[0].push(left);
        self.pre_delay[1].push(right);
        let mut input = [self.pre_delay[0].read(pre), self.pre_delay[1].read(pre)];
        for (side, sample) in input.iter_mut().enumerate() {
            for diffuser in &mut self.diffusers[side] {
                *sample = diffuser.process(*sample);
            }
        }
        for (sway, step) in self.sway.iter_mut().zip(self.sway_step) {
            *sway = (*sway + step).fract();
        }
        let sway_frames = SWAY_MS * self.rate / 1000.0;
        let mut taps = [0.0f32; 8];
        for (i, tap) in taps.iter_mut().enumerate() {
            let sway = (self.sway[i % 4] * TAU).sin() * sway_frames;
            let delayed = self.lines[i].read(self.lengths[i] + sway);
            *tap = self.damping[i].low(delayed, self.damping_coefficient) * self.feedback[i];
        }
        // Householder: each line gets every other, nothing gained or lost.
        let sum: f32 = taps.iter().sum::<f32>() * (2.0 / 8.0);
        for (i, tap) in taps.iter().enumerate() {
            let side = input[i % 2];
            self.lines[i].push(tap - sum + side);
        }
        let left = taps[0] + taps[2] - taps[4] + taps[6];
        let right = taps[1] - taps[3] + taps[5] + taps[7];
        (left * 0.5, right * 0.5)
    }
}

/// Voices of the chorus on each side, their delays and how fast they sway.
const CHORUS_MS: [f32; 3] = [11.0, 16.0, 21.0];
const CHORUS_DEPTH_MS: f32 = 2.2;
const CHORUS_HZ: [f32; 3] = [0.23, 0.31, 0.41];
/// How much of the chorus is heard beside the dry sound.
const CHORUS_MIX: f32 = 0.35;

/// An ensemble chorus: three slowly swaying copies a side, a little later
/// than the dry sound, so one voice sounds like several, and wide.
pub(super) struct Chorus {
    rate: f32,
    lines: [Line; 2],
    phases: [f32; 3],
    steps: [f32; 3],
}

impl Chorus {
    pub(super) fn new(rate: u32) -> Self {
        let rate_f = rate as f32;
        let longest = (CHORUS_MS[2] + CHORUS_DEPTH_MS) * rate_f / 1000.0;
        Self {
            rate: rate_f,
            lines: [
                Line::new(longest as usize + 8),
                Line::new(longest as usize + 8),
            ],
            phases: [0.0, 0.33, 0.66],
            steps: CHORUS_HZ.map(|hz| hz / rate_f),
        }
    }

    /// One frame through the chorus: the dry sound and the ensemble.
    #[inline]
    pub(super) fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        self.lines[0].push(left);
        self.lines[1].push(right);
        let ms = self.rate / 1000.0;
        let (mut wet_left, mut wet_right) = (0.0, 0.0);
        let voices = self.phases.iter_mut().zip(self.steps).zip(CHORUS_MS);
        for ((phase, step), delay) in voices {
            *phase = (*phase + step).fract();
            let sway = (*phase * TAU).sin() * CHORUS_DEPTH_MS;
            // The right side sways the other way, for width.
            wet_left += self.lines[0].read((delay + sway) * ms);
            wet_right += self.lines[1].read((delay - sway) * ms);
        }
        let wet = CHORUS_MIX / 3.0;
        (left + wet_left * wet, right + wet_right * wet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hall_rings_on_for_seconds_and_dies_away() {
        let rate = 48_000;
        let mut hall = Hall::new(rate);
        let mut out = Vec::new();
        for i in 0..rate as usize * 9 {
            let click = if i == 0 { 1.0 } else { 0.0 };
            let (l, r) = hall.process(click, click);
            out.push((l, r));
        }
        let level = |from: f32, to: f32| {
            let slice = &out[(from * rate as f32) as usize..(to * rate as f32) as usize];
            (slice.iter().map(|(l, r)| l * l + r * r).sum::<f32>() / slice.len() as f32).sqrt()
        };
        let early = level(0.05, 0.5);
        let two = level(2.0, 2.5);
        let late = level(8.0, 8.5);
        assert!(early > 0.0 && two > early * 0.01, "rings on: {early} {two}");
        assert!(late < early * 0.002, "and dies away: {early} {late}");
        assert!(
            out.iter()
                .all(|(l, r)| l.is_finite() && r.is_finite() && l.abs() < 1.0)
        );
        // Wide: the two sides differ.
        let side: f32 = out.iter().map(|(l, r)| (l - r).abs()).sum();
        assert!(side > 0.1);
    }

    #[test]
    fn the_chorus_keeps_the_level_and_widens_a_mono_sound() {
        let rate = 48_000;
        let mut chorus = Chorus::new(rate);
        let mut dry = 0.0;
        let mut wet = 0.0;
        let mut side = 0.0;
        for i in 0..rate as usize * 2 {
            let x = (i as f32 * 440.0 * TAU / rate as f32).sin() * 0.5;
            let (l, r) = chorus.process(x, x);
            if i > rate as usize / 2 {
                dry += x * x * 2.0;
                wet += l * l + r * r;
                side += (l - r) * (l - r);
            }
        }
        assert!(
            (wet / dry).sqrt() > 0.6 && (wet / dry).sqrt() < 1.3,
            "{}",
            (wet / dry).sqrt()
        );
        assert!(side > wet * 0.01, "wide");
    }
}

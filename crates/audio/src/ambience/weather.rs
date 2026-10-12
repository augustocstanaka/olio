//! The weather: filtered noise with slow movement in it. Rain is a hiss
//! with drops on top; wind is low noise that gusts; a winter wind is
//! thinner, with a whistle.

use std::f32::consts::{FRAC_PI_4, PI, TAU};

use super::{Rng, Weather};
use crate::mixer::Ramp;

#[inline]
fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    (angle.cos(), angle.sin())
}

fn coefficient(hz: f32, rate: u32) -> f32 {
    let x = (-TAU * hz / rate as f32).exp();
    (1.0 - x).clamp(0.0, 1.0)
}

/// A one-pole filter.
#[derive(Clone, Copy, Default)]
struct OnePole {
    state: f32,
}

impl OnePole {
    #[inline]
    fn low(&mut self, input: f32, coefficient: f32) -> f32 {
        self.state += (input - self.state) * coefficient;
        self.state
    }

    #[inline]
    fn high(&mut self, input: f32, coefficient: f32) -> f32 {
        input - self.low(input, coefficient)
    }
}

/// A two-pole resonant band-pass.
#[derive(Clone, Copy, Default)]
struct BandPass {
    a1: f32,
    a2: f32,
    b0: f32,
    z1: f32,
    z2: f32,
}

impl BandPass {
    fn tune(&mut self, hz: f32, q: f32, rate: u32) {
        let w = 2.0 * PI * hz / rate as f32;
        let r = (-w / (2.0 * q.max(0.1))).exp();
        self.a1 = -2.0 * r * w.cos();
        self.a2 = r * r;
        self.b0 = 1.0 - r;
    }

    #[inline]
    fn next(&mut self, input: f32) -> f32 {
        let out = self.b0 * input - self.a1 * self.z1 - self.a2 * self.z2;
        self.z2 = self.z1;
        self.z1 = out;
        out
    }
}

/// One raindrop: a short burst of noise through a band-pass, dying away.
#[derive(Clone, Copy, Default)]
struct Drop {
    env: f32,
    decay: f32,
    filter: BandPass,
    pan: f32,
}

const DROPS: usize = 10;

pub(super) struct Bed {
    rng: Rng,
    rate: u32,
    /// Gusts: where the wind is heading, and where it is.
    gust: f32,
    gust_target: f32,
    gust_left: u32,
    low: [OnePole; 2],
    high: [OnePole; 2],
    smooth: [OnePole; 2],
    whistle: [BandPass; 2],
    drops: [Drop; DROPS],
    /// The weather's own gain, so a change of weather does not click.
    gain: Ramp,
    weather: Weather,
}

impl Bed {
    pub(super) fn new(rate: u32, seed: u64) -> Self {
        let mut bed = Self {
            rng: Rng::new(seed),
            rate,
            gust: 0.5,
            gust_target: 0.5,
            gust_left: 0,
            low: [OnePole::default(); 2],
            high: [OnePole::default(); 2],
            smooth: [OnePole::default(); 2],
            whistle: [BandPass::default(); 2],
            drops: [Drop::default(); DROPS],
            gain: Ramp::at(0.0),
            weather: Weather::None,
        };
        for filter in &mut bed.whistle {
            filter.tune(800.0, 12.0, rate);
        }
        bed
    }

    pub(super) fn set_weather(&mut self, weather: Weather) {
        if weather != self.weather {
            self.weather = weather;
            let frames = self.rate / 8;
            let target = if weather == Weather::None { 0.0 } else { 1.0 };
            self.gain.towards(target, frames);
        }
    }

    /// Moves the gusts along: a slow drift towards a target picked every
    /// few seconds.
    #[inline]
    fn breathe(&mut self) {
        if self.gust_left == 0 {
            self.gust_target = self.rng.range(0.15, 1.0);
            self.gust_left = (self.rng.range(1.5, 6.0) * self.rate as f32) as u32;
            let hz = self.rng.range(500.0, 1400.0);
            for filter in &mut self.whistle {
                filter.tune(hz, 14.0, self.rate);
            }
        }
        self.gust_left -= 1;
        self.gust += (self.gust_target - self.gust) * 0.00002;
    }

    /// The next frame of weather.
    #[inline]
    pub(super) fn next(&mut self) -> (f32, f32) {
        let gain = self.gain.next();
        if gain == 0.0 && self.gain.is_silent() {
            return (0.0, 0.0);
        }
        self.breathe();
        let rate = self.rate;
        let mut out = [0.0f32; 2];
        match self.weather {
            Weather::None => {}
            Weather::Rain => {
                let hiss = coefficient(1200.0, rate);
                let soft = coefficient(5500.0, rate);
                for (side, sample) in out.iter_mut().enumerate() {
                    let noise = self.rng.signed();
                    let high = self.high[side].high(noise, hiss);
                    *sample = self.low[side].low(high, soft) * 0.32;
                }
                // About twenty drops a second, each a few milliseconds.
                if self.rng.chance(22.0 / rate as f32)
                    && let Some(drop) = self.drops.iter_mut().find(|drop| drop.env < 0.001)
                {
                    drop.env = self.rng.range(0.3, 1.0);
                    drop.decay = 1.0 - 1.0 / (self.rng.range(0.004, 0.02) * rate as f32);
                    drop.filter.tune(self.rng.range(1800.0, 6500.0), 4.0, rate);
                    drop.pan = self.rng.signed() * 0.8;
                }
                for drop in &mut self.drops {
                    if drop.env >= 0.001 {
                        let burst = drop.filter.next(self.rng.signed()) * drop.env;
                        drop.env *= drop.decay;
                        let (l, r) = pan_gains(drop.pan);
                        out[0] += burst * l * 0.85;
                        out[1] += burst * r * 0.85;
                    }
                }
            }
            Weather::Wind | Weather::Winter => {
                let winter = self.weather == Weather::Winter;
                let cutoff = if winter {
                    250.0 + 900.0 * self.gust
                } else {
                    120.0 + 600.0 * self.gust
                };
                let body = coefficient(cutoff, rate);
                let level = (0.25 + 0.75 * self.gust) * if winter { 0.55 } else { 0.8 };
                let whistle_level = if winter {
                    0.25 + 0.5 * self.gust
                } else {
                    0.12 * self.gust
                };
                for (side, sample) in out.iter_mut().enumerate() {
                    let noise = self.rng.signed();
                    let rumble = self.low[side].low(noise, body);
                    let rumble = self.smooth[side].low(rumble, body);
                    let whistle = self.whistle[side].next(noise) * whistle_level;
                    *sample = (rumble * 3.5 * level + whistle) * 0.7;
                }
            }
        }
        (out[0] * gain, out[1] * gain)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_weather_sounds_within_bounds_and_none_is_silent() {
        for weather in Weather::ALL {
            let mut bed = Bed::new(48_000, 4);
            bed.set_weather(weather);
            let frames: Vec<(f32, f32)> = (0..48_000 * 3).map(|_| bed.next()).collect();
            let tail = &frames[48_000..];
            let peak = tail
                .iter()
                .map(|(l, r)| l.abs().max(r.abs()))
                .fold(0.0, f32::max);
            if weather == Weather::None {
                assert_eq!(peak, 0.0);
            } else {
                assert!(peak > 0.01 && peak < 1.0, "{}: {peak}", weather.name());
                assert!(tail.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
            }
        }
    }
}

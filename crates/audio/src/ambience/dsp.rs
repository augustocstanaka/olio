//! Small building blocks shared by the instruments and the backgrounds:
//! panning, pitch, and two cheap filters. Nothing here allocates.

use std::f32::consts::{FRAC_PI_4, PI, TAU};

/// Equal-power panning: -1 is left, 1 right.
#[inline]
pub(super) fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    (angle.cos(), angle.sin())
}

/// The frequency of a MIDI note.
pub(super) fn hz(midi: u8) -> f32 {
    440.0 * 2f32.powf((f32::from(midi) - 69.0) / 12.0)
}

/// A one-pole low-pass coefficient for `hz` at `rate`.
pub(super) fn coefficient(hz: f32, rate: u32) -> f32 {
    let x = (-TAU * hz / rate as f32).exp();
    (1.0 - x).clamp(0.0, 1.0)
}

/// The per-frame factor that takes a level down 60 dB in `seconds`.
pub(super) fn decay(seconds: f32, rate: u32) -> f32 {
    (-6.9 / (seconds.max(1e-4) * rate as f32)).exp()
}

/// A one-pole filter.
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct OnePole {
    state: f32,
}

impl OnePole {
    #[inline]
    pub(super) fn low(&mut self, input: f32, coefficient: f32) -> f32 {
        self.state += (input - self.state) * coefficient;
        self.state
    }

    #[inline]
    pub(super) fn high(&mut self, input: f32, coefficient: f32) -> f32 {
        input - self.low(input, coefficient)
    }
}

/// A two-pole resonant band-pass.
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct BandPass {
    a1: f32,
    a2: f32,
    b0: f32,
    z1: f32,
    z2: f32,
}

impl BandPass {
    pub(super) fn tune(&mut self, hz: f32, q: f32, rate: u32) {
        let w = 2.0 * PI * hz.min(rate as f32 * 0.45) / rate as f32;
        let r = (-w / (2.0 * q.max(0.1))).exp();
        self.a1 = -2.0 * r * w.cos();
        self.a2 = r * r;
        self.b0 = 1.0 - r;
    }

    #[inline]
    pub(super) fn next(&mut self, input: f32) -> f32 {
        let out = self.b0 * input - self.a1 * self.z1 - self.a2 * self.z2;
        self.z2 = self.z1;
        self.z1 = out;
        out
    }
}

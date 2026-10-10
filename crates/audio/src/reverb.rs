//! A stereo room reverb (the public-domain Freeverb design by Jezar at
//! Dreampoint): eight damped comb filters in parallel, then four all-pass
//! filters in series, per channel.
//!
//! The mixer owns one instance as a send bus: each voice sends a share of
//! itself in, and the reverb's output is added to the mix. Buffers are
//! allocated in [`Reverb::new`], so [`Reverb::process`] is real-time safe.

/// Comb and all-pass lengths in samples at 44.1 kHz, scaled to the actual
/// rate. The right channel's are longer by `STEREO_SPREAD`.
const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const TUNING_RATE: f32 = 44_100.0;

const INPUT_GAIN: f32 = 0.015;
const ALLPASS_FEEDBACK: f32 = 0.5;
/// Room size 0.75 on Freeverb's scale: a medium hall, about two seconds.
const COMB_FEEDBACK: f32 = 0.75 * 0.28 + 0.7;
const DAMPING: f32 = 0.35 * 0.4;
/// Freeverb's wet scaling, with full stereo width.
const WET: f32 = 1.0;

struct Comb {
    buffer: Vec<f32>,
    index: usize,
    filtered: f32,
}

impl Comb {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; length.max(1)],
            index: 0,
            filtered: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.index];
        self.filtered = flush(output * (1.0 - DAMPING) + self.filtered * DAMPING);
        self.buffer[self.index] = input + self.filtered * COMB_FEEDBACK;
        self.index = (self.index + 1) % self.buffer.len();
        output
    }
}

struct AllPass {
    buffer: Vec<f32>,
    index: usize,
}

impl AllPass {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; length.max(1)],
            index: 0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.buffer[self.index];
        self.buffer[self.index] = flush(input + delayed * ALLPASS_FEEDBACK);
        self.index = (self.index + 1) % self.buffer.len();
        delayed - input
    }
}

/// Very small values are flushed to zero so a decaying tail never reaches
/// denormal numbers, which are slow on some CPUs.
#[inline]
fn flush(value: f32) -> f32 {
    if value.abs() < 1e-20 { 0.0 } else { value }
}

struct Channel {
    combs: Vec<Comb>,
    allpasses: Vec<AllPass>,
}

impl Channel {
    fn new(rate: u32, spread: usize) -> Self {
        let scale = |length: usize| ((length + spread) as f32 * rate as f32 / TUNING_RATE) as usize;
        Self {
            combs: COMB_TUNING.iter().map(|&n| Comb::new(scale(n))).collect(),
            allpasses: ALLPASS_TUNING
                .iter()
                .map(|&n| AllPass::new(scale(n)))
                .collect(),
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let mut output = 0.0;
        for comb in &mut self.combs {
            output += comb.process(input);
        }
        for allpass in &mut self.allpasses {
            output = allpass.process(output);
        }
        output
    }
}

/// A stereo reverb at a fixed sample rate.
pub struct Reverb {
    left: Channel,
    right: Channel,
}

impl Reverb {
    pub fn new(rate: u32) -> Self {
        Self {
            left: Channel::new(rate, 0),
            right: Channel::new(rate, STEREO_SPREAD),
        }
    }

    /// The reverb's output (wet only) for one input frame.
    #[inline]
    pub fn process(&mut self, left: f32, right: f32) -> (f32, f32) {
        let input = (left + right) * INPUT_GAIN;
        (
            self.left.process(input) * WET,
            self.right.process(input) * WET,
        )
    }
}

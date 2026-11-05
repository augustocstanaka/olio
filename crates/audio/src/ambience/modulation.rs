//! Slow movement for every layer, so the music keeps breathing while its
//! notes repeat: how loud each layer is, how bright, where it sits across
//! the stereo field and how much of it goes into the hall all drift, each
//! on its own slow wave and a wandering walk. The waves' periods are
//! different primes, so no two ever line up and the whole never repeats
//! in a way the ear catches. The bass, the drone and the drums keep to
//! the middle; only their level and brightness move.
//!
//! The movement is worked out every [`CONTROL`] frames of the music's own
//! clock, not per block, so a render is the same whatever the buffer
//! size. Nothing here allocates.

use std::f32::consts::TAU;

use super::{Instrument, Rng};

/// How often the movement is worked out, in frames: rarely enough to cost
/// nothing, often enough that no step is heard.
pub(super) const CONTROL: u64 = 64;

/// What moves in each layer.
const ASPECTS: usize = 4;
const LEVEL: usize = 0;
const BRIGHTNESS: usize = 1;
const PAN: usize = 2;
const SEND: usize = 3;

/// The waves' periods in seconds, one per layer and aspect: the primes
/// from 31 to 199, so no two line up.
const PERIODS: [f32; Instrument::COUNT * ASPECTS] = [
    31.0, 37.0, 41.0, 43.0, 47.0, 53.0, 59.0, 61.0, 67.0, 71.0, 73.0, 79.0, 83.0, 89.0, 97.0,
    101.0, 103.0, 107.0, 109.0, 113.0, 127.0, 131.0, 137.0, 139.0, 149.0, 151.0, 157.0, 163.0,
    167.0, 173.0, 179.0, 181.0, 191.0, 193.0, 197.0, 199.0,
];
/// How much of a drift is its wave; the rest is its walk.
const WAVE: f32 = 0.65;
/// The walk: how long it takes to wander back towards the middle, how far
/// it strays (as a spread, about), and how long its steps are smoothed.
const WALK_RETURN_SECONDS: f32 = 20.0;
const WALK_SPREAD: f32 = 0.5;
const WALK_SMOOTH_SECONDS: f32 = 8.0;

/// How far each aspect moves at most: the level in decibels either way,
/// the brightness as a share of the layer darkened, the place as a pan,
/// and the hall send as a share of itself.
fn depths(instrument: Instrument) -> [f32; ASPECTS] {
    match instrument {
        Instrument::Bass => [1.0, 0.1, 0.0, 0.2],
        Instrument::Drone | Instrument::Drums => [1.5, 0.15, 0.0, 0.25],
        Instrument::Pad => [2.5, 0.2, 0.15, 0.3],
        Instrument::Strings => [2.5, 0.2, 0.2, 0.3],
        Instrument::Harp | Instrument::Bells | Instrument::Piano | Instrument::Violin => {
            [2.5, 0.2, 0.3, 0.3]
        }
    }
}

/// One slowly moving value, -1 to 1: a wave plus a walk.
#[derive(Clone, Copy, Debug)]
struct Drift {
    phase: f32,
    step: f32,
    walk: f32,
    smooth: f32,
}

impl Drift {
    fn tick(&mut self, rng: &mut Rng, walk: &Walk) -> f32 {
        self.phase = (self.phase + self.step).fract();
        self.walk += rng.range(-1.0, 1.0) * walk.step - self.walk * walk.pull;
        self.smooth += (self.walk - self.smooth) * walk.smoothing;
        let wave = (self.phase * TAU).sin();
        (WAVE * wave + (1.0 - WAVE) * (self.smooth / WALK_SPREAD).clamp(-1.0, 1.0)).clamp(-1.0, 1.0)
    }
}

/// The walk's constants for a control step.
#[derive(Clone, Copy, Debug)]
struct Walk {
    step: f32,
    pull: f32,
    smoothing: f32,
}

/// Where a layer has drifted to now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Sway {
    /// A gain about 1.
    pub level: f32,
    /// A share of the layer to darken more (or, below 0, less) than its
    /// place does.
    pub darkness: f32,
    /// A pan, -1 left to 1 right, about the middle.
    pub pan: f32,
    /// A factor on the layer's hall send, about 1.
    pub send: f32,
}

impl Sway {
    pub(super) const STILL: Self = Self {
        level: 1.0,
        darkness: 0.0,
        pan: 0.0,
        send: 1.0,
    };
}

/// Every layer's drifts.
pub(super) struct Motion {
    rng: Rng,
    walk: Walk,
    drifts: [[Drift; ASPECTS]; Instrument::COUNT],
    now: [Sway; Instrument::COUNT],
}

impl Motion {
    pub(super) fn new(rate: u32, seed: u64) -> Self {
        let ticks_per_second = rate as f32 / CONTROL as f32;
        let pull = 1.0 / (WALK_RETURN_SECONDS * ticks_per_second);
        // A walk pulled back by `pull` strays about sqrt(step² / 6 pull).
        let walk = Walk {
            step: WALK_SPREAD * (6.0 * pull).sqrt(),
            pull,
            smoothing: 1.0 / (WALK_SMOOTH_SECONDS * ticks_per_second),
        };
        let mut rng = Rng::new(seed);
        let mut drifts = [[Drift {
            phase: 0.0,
            step: 0.0,
            walk: 0.0,
            smooth: 0.0,
        }; ASPECTS]; Instrument::COUNT];
        for (layer, aspects) in drifts.iter_mut().enumerate() {
            for (aspect, drift) in aspects.iter_mut().enumerate() {
                drift.phase = rng.unit();
                drift.step = 1.0 / (PERIODS[layer * ASPECTS + aspect] * ticks_per_second);
            }
        }
        Self {
            rng,
            walk,
            drifts,
            now: [Sway::STILL; Instrument::COUNT],
        }
    }

    /// Moves every drift on one control step.
    pub(super) fn tick(&mut self) {
        for instrument in Instrument::ALL {
            let index = instrument.index();
            let depth = depths(instrument);
            let mut value = [0.0; ASPECTS];
            for (aspect, drift) in self.drifts[index].iter_mut().enumerate() {
                value[aspect] = drift.tick(&mut self.rng, &self.walk) * depth[aspect];
            }
            self.now[index] = Sway {
                level: 10f32.powf(value[LEVEL] / 20.0),
                darkness: value[BRIGHTNESS],
                pan: value[PAN],
                send: 1.0 + value[SEND],
            };
        }
    }

    /// Where a layer is now.
    pub(super) fn sway(&self, instrument: usize) -> &Sway {
        &self.now[instrument]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layer's sway every second for `seconds`.
    fn sways(instrument: Instrument, seconds: usize) -> Vec<Sway> {
        let rate = 48_000;
        let mut motion = Motion::new(rate, 3);
        let per_second = rate as usize / CONTROL as usize;
        (0..seconds)
            .map(|_| {
                for _ in 0..per_second {
                    motion.tick();
                }
                *motion.sway(instrument.index())
            })
            .collect()
    }

    #[test]
    fn every_layer_drifts_within_its_depth_and_never_jumps() {
        for instrument in Instrument::ALL {
            let depth = depths(instrument);
            let sways = sways(instrument, 600);
            for sway in &sways {
                let db = 20.0 * sway.level.log10();
                assert!(db.abs() <= depth[LEVEL] + 1e-3, "{instrument:?} {db}");
                assert!(sway.darkness.abs() <= depth[BRIGHTNESS] + 1e-6);
                assert!(sway.pan.abs() <= depth[PAN] + 1e-6);
                assert!((sway.send - 1.0).abs() <= depth[SEND] + 1e-6);
            }
            // It moves: over ten minutes the level covers much of its range.
            let (low, high) = sways.iter().fold((f32::MAX, f32::MIN), |(low, high), s| {
                (low.min(s.level), high.max(s.level))
            });
            assert!(
                20.0 * (high / low).log10() > depth[LEVEL],
                "{instrument:?} barely moves: {low} {high}"
            );
            // Slowly: a swell, never a step; a second moves the level
            // well under half a decibel.
            for pair in sways.windows(2) {
                let step = 20.0 * (pair[1].level / pair[0].level).log10();
                assert!(step.abs() < 0.4, "{instrument:?} steps {step} dB");
            }
        }
    }

    #[test]
    fn the_bass_drone_and_drums_keep_to_the_middle() {
        for instrument in [Instrument::Bass, Instrument::Drone, Instrument::Drums] {
            assert!(sways(instrument, 120).iter().all(|s| s.pan == 0.0));
        }
        assert!(
            sways(Instrument::Harp, 120)
                .iter()
                .any(|s| s.pan.abs() > 0.05)
        );
    }

    #[test]
    fn the_same_seed_moves_the_same_way() {
        assert_eq!(sways(Instrument::Piano, 90), sways(Instrument::Piano, 90));
        let other = {
            let mut motion = Motion::new(48_000, 4);
            for _ in 0..48_000 * 90 / CONTROL as usize {
                motion.tick();
            }
            *motion.sway(Instrument::Piano.index())
        };
        assert_ne!(
            sways(Instrument::Piano, 90)[89],
            other,
            "another seed, another way"
        );
    }
}

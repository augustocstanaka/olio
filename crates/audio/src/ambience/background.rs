//! The background: the sound under the music, made from filtered noise
//! with slow movement in it. Rain is a hiss with drops on top; wind is low
//! noise that gusts, a winter wind thinner with a whistle; waves swell and
//! break; a fire roars low and crackles in clusters; thunder rolls far off
//! over soft rain; a forest is leaves in a breeze and a bird now and then;
//! a desert is a dry wind that lifts sand in its gusts and sometimes sings.
//!
//! Everything lives in [`Bed`], built once; a frame allocates nothing.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use super::dsp::{BandPass, OnePole, coefficient, decay, pan_gains};
use super::{Background, Rng};
use crate::mixer::Ramp;

/// A short burst of noise through a band-pass, dying away: a raindrop, or
/// a crackle in a fire.
#[derive(Clone, Copy, Default)]
struct Burst {
    env: f32,
    decay: f32,
    filter: BandPass,
    pan: f32,
}

impl Burst {
    #[inline]
    fn live(&self) -> bool {
        self.env >= 0.001
    }
}

/// A bird's chirp: a sine sliding in pitch under a smooth envelope.
#[derive(Clone, Copy, Default)]
struct Chirp {
    phase: f32,
    hz: f32,
    /// How far the pitch slides by the end: 0.3 is up 30 %.
    sweep: f32,
    /// Frames sounded, of `length`.
    t: u32,
    length: u32,
    level: f32,
    pan: f32,
}

impl Chirp {
    #[inline]
    fn live(&self) -> bool {
        self.t < self.length
    }
}

const BURSTS: usize = 12;
const CHIRPS: usize = 3;

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
    /// A fire's flame, a forest's leaves.
    body: [BandPass; 2],
    bursts: [Burst; BURSTS],
    /// A fire's heat: each crackle makes the next more likely, so they
    /// come in clusters.
    heat: f32,
    heat_decay: f32,
    /// Waves: how far through this wave (0 to 1), how fast it goes, how
    /// big it is and which side it breaks on.
    swell: f32,
    swell_step: f32,
    swell_height: f32,
    swell_pan: f32,
    /// Thunder: frames until the next roll, the roll's level, how it rises
    /// and dies, and how bright it starts.
    roll_wait: u32,
    roll: f32,
    roll_peak: f32,
    roll_rise: f32,
    roll_decay: f32,
    roll_rising: bool,
    roll_bright: f32,
    roll_low: [OnePole; 2],
    roll_smooth: [OnePole; 2],
    flutter: OnePole,
    /// A forest's birds: frames until the next chirp, chirps left in this
    /// song, and the song's own voice.
    chirps: [Chirp; CHIRPS],
    bird_wait: u32,
    song_left: u8,
    song_hz: f32,
    song_sweep: f32,
    song_frames: u32,
    song_gap: f32,
    song_pan: f32,
    song_level: f32,
    /// The background's own gain: a change fades out what plays, then
    /// fades in what was asked for, so nothing clicks.
    gain: Ramp,
    playing: Background,
    wanted: Background,
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
            body: [BandPass::default(); 2],
            bursts: [Burst::default(); BURSTS],
            heat: 0.0,
            heat_decay: decay(0.3, rate),
            swell: 0.0,
            swell_step: 0.0,
            swell_height: 0.8,
            swell_pan: 0.0,
            roll_wait: 0,
            roll: 0.0,
            roll_peak: 0.0,
            roll_rise: 0.0,
            roll_decay: 0.0,
            roll_rising: false,
            roll_bright: 0.5,
            roll_low: [OnePole::default(); 2],
            roll_smooth: [OnePole::default(); 2],
            flutter: OnePole::default(),
            chirps: [Chirp::default(); CHIRPS],
            bird_wait: 0,
            song_left: 0,
            song_hz: 3000.0,
            song_sweep: 0.2,
            song_frames: 1,
            song_gap: 0.1,
            song_pan: 0.0,
            song_level: 0.5,
            gain: Ramp::at(0.0),
            playing: Background::None,
            wanted: Background::None,
        };
        for filter in &mut bed.whistle {
            filter.tune(800.0, 12.0, rate);
        }
        bed
    }

    pub(super) fn set_background(&mut self, background: Background) {
        if background == self.wanted {
            return;
        }
        self.wanted = background;
        if self.gain.is_silent() {
            self.switch();
        } else if background == self.playing {
            // Asked back for what is still fading out.
            self.gain.towards(1.0, self.rate / 4);
        } else {
            self.gain.towards(0.0, self.rate / 4);
        }
    }

    /// Starts what was asked for, once what played has faded out.
    fn switch(&mut self) {
        self.playing = self.wanted;
        let rate = self.rate;
        match self.playing {
            Background::Fire => {
                for filter in &mut self.body {
                    filter.tune(900.0, 0.7, rate);
                }
            }
            Background::Forest => {
                for filter in &mut self.body {
                    filter.tune(2600.0, 0.6, rate);
                }
                self.song_left = 0;
                self.bird_wait = (self.rng.range(0.5, 3.0) * rate as f32) as u32;
            }
            Background::Thunder => {
                self.roll = 0.0;
                self.roll_rising = false;
                self.roll_wait = (self.rng.range(4.0, 10.0) * rate as f32) as u32;
            }
            Background::Waves => {
                self.swell = self.rng.range(0.0, 0.3);
                self.swell_step = 1.0 / (9.0 * rate as f32);
            }
            _ => {}
        }
        if self.playing != Background::None {
            self.gain.towards(1.0, rate / 2);
        }
    }

    /// Moves the gusts along: a slow drift towards a target picked every
    /// few seconds.
    #[inline]
    fn breathe(&mut self) {
        if self.gust_left == 0 {
            self.gust_target = self.rng.range(0.15, 1.0);
            self.gust_left = (self.rng.range(1.5, 6.0) * self.rate as f32) as u32;
            let (low, high) = if self.playing == Background::Desert {
                (1100.0, 2200.0)
            } else {
                (500.0, 1400.0)
            };
            let hz = self.rng.range(low, high);
            for filter in &mut self.whistle {
                filter.tune(hz, 14.0, self.rate);
            }
        }
        self.gust_left -= 1;
        self.gust += (self.gust_target - self.gust) * 0.00002;
    }

    /// The next frame of the background.
    #[inline]
    pub(super) fn next(&mut self) -> (f32, f32) {
        if self.playing != self.wanted && self.gain.is_silent() {
            self.switch();
        }
        let gain = self.gain.next();
        if gain == 0.0 && self.gain.is_silent() {
            return (0.0, 0.0);
        }
        self.breathe();
        let out = match self.playing {
            Background::None => [0.0; 2],
            Background::Rain => self.rain(1.0),
            Background::Thunder => {
                let rain = self.rain(0.55);
                let roll = self.thunder();
                [rain[0] + roll[0], rain[1] + roll[1]]
            }
            Background::Wind => self.wind(false),
            Background::Winter => self.wind(true),
            Background::Waves => self.waves(),
            Background::Forest => self.forest(),
            Background::Desert => self.desert(),
            Background::Fire => self.fire(),
        };
        (out[0] * gain, out[1] * gain)
    }

    /// The bursts that are sounding, at `level`.
    #[inline]
    fn bursts(&mut self, level: f32) -> [f32; 2] {
        let mut out = [0.0; 2];
        for burst in &mut self.bursts {
            if burst.live() {
                let sound = burst.filter.next(self.rng.signed()) * burst.env;
                burst.env *= burst.decay;
                let (l, r) = pan_gains(burst.pan);
                out[0] += sound * l * level;
                out[1] += sound * r * level;
            }
        }
        out
    }

    fn free_burst(&mut self) -> Option<&mut Burst> {
        self.bursts.iter_mut().find(|burst| !burst.live())
    }

    fn rain(&mut self, level: f32) -> [f32; 2] {
        let rate = self.rate;
        let hiss = coefficient(1200.0, rate);
        let soft = coefficient(5500.0, rate);
        let mut out = [0.0f32; 2];
        for (side, sample) in out.iter_mut().enumerate() {
            let noise = self.rng.signed();
            let high = self.high[side].high(noise, hiss);
            *sample = self.low[side].low(high, soft) * 0.32 * level;
        }
        // About twenty drops a second, each a few milliseconds.
        if self.rng.chance(22.0 * level / rate as f32) {
            let (env, length, hz, pan) = (
                self.rng.range(0.3, 1.0),
                self.rng.range(0.004, 0.02),
                self.rng.range(1800.0, 6500.0),
                self.rng.signed() * 0.8,
            );
            if let Some(drop) = self.free_burst() {
                drop.env = env;
                drop.decay = 1.0 - 1.0 / (length * rate as f32);
                drop.filter = BandPass::default();
                drop.filter.tune(hz, 4.0, rate);
                drop.pan = pan;
            }
        }
        let drops = self.bursts(0.85 * level);
        [out[0] + drops[0], out[1] + drops[1]]
    }

    fn wind(&mut self, winter: bool) -> [f32; 2] {
        let cutoff = if winter {
            250.0 + 900.0 * self.gust
        } else {
            120.0 + 600.0 * self.gust
        };
        let body = coefficient(cutoff, self.rate);
        let level = (0.25 + 0.75 * self.gust) * if winter { 0.55 } else { 0.8 };
        let whistle_level = if winter {
            0.25 + 0.5 * self.gust
        } else {
            0.12 * self.gust
        };
        let mut out = [0.0f32; 2];
        for (side, sample) in out.iter_mut().enumerate() {
            let noise = self.rng.signed();
            let rumble = self.low[side].low(noise, body);
            let rumble = self.smooth[side].low(rumble, body);
            let whistle = self.whistle[side].next(noise) * whistle_level;
            *sample = (rumble * 3.5 * level + whistle) * 0.7;
        }
        out
    }

    /// Waves: noise that swells over a few seconds, brightening as it
    /// breaks, with foam at the crest, then washes back. Never quite
    /// silent between waves.
    fn waves(&mut self) -> [f32; 2] {
        let rate = self.rate;
        self.swell += self.swell_step;
        if self.swell >= 1.0 {
            self.swell -= 1.0;
            self.swell_step = 1.0 / (self.rng.range(7.0, 13.0) * rate as f32);
            self.swell_height = self.rng.range(0.55, 1.0);
            self.swell_pan = self.rng.signed() * 0.5;
        }
        let p = self.swell;
        let shape = if p < 0.45 {
            let rise = (p / 0.45 * FRAC_PI_2).sin();
            rise * rise
        } else {
            ((1.0 - p) / 0.55).powf(1.5)
        };
        let env = 0.12 + 0.88 * shape * self.swell_height;
        let body = coefficient(220.0 + 1300.0 * env, rate);
        let foam = coefficient(2500.0, rate);
        let (left, right) = pan_gains(self.swell_pan);
        let sides = [
            left * std::f32::consts::SQRT_2,
            right * std::f32::consts::SQRT_2,
        ];
        let mut out = [0.0f32; 2];
        for (side, sample) in out.iter_mut().enumerate() {
            let noise = self.rng.signed();
            let wash = self.low[side].low(noise, body);
            let wash = self.smooth[side].low(wash, body);
            let spray = self.high[side].high(noise, foam);
            let spread = 0.6 + 0.4 * sides[side];
            *sample = (wash * 1.1 * env + spray * 0.05 * env * env) * spread;
        }
        out
    }

    /// A fire: a low, breathing roar, a soft flame hiss, and crackles in
    /// clusters, with a bigger pop of wood now and then.
    fn fire(&mut self) -> [f32; 2] {
        let rate = self.rate;
        let roar = coefficient(110.0 + 90.0 * self.gust, rate);
        let breath = 0.55 + 0.45 * self.gust;
        let mut out = [0.0f32; 2];
        for (side, sample) in out.iter_mut().enumerate() {
            let noise = self.rng.signed();
            let low = self.low[side].low(noise, roar);
            let low = self.smooth[side].low(low, roar);
            let flame = self.body[side].next(noise);
            *sample = low * 2.2 * breath + flame * 0.04;
        }
        self.heat *= self.heat_decay;
        if self.rng.chance((2.5 + 30.0 * self.heat) / rate as f32) {
            self.heat = (self.heat + 0.4).min(1.0);
            let pop = self.rng.chance(0.12);
            let (env, length, hz, q, pan) = if pop {
                (
                    self.rng.range(0.6, 1.0),
                    self.rng.range(0.015, 0.04),
                    self.rng.range(700.0, 1600.0),
                    3.0,
                    self.rng.signed() * 0.5,
                )
            } else {
                (
                    self.rng.range(0.2, 0.7),
                    self.rng.range(0.002, 0.008),
                    self.rng.range(1800.0, 6000.0),
                    1.5,
                    self.rng.signed() * 0.6,
                )
            };
            if let Some(crackle) = self.free_burst() {
                crackle.env = env;
                crackle.decay = decay(length, rate);
                // From rest, so the last crackle's ringing cannot add to it.
                crackle.filter = BandPass::default();
                crackle.filter.tune(hz, q, rate);
                crackle.pan = pan;
            }
        }
        let crackles = self.bursts(1.6);
        [out[0] + crackles[0], out[1] + crackles[1]]
    }

    /// Thunder far off: now and then a roll rises over a moment and
    /// rumbles away over several seconds, its loudness fluttering and its
    /// colour darkening as it goes.
    fn thunder(&mut self) -> [f32; 2] {
        let rate = self.rate;
        if self.roll_wait > 0 {
            self.roll_wait -= 1;
        } else if !self.roll_rising {
            self.roll_rising = true;
            self.roll_peak = self.rng.range(0.5, 1.0);
            self.roll_rise = 1.0 / (self.rng.range(0.3, 0.9) * rate as f32);
            self.roll_decay = decay(self.rng.range(5.0, 9.0), rate);
            self.roll_bright = self.rng.range(0.4, 1.0);
            self.roll_wait = (self.rng.range(20.0, 55.0) * rate as f32) as u32;
        }
        if self.roll_rising {
            self.roll += self.roll_rise * self.roll_peak;
            if self.roll >= self.roll_peak {
                self.roll_rising = false;
            }
        } else {
            self.roll *= self.roll_decay;
        }
        if self.roll < 1e-5 && !self.roll_rising {
            self.roll = 0.0;
            return [0.0; 2];
        }
        let fresh = self.roll / self.roll_peak.max(1e-3);
        let tone = coefficient(60.0 + 320.0 * self.roll_bright * fresh, rate);
        let flutter_rate = coefficient(4.0, rate);
        let flutter =
            (0.55 + 30.0 * self.flutter.low(self.rng.signed(), flutter_rate)).clamp(0.05, 1.6);
        let mut out = [0.0f32; 2];
        for (side, sample) in out.iter_mut().enumerate() {
            let noise = self.rng.signed();
            let rumble = self.roll_low[side].low(noise, tone);
            let rumble = self.roll_smooth[side].low(rumble, tone);
            *sample = rumble * 9.0 * self.roll * flutter;
        }
        out
    }

    /// A forest: an airy hiss of leaves that swells with the breeze, and
    /// now and then a bird: a short song of a few chirps from one place.
    fn forest(&mut self) -> [f32; 2] {
        let rate = self.rate;
        let leaves = 0.12 + 0.55 * self.gust * self.gust;
        let hush = coefficient(300.0, rate);
        let mut out = [0.0f32; 2];
        for (side, sample) in out.iter_mut().enumerate() {
            let noise = self.rng.signed();
            let air = self.body[side].next(noise);
            let low = self.low[side].low(noise, hush);
            *sample = air * leaves * 0.35 + low * 0.3;
        }
        if self.bird_wait > 0 {
            self.bird_wait -= 1;
        } else {
            if self.song_left == 0 {
                // A new bird, somewhere: its own pitch, call and distance.
                self.song_left = 2 + self.rng.below(4) as u8;
                self.song_hz = self.rng.range(2200.0, 4200.0);
                self.song_sweep = if self.rng.chance(0.5) {
                    self.rng.range(0.15, 0.4)
                } else {
                    -self.rng.range(0.15, 0.35)
                };
                self.song_frames = (self.rng.range(0.05, 0.14) * rate as f32) as u32;
                self.song_gap = self.rng.range(0.08, 0.22);
                self.song_pan = self.rng.signed() * 0.8;
                self.song_level = self.rng.range(0.35, 1.0);
            }
            let hz = self.song_hz * self.rng.range(0.94, 1.06);
            let (sweep, length, level, pan) = (
                self.song_sweep,
                self.song_frames,
                self.song_level,
                self.song_pan,
            );
            if let Some(chirp) = self.chirps.iter_mut().find(|chirp| !chirp.live()) {
                *chirp = Chirp {
                    phase: 0.0,
                    hz,
                    sweep,
                    t: 0,
                    length,
                    level,
                    pan,
                };
            }
            self.song_left -= 1;
            let seconds = if self.song_left > 0 {
                self.song_frames as f32 / rate as f32 + self.song_gap
            } else {
                self.rng.range(3.0, 12.0)
            };
            self.bird_wait = (seconds * rate as f32) as u32;
        }
        for chirp in &mut self.chirps {
            if chirp.live() {
                let p = chirp.t as f32 / chirp.length as f32;
                chirp.phase =
                    (chirp.phase + chirp.hz * (1.0 + chirp.sweep * p) / rate as f32).fract();
                let env = (PI * p).sin();
                let sound = (chirp.phase * TAU).sin() * env * env * chirp.level * 0.07;
                let (l, r) = pan_gains(chirp.pan);
                out[0] += sound * l;
                out[1] += sound * r;
                chirp.t += 1;
            }
        }
        out
    }

    /// A desert: a thin, dry wind; its strong gusts lift a hiss of sand
    /// and make the dunes sing.
    fn desert(&mut self) -> [f32; 2] {
        let rate = self.rate;
        let body = coefficient(300.0 + 900.0 * self.gust, rate);
        let sand_tone = coefficient(4500.0, rate);
        let level = (0.2 + 0.8 * self.gust) * 0.45;
        let sand = self.gust * self.gust * self.gust * 0.22;
        let whistle_level = 0.18 * self.gust * self.gust;
        let mut out = [0.0f32; 2];
        for (side, sample) in out.iter_mut().enumerate() {
            let noise = self.rng.signed();
            let rumble = self.low[side].low(noise, body);
            let rumble = self.smooth[side].low(rumble, body);
            let grains = self.high[side].high(noise, sand_tone) * sand;
            let whistle = self.whistle[side].next(noise) * whistle_level;
            *sample = (rumble * 3.5 * level + grains + whistle) * 0.7;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn render(background: Background, seed: u64, seconds: usize) -> Vec<(f32, f32)> {
        let mut bed = Bed::new(RATE, seed);
        bed.set_background(background);
        (0..RATE as usize * seconds).map(|_| bed.next()).collect()
    }

    /// The RMS of each `window`-frame stretch.
    fn levels(frames: &[(f32, f32)], window: usize) -> Vec<f32> {
        frames
            .chunks(window)
            .map(|chunk| {
                let sum: f32 = chunk.iter().map(|(l, r)| l * l + r * r).sum();
                (sum / (2 * chunk.len()) as f32).sqrt()
            })
            .collect()
    }

    fn median(values: &[f32]) -> f32 {
        let mut sorted = values.to_vec();
        sorted.sort_by(f32::total_cmp);
        sorted[sorted.len() / 2]
    }

    #[test]
    fn each_background_sounds_within_bounds_and_none_is_silent() {
        for background in Background::ALL {
            let frames = render(background, 4, 4);
            let tail = &frames[RATE as usize..];
            let peak = tail
                .iter()
                .map(|(l, r)| l.abs().max(r.abs()))
                .fold(0.0, f32::max);
            let rms = levels(tail, tail.len())[0];
            if background == Background::None {
                assert_eq!(peak, 0.0);
            } else {
                assert!(
                    peak > 0.01 && peak < 1.0,
                    "{}: peak {peak}",
                    background.name()
                );
                assert!(
                    (0.004..0.2).contains(&rms),
                    "{}: rms {rms}",
                    background.name()
                );
                assert!(tail.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
            }
        }
    }

    #[test]
    fn thunder_rolls_now_and_then_over_the_rain() {
        let frames = render(Background::Thunder, 7, 40);
        let levels = levels(&frames, RATE as usize / 2);
        let loudest = levels.iter().copied().fold(0.0, f32::max);
        let typical = median(&levels);
        assert!(loudest > typical * 2.5, "{loudest} over {typical}");
    }

    #[test]
    fn waves_swell_and_wash_back() {
        let frames = render(Background::Waves, 3, 30);
        let levels = levels(&frames[RATE as usize..], RATE as usize / 2);
        let loudest = levels.iter().copied().fold(0.0, f32::max);
        let quietest = levels.iter().copied().fold(f32::MAX, f32::min);
        assert!(loudest > quietest * 2.0, "{loudest} vs {quietest}");
        // Several crests: halves louder than both neighbours, well above
        // the quietest.
        let crests = levels
            .windows(3)
            .filter(|w| w[1] > w[0] && w[1] >= w[2] && w[1] > quietest * 1.6)
            .count();
        assert!((2..=8).contains(&crests), "{crests} crests in 30 s");
    }

    #[test]
    fn a_forest_has_sparse_birds_and_a_fire_crackles_in_clusters() {
        let mut bed = Bed::new(RATE, 9);
        bed.set_background(Background::Forest);
        let mut chirps = 0;
        for _ in 0..RATE * 30 {
            let before = bed.chirps.iter().filter(|c| c.live()).count();
            bed.next();
            if bed.chirps.iter().filter(|c| c.live()).count() > before {
                chirps += 1;
            }
        }
        assert!((4..=60).contains(&chirps), "{chirps} chirps in 30 s");

        let mut bed = Bed::new(RATE, 9);
        bed.set_background(Background::Fire);
        let mut crackles = 0;
        for _ in 0..RATE * 10 {
            let before = bed.bursts.iter().filter(|b| b.live()).count();
            bed.next();
            if bed.bursts.iter().filter(|b| b.live()).count() > before {
                crackles += 1;
            }
        }
        assert!(
            (20..=400).contains(&crackles),
            "{crackles} crackles in 10 s"
        );
    }

    #[test]
    fn a_change_fades_across_and_changing_back_returns() {
        let mut bed = Bed::new(RATE, 2);
        bed.set_background(Background::Rain);
        for _ in 0..RATE {
            bed.next();
        }
        bed.set_background(Background::Fire);
        assert_eq!(bed.playing, Background::Rain, "rain fades out first");
        for _ in 0..RATE {
            bed.next();
        }
        assert_eq!(bed.playing, Background::Fire);
        assert_eq!(bed.gain.target(), 1.0);

        // Back to fire before the rain has gone: the fire comes back up.
        bed.set_background(Background::Rain);
        bed.next();
        bed.set_background(Background::Fire);
        for _ in 0..RATE {
            bed.next();
        }
        assert_eq!(bed.playing, Background::Fire);
        assert_eq!(bed.gain.target(), 1.0);
    }
}

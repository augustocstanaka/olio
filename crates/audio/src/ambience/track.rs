//! Rendering the ambience to a track: the mixer the live bed plays
//! through, run offline, so a track sounds like the bed, reverb and all.
//! Not for the audio thread: it allocates the whole track.

use super::AmbienceSettings;
use crate::{Clip, Engine};

use std::collections::VecDeque;

/// The rate tracks are rendered at.
pub const TRACK_RATE: u32 = 48_000;
/// A track's loudness: -20 dBFS RMS, a little under where a tile's own
/// normalizing puts every sound.
const LOUDNESS: f32 = 0.1;
/// No peak goes over -1 dBFS; the rare one that would (a crackle in a
/// fire) is turned down by the limiter.
const CEILING: f32 = 0.891;
/// The limiter turns down this far ahead of a peak, and takes this long to
/// come back up.
const LOOK_AHEAD_SECONDS: f32 = 0.002;
const RELEASE_SECONDS: f32 = 0.08;
/// At the end the bed fades out over a second and the reverb rings on.
const TAIL_SECONDS: f32 = 2.0;
/// The very end is faded to silence, whatever still rings.
const LAST_FADE_SECONDS: f32 = 0.5;
const CHUNK: usize = 4096;

/// Renders `seconds` of the ambience with `settings` (switched on and not
/// ducked, whatever they say) into a stereo clip that fades in over a
/// second, fades out at the end, and is about -20 dBFS RMS with no peak
/// over -1 dBFS. The same settings render the same track.
///
/// `progress` hears how far the render has got, 0 to 1, after each chunk;
/// returning false stops it, and then nothing is returned.
pub fn render_track(
    settings: &AmbienceSettings,
    seconds: f32,
    mut progress: impl FnMut(f32) -> bool,
) -> Option<Clip> {
    let rate = TRACK_RATE;
    let seconds = if seconds.is_finite() { seconds } else { 0.0 };
    let total = (seconds.max(TAIL_SECONDS + 1.0) * rate as f32) as usize;
    let playing = total - (TAIL_SECONDS * rate as f32) as usize;
    let on = AmbienceSettings {
        enabled: true,
        duck: false,
        ..*settings
    };
    let (mut engine, mut mixer) = Engine::offline(rate);
    engine.set_ambience(on);
    let mut samples = vec![0.0f32; total * 2];
    let mut done = 0;
    while done < total {
        let end = if done < playing { playing } else { total };
        let frames = CHUNK.min(end - done);
        mixer.render(&mut samples[done * 2..(done + frames) * 2], 2);
        done += frames;
        if done == playing {
            engine.set_ambience(AmbienceSettings {
                enabled: false,
                ..on
            });
        }
        if !progress(done as f32 / total as f32) {
            return None;
        }
    }

    let fade = ((LAST_FADE_SECONDS * rate as f32) as usize).min(total);
    let (frames, _) = samples[(total - fade) * 2..].as_chunks_mut::<2>();
    for (i, frame) in frames.iter_mut().enumerate() {
        let gain = 1.0 - (i + 1) as f32 / fade as f32;
        frame[0] *= gain;
        frame[1] *= gain;
    }
    normalize(&mut samples, rate);
    Some(Clip::new(rate, 2, samples))
}

/// Brings interleaved stereo `samples` to [`LOUDNESS`], limiting any peak
/// that would then pass [`CEILING`].
fn normalize(samples: &mut [f32], rate: u32) {
    let squares: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    let rms = (squares / samples.len().max(1) as f64).sqrt() as f32;
    if rms < 1e-7 {
        return;
    }
    let gain = (LOUDNESS / rms).min(10_000.0);
    let (frames, _) = samples.as_chunks_mut::<2>();
    // The most a frame may be raised by without passing the ceiling.
    let allowed = |[l, r]: [f32; 2]| {
        let peak = l.abs().max(r.abs()) * gain;
        if peak > CEILING { CEILING / peak } else { 1.0 }
    };
    let ahead = ((LOOK_AHEAD_SECONDS * rate as f32) as usize).max(1);
    let release = 1.0 / (RELEASE_SECONDS * rate as f32);
    // The least allowed over the next `ahead` frames (a rising queue of
    // candidates), slowed on the way back up, then averaged over `ahead`
    // frames: every frame's gain is at most what it allows, and the gain
    // moves smoothly. A frame is read before any frame at or after it is
    // changed.
    let mut least: VecDeque<(usize, f32)> = VecDeque::with_capacity(ahead + 1);
    let mut held = 1.0f32;
    let mut recent: VecDeque<f32> = VecDeque::with_capacity(ahead + 1);
    let mut sum = 0.0f64;
    for i in 0..frames.len() + ahead {
        if let Some(&frame) = frames.get(i) {
            let value = allowed(frame);
            while least.back().is_some_and(|&(_, last)| last >= value) {
                least.pop_back();
            }
            least.push_back((i, value));
        }
        let Some(at) = i.checked_sub(ahead) else {
            continue;
        };
        while least.front().is_some_and(|&(first, _)| first < at) {
            least.pop_front();
        }
        let limit = least.front().map_or(1.0, |&(_, value)| value);
        held = limit.min(held + release);
        recent.push_back(held);
        sum += f64::from(held);
        if recent.len() > ahead {
            sum -= f64::from(recent.pop_front().unwrap_or(0.0));
        }
        let smooth = (sum / recent.len() as f64) as f32;
        let [l, r] = &mut frames[at];
        *l *= gain * smooth;
        *r *= gain * smooth;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rare_spike_is_limited_instead_of_holding_the_rest_down() {
        let rate = 48_000;
        // A quiet steady tone and one click forty times louder.
        let mut samples: Vec<f32> = (0..rate * 2)
            .flat_map(|i| {
                let s = (i as f32 * 0.05).sin() * 0.01;
                [s, s]
            })
            .collect();
        samples[rate as usize] = 0.4;
        normalize(&mut samples, rate);
        let peak = samples.iter().fold(0.0f32, |p, s| p.max(s.abs()));
        assert!(peak <= CEILING + 1e-4, "{peak}");
        // The tone away from the click is at the loudness asked for.
        let tone = &samples[..rate as usize / 2];
        let rms = (tone.iter().map(|s| s * s).sum::<f32>() / tone.len() as f32).sqrt();
        assert!((rms - LOUDNESS).abs() < LOUDNESS * 0.05, "{rms}");
    }
}

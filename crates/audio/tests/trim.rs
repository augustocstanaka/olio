//! Finding the audible part of a clip, for trimming silence at its ends.

use std::time::Duration;

use olio_audio::Clip;

const THRESHOLD: f32 = 0.001;

fn clip(samples: Vec<f32>) -> Clip {
    Clip::new(1_000, 1, samples)
}

#[test]
fn silence_at_both_ends_is_left_out() {
    let mut samples = vec![0.0; 100];
    samples.extend([0.5, -0.2, 0.0, 0.3]);
    samples.extend(vec![0.0002; 50]);
    let clip = clip(samples);
    assert_eq!(clip.audible_range(THRESHOLD, Duration::ZERO), 100..104);
}

#[test]
fn padding_keeps_a_little_either_side() {
    let mut samples = vec![0.0; 100];
    samples.push(1.0);
    samples.extend(vec![0.0; 100]);
    let clip = clip(samples);
    // 10 ms at 1 kHz is 10 frames.
    assert_eq!(
        clip.audible_range(THRESHOLD, Duration::from_millis(10)),
        90..111
    );
    assert_eq!(
        clip.audible_range(THRESHOLD, Duration::from_secs(1)),
        0..201,
        "padding stops at the ends"
    );
}

#[test]
fn any_channel_counts_as_sound() {
    let clip = Clip::new(1_000, 2, vec![0.0, 0.0, 0.0, 0.4, 0.0, 0.0]);
    assert_eq!(clip.audible_range(THRESHOLD, Duration::ZERO), 1..2);
}

#[test]
fn a_silent_clip_is_kept_whole() {
    let clip = clip(vec![0.0; 50]);
    assert_eq!(clip.audible_range(THRESHOLD, Duration::ZERO), 0..50);
}

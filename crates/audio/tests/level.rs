//! Measuring how loud a clip is, for Normalize.

use olio_audio::Clip;

#[test]
fn rms_and_peak_measure_the_frames_asked_for() {
    // A square wave of 0.5 is 0.5 RMS; the loud spike is outside the range.
    let mut samples = vec![0.5, -0.5, 0.5, -0.5];
    samples.push(0.9);
    let clip = Clip::new(48_000, 1, samples);
    assert!((clip.rms(0..4) - 0.5).abs() < 1e-6);
    assert_eq!(clip.peak(0..4), 0.5);
    assert_eq!(clip.peak(0..5), 0.9);
}

#[test]
fn stereo_counts_both_channels_and_odd_ranges_are_safe() {
    let clip = Clip::new(48_000, 2, vec![0.0, 0.8, 0.0, -0.8]);
    assert_eq!(clip.peak(0..2), 0.8);
    assert!((clip.rms(0..2) - 0.8 / 2f32.sqrt()).abs() < 1e-6);
    assert_eq!(clip.rms(5..9), 0.0);
    assert_eq!(clip.peak(1..1), 0.0);
}

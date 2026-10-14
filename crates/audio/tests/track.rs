//! Rendering the ambience to a track.

use olio_audio::ambience::Style;
use olio_audio::instruments::Recorded;
use olio_audio::{AmbienceSettings, TRACK_RATE, render_track};

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()))
}

fn rms_db(samples: &[f32]) -> f32 {
    let squares: f32 = samples.iter().map(|s| s * s).sum();
    20.0 * (squares / samples.len() as f32).sqrt().log10()
}

#[test]
fn a_track_is_as_long_as_asked_at_one_loudness_and_fades_at_both_ends() {
    let settings = Style::Campfire.settings();
    let mut reported = Vec::new();
    let clip = render_track(&settings, 6.0, &Recorded::default(), |done| {
        reported.push(done);
        true
    })
    .unwrap();
    assert_eq!(clip.rate(), TRACK_RATE);
    assert_eq!(clip.channels(), 2);
    assert_eq!(clip.frames(), TRACK_RATE as usize * 6);
    let samples = clip.samples();
    assert!(samples.iter().all(|s| s.is_finite()));
    // About -20 dBFS, whatever the style, with no peak over -1 dBFS: a
    // fire's crackles are limited rather than holding the rest down.
    assert!(
        (-22.0..=-19.0).contains(&rms_db(samples)),
        "{}",
        rms_db(samples)
    );
    assert!(peak(samples) <= 0.8915, "{}", peak(samples));
    // It fades in from silence and ends in silence.
    assert!(peak(&samples[..96]) < 0.01);
    assert_eq!(*samples.last().unwrap(), 0.0);
    assert!(peak(&samples[samples.len() - 200..]) < 0.01);
    // Progress rises to the end.
    assert!(reported.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(*reported.last().unwrap(), 1.0);
}

#[test]
fn the_same_settings_render_the_same_track_and_a_new_seed_another() {
    let settings = AmbienceSettings {
        enabled: false,
        ..Style::ForestMorning.settings()
    };
    // Long enough for the instruments to come in.
    let first = render_track(&settings, 12.0, &Recorded::default(), |_| true).unwrap();
    let again = render_track(&settings, 12.0, &Recorded::default(), |_| true).unwrap();
    assert!(first.samples() == again.samples());
    let other = AmbienceSettings {
        seed: 2,
        ..settings
    };
    let different = render_track(&other, 12.0, &Recorded::default(), |_| true).unwrap();
    assert!(first.samples() != different.samples());
}

#[test]
fn a_render_stops_when_asked() {
    let settings = Style::RainStudy.settings();
    let mut chunks = 0;
    let clip = render_track(&settings, 60.0, &Recorded::default(), |_| {
        chunks += 1;
        chunks < 3
    });
    assert!(clip.is_none());
    assert_eq!(chunks, 3);
}

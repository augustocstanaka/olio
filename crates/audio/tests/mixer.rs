//! Mixing rules, checked by rendering through an offline engine.

use std::sync::Arc;
use std::time::Duration;

use olio_audio::{AmbienceSettings, Background, Clip, Engine, MAX_VOICES, Mixer, PlayOptions};

const RATE: u32 = 48_000;
/// The mixer's shortest fade (5 ms) at `RATE`.
const DECLICK_FRAMES: usize = 240;

fn mono(rate: u32, samples: &[f32]) -> Arc<Clip> {
    Arc::new(Clip::new(rate, 1, samples.to_vec()))
}

fn constant(value: f32, frames: usize) -> Arc<Clip> {
    mono(RATE, &vec![value; frames])
}

fn render(mixer: &mut Mixer, frames: usize, channels: usize) -> Vec<f32> {
    let mut out = vec![f32::NAN; frames * channels];
    mixer.render(&mut out, channels);
    out
}

fn left(stereo: &[f32]) -> Vec<f32> {
    stereo
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| frame[0])
        .collect()
}

fn assert_close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} is not within {tolerance} of {expected}"
    );
}

#[test]
fn a_clip_at_the_output_rate_plays_sample_for_sample() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let samples = [0.1, -0.2, 0.3, -0.4];
    engine.play(mono(RATE, &samples), PlayOptions::default());
    let out = render(&mut mixer, 6, 2);
    assert_eq!(
        out,
        [
            0.1, 0.1, -0.2, -0.2, 0.3, 0.3, -0.4, -0.4, 0.0, 0.0, 0.0, 0.0
        ]
    );
}

#[test]
fn a_stereo_clip_keeps_its_channels_and_a_mono_output_gets_their_average() {
    let clip = Arc::new(Clip::new(RATE, 2, vec![0.25, -0.5, 0.25, -0.5]));
    let (mut engine, mut mixer) = Engine::offline(RATE);
    engine.play(Arc::clone(&clip), PlayOptions::default());
    assert_eq!(render(&mut mixer, 2, 2), [0.25, -0.5, 0.25, -0.5]);

    let (mut engine, mut mixer) = Engine::offline(RATE);
    engine.play(clip, PlayOptions::default());
    assert_eq!(render(&mut mixer, 2, 1), [-0.125, -0.125]);
}

#[test]
fn extra_output_channels_stay_silent() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    engine.play(constant(0.5, 10), PlayOptions::default());
    let out = render(&mut mixer, 2, 4);
    assert_eq!(out, [0.5, 0.5, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0]);
}

#[test]
fn voices_add_together_and_the_mix_is_clipped_to_full_scale() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    engine.play(constant(0.25, 10), PlayOptions::default());
    engine.play(constant(0.5, 10), PlayOptions::default());
    assert_eq!(left(&render(&mut mixer, 1, 2)), [0.75]);

    engine.play(constant(0.8, 10), PlayOptions::default());
    assert_eq!(left(&render(&mut mixer, 1, 2)), [1.0]);
}

#[test]
fn a_voice_plays_at_its_gain() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let options = PlayOptions {
        gain: 0.5,
        ..PlayOptions::default()
    };
    engine.play(constant(0.8, 10), options);
    assert_eq!(left(&render(&mut mixer, 1, 2)), [0.4]);
}

#[test]
fn a_clip_that_plays_out_ends_and_its_clip_comes_back() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let clip = constant(0.5, 100);
    let voice = engine
        .play(Arc::clone(&clip), PlayOptions::default())
        .unwrap();
    assert!(engine.poll().is_empty(), "nothing ends before rendering");

    let out = left(&render(&mut mixer, 256, 2));
    assert!(out[..100].iter().all(|&sample| sample == 0.5));
    assert!(out[100..].iter().all(|&sample| sample == 0.0));

    assert_eq!(engine.poll(), [voice]);
    assert_eq!(engine.live_voices(), 0);
    assert_eq!(engine.position(voice), None);
    // The mixer gave its reference back; only the test's own remains.
    assert_eq!(Arc::strong_count(&clip), 1);
}

#[test]
fn a_looping_clip_starts_again_until_stopped() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let options = PlayOptions {
        looping: true,
        ..PlayOptions::default()
    };
    let voice = engine.play(mono(RATE, &[0.1, 0.2, 0.3]), options).unwrap();
    let out = left(&render(&mut mixer, 7, 2));
    assert_eq!(out, [0.1, 0.2, 0.3, 0.1, 0.2, 0.3, 0.1]);
    render(&mut mixer, 10_000, 2);
    assert!(engine.poll().is_empty(), "a loop never ends by itself");

    engine.stop(voice, Duration::ZERO);
    render(&mut mixer, DECLICK_FRAMES + 1, 2);
    assert_eq!(engine.poll(), [voice]);
}

#[test]
fn looping_can_be_turned_off_while_playing() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let options = PlayOptions {
        looping: true,
        ..PlayOptions::default()
    };
    let voice = engine.play(constant(0.5, 10), options).unwrap();
    render(&mut mixer, 25, 2);
    engine.set_looping(voice, false);
    let out = left(&render(&mut mixer, 10, 2));
    assert_eq!(out, [0.5, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
    assert_eq!(engine.poll(), [voice]);
}

#[test]
fn stopping_fades_out_over_a_few_milliseconds_instead_of_clicking() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let voice = engine
        .play(constant(1.0, RATE as usize), PlayOptions::default())
        .unwrap();
    render(&mut mixer, 100, 2);
    engine.stop(voice, Duration::ZERO);
    let out = left(&render(&mut mixer, 1000, 2));

    assert!(out[0] > 0.99 && out[0] < 1.0, "no jump: {}", out[0]);
    assert!(out.windows(2).all(|pair| pair[1] <= pair[0]), "fades down");
    assert_eq!(out[DECLICK_FRAMES - 1], 0.0, "silent after 5 ms");
    assert!(out[DECLICK_FRAMES..].iter().all(|&sample| sample == 0.0));
    assert_eq!(engine.poll(), [voice]);
}

#[test]
fn a_stop_fade_lasts_as_long_as_asked() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let voice = engine
        .play(constant(1.0, RATE as usize), PlayOptions::default())
        .unwrap();
    engine.stop(voice, Duration::from_millis(100));
    let out = left(&render(&mut mixer, 6000, 2));
    assert_close(out[2399], 0.5, 0.001);
    assert_eq!(out[4799], 0.0);
    assert_eq!(engine.poll(), [voice]);
}

#[test]
fn stop_all_ends_every_voice() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let first = engine.play(constant(0.2, 10_000), PlayOptions::default());
    let second = engine.play(constant(0.2, 10_000), PlayOptions::default());
    render(&mut mixer, 10, 2);
    engine.stop_all(Duration::ZERO);
    render(&mut mixer, DECLICK_FRAMES + 1, 2);
    let mut ended = engine.poll();
    ended.sort();
    assert_eq!(ended, [first.unwrap(), second.unwrap()]);
    assert_eq!(engine.live_voices(), 0);
}

#[test]
fn stopping_a_voice_that_has_ended_is_ignored() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let voice = engine
        .play(constant(0.5, 4), PlayOptions::default())
        .unwrap();
    render(&mut mixer, 8, 2);
    assert_eq!(engine.poll(), [voice]);
    engine.stop(voice, Duration::ZERO);
    let other = engine
        .play(constant(0.5, 100), PlayOptions::default())
        .unwrap();
    let out = left(&render(&mut mixer, 50, 2));
    assert!(out.iter().all(|&sample| sample == 0.5), "{out:?}");
    assert!(engine.position(other).is_some());
}

#[test]
fn a_clip_at_another_rate_is_resampled() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    // A straight line at half the output rate: interpolation must keep it
    // straight, at half the slope per output frame.
    let line: Vec<f32> = (0..1000).map(|i| i as f32 / 1000.0).collect();
    engine.play(mono(RATE / 2, &line), PlayOptions::default());
    let out = left(&render(&mut mixer, 2100, 2));
    for (frame, &sample) in out.iter().enumerate().take(1990).skip(2) {
        assert_close(sample, frame as f32 / 2000.0, 1e-4);
    }
    assert!(out[2001..].iter().all(|&sample| sample == 0.0));
}

#[test]
fn positions_are_published_in_frames_of_the_clip() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let same_rate = engine
        .play(constant(0.1, 10_000), PlayOptions::default())
        .unwrap();
    let half_rate = engine
        .play(mono(RATE / 2, &[0.1; 10_000]), PlayOptions::default())
        .unwrap();
    assert_eq!(engine.position(same_rate), None, "not rendered yet");
    render(&mut mixer, 480, 2);
    assert_eq!(engine.position(same_rate), Some(480));
    assert_eq!(engine.position(half_rate), Some(240));
}

#[test]
fn the_engine_refuses_more_than_the_maximum_number_of_voices() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let clip = constant(0.001, 100_000);
    let voices: Vec<_> = (0..MAX_VOICES)
        .map(|_| engine.play(Arc::clone(&clip), PlayOptions::default()))
        .collect();
    assert!(voices.iter().all(Option::is_some));
    assert_eq!(engine.play(Arc::clone(&clip), PlayOptions::default()), None);

    render(&mut mixer, 10, 2);
    engine.stop(voices[0].unwrap(), Duration::ZERO);
    render(&mut mixer, DECLICK_FRAMES + 1, 2);
    assert_eq!(engine.poll(), [voices[0].unwrap()]);
    assert!(engine.play(clip, PlayOptions::default()).is_some());
    render(&mut mixer, 10, 2);
}

#[test]
fn the_master_gain_scales_the_whole_mix_without_a_jump() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    engine.play(constant(0.8, RATE as usize), PlayOptions::default());
    engine.set_master_gain(0.5);
    let out = left(&render(&mut mixer, 1000, 2));
    assert!(out[0] > 0.79, "ramps rather than jumping: {}", out[0]);
    assert_close(out[DECLICK_FRAMES], 0.4, 1e-6);
    assert_close(out[999], 0.4, 1e-6);
}

#[test]
fn a_fade_in_rises_from_silence_on_the_first_pass_only() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let options = PlayOptions {
        looping: true,
        fade_in: Duration::from_millis(10),
        ..PlayOptions::default()
    };
    engine.play(constant(1.0, 960), options);
    let out = left(&render(&mut mixer, 960 * 2, 2));
    assert_eq!(out[0], 0.0);
    assert_close(out[240], 0.5, 1e-3);
    assert!(out[480..960].iter().all(|&sample| sample == 1.0));
    assert!(
        out[960..].iter().all(|&sample| sample == 1.0),
        "no fade on the second pass"
    );
}

#[test]
fn a_fade_out_sinks_to_silence_before_the_end() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let options = PlayOptions {
        fade_out: Duration::from_millis(50),
        ..PlayOptions::default()
    };
    let voice = engine.play(constant(1.0, 4800), options).unwrap();
    let out = left(&render(&mut mixer, 5000, 2));
    assert_eq!(out[2399], 1.0, "before the fade");
    assert_close(out[4800 - 1200], 0.5, 1e-3);
    assert!(out[4799] < 0.001);
    assert_eq!(engine.poll(), [voice]);
}

#[test]
fn a_region_plays_and_loops_only_its_frames() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let samples = [0.1, 0.2, 0.3, 0.4, 0.5];
    let once = PlayOptions {
        region: Some(1..4),
        ..PlayOptions::default()
    };
    engine.play(mono(RATE, &samples), once);
    assert_eq!(left(&render(&mut mixer, 5, 2)), [0.2, 0.3, 0.4, 0.0, 0.0]);

    let looped = PlayOptions {
        region: Some(1..4),
        looping: true,
        ..PlayOptions::default()
    };
    let voice = engine.play(mono(RATE, &samples), looped).unwrap();
    assert_eq!(
        left(&render(&mut mixer, 7, 2)),
        [0.2, 0.3, 0.4, 0.2, 0.3, 0.4, 0.2]
    );
    assert_eq!(engine.position(voice), Some(2));
}

#[test]
fn reverb_rings_on_after_a_sound_ends_and_dies_away() {
    let click = mono(RATE, &[0.8; 64]);
    let tail = |reverb: f32| {
        let (mut engine, mut mixer) = Engine::offline(RATE);
        let options = PlayOptions {
            reverb,
            ..PlayOptions::default()
        };
        engine.play(Arc::clone(&click), options);
        left(&render(&mut mixer, RATE as usize * 3, 2))
    };
    let energy = |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>();

    let dry = tail(0.0);
    assert!(
        dry[64..].iter().all(|&sample| sample == 0.0),
        "no reverb, no tail"
    );

    let wet = tail(1.0);
    let early = energy(&wet[2_000..12_000]);
    let late = energy(&wet[100_000..110_000]);
    assert!(early > 1e-4, "the tail is audible: {early}");
    assert!(late < early / 10.0, "and decays: {early} -> {late}");
    assert!(
        wet.iter()
            .all(|sample| sample.is_finite() && sample.abs() <= 1.0)
    );
}

#[test]
fn the_reverb_send_can_change_while_a_voice_plays() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let voice = engine
        .play(constant(0.5, RATE as usize), PlayOptions::default())
        .unwrap();
    render(&mut mixer, 1000, 2);
    engine.set_reverb(voice, 1.0);
    engine.stop(voice, Duration::ZERO);
    render(&mut mixer, 1000, 2);
    let out = left(&render(&mut mixer, 4000, 2));
    assert!(
        out.iter().any(|&sample| sample != 0.0),
        "the reverb tail sounds"
    );
}

#[test]
fn output_peaks_report_the_loudest_sample_since_the_last_read() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    engine.play(mono(RATE, &[0.1, -0.6, 0.3]), PlayOptions::default());
    render(&mut mixer, 64, 2);
    assert_eq!(engine.take_output_peaks(), [0.6, 0.6]);
    // Taken: nothing louder since.
    render(&mut mixer, 64, 2);
    assert_eq!(engine.take_output_peaks(), [0.0, 0.0]);
}

#[test]
fn output_peaks_follow_the_master_gain_and_show_clipping() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    engine.play(constant(0.8, 4800), PlayOptions::default());
    engine.play(constant(0.8, 4800), PlayOptions::default());
    render(&mut mixer, 512, 2);
    // The mix is 1.6: the output clipped, and the meter says so.
    assert_close(engine.take_output_peaks()[0], 1.6, 1e-6);

    engine.set_master_gain(0.25);
    render(&mut mixer, 1024, 2);
    engine.take_output_peaks();
    render(&mut mixer, 512, 2);
    assert_close(engine.take_output_peaks()[1], 0.4, 1e-6);
}

#[test]
fn peaks_survive_several_buffers_between_reads() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let mut samples = vec![0.1; 1000];
    samples[100] = 0.9;
    engine.play(mono(RATE, &samples), PlayOptions::default());
    for _ in 0..8 {
        render(&mut mixer, 128, 2);
    }
    assert_eq!(engine.take_output_peaks(), [0.9, 0.9]);
}

#[test]
fn each_voice_reports_its_own_level_until_it_ends() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let quiet = engine
        .play(constant(0.2, 4800), PlayOptions::default())
        .unwrap();
    let loud = engine
        .play(
            constant(0.5, 100),
            PlayOptions {
                gain: 0.5,
                ..PlayOptions::default()
            },
        )
        .unwrap();
    render(&mut mixer, 64, 2);
    assert_close(engine.take_voice_peak(quiet).unwrap(), 0.2, 1e-6);
    assert_close(engine.take_voice_peak(loud).unwrap(), 0.25, 1e-6);

    // The loud one plays out; its slot reports nothing for it any more.
    render(&mut mixer, 256, 2);
    engine.poll();
    assert_eq!(engine.take_voice_peak(loud), None);
    assert!(engine.take_voice_peak(quiet).is_some());
}

#[test]
fn a_new_voice_does_not_inherit_the_level_of_its_slot() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    engine.play(constant(0.9, 10), PlayOptions::default());
    render(&mut mixer, 64, 2);
    render(&mut mixer, 64, 2);
    engine.poll();
    let quiet = engine
        .play(constant(0.1, 4800), PlayOptions::default())
        .unwrap();
    render(&mut mixer, 64, 2);
    assert_close(engine.take_voice_peak(quiet).unwrap(), 0.1, 1e-6);
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()))
}

#[test]
fn the_ambience_is_silent_until_turned_on_and_stays_quiet() {
    let (mut engine, mut mixer) = Engine::offline(RATE);
    let silent = render(&mut mixer, 4800, 2);
    assert_eq!(peak(&silent), 0.0);

    let full = AmbienceSettings {
        enabled: true,
        level: 1.0,
        background: Background::Rain,
        ..AmbienceSettings::default()
    };
    engine.set_ambience(full);
    // Past the second it takes to fade in.
    render(&mut mixer, RATE as usize * 2, 2);
    let mut out = Vec::new();
    for _ in 0..20 {
        out.extend(render(&mut mixer, 4800, 2));
    }
    let level = rms(&out);
    assert!(level > 0.002, "it can be heard: {level}");
    assert!(level < 0.08, "but it stays under the sounds: {level}");
    assert!(
        peak(&out) < 0.5,
        "and never gets near full scale: {}",
        peak(&out)
    );

    engine.set_ambience(AmbienceSettings {
        enabled: false,
        ..full
    });
    render(&mut mixer, RATE as usize * 2, 2);
    let after = render(&mut mixer, 4800, 2);
    // Only the reverb's tail is left, well under hearing.
    assert!(
        peak(&after) < 1e-3,
        "off, it fades to nothing: {}",
        peak(&after)
    );
    render(&mut mixer, RATE as usize * 8, 2);
    let later = render(&mut mixer, 4800, 2);
    assert_eq!(peak(&later), 0.0);
}

#[test]
fn the_ambience_ducks_under_a_playing_sound() {
    let settings = AmbienceSettings {
        enabled: true,
        level: 0.8,
        background: Background::Wind,
        duck: true,
        ..AmbienceSettings::default()
    };
    let bed_level = |with_voice: bool| {
        let (mut engine, mut mixer) = Engine::offline(RATE);
        engine.set_ambience(settings);
        render(&mut mixer, RATE as usize * 2, 2);
        if with_voice {
            // A silent clip still counts as playing.
            engine.play(constant(0.0, RATE as usize * 4), PlayOptions::default());
            render(&mut mixer, RATE as usize, 2);
        }
        rms(&render(&mut mixer, RATE as usize, 2))
    };
    let (alone, ducked) = (bed_level(false), bed_level(true));
    assert!(ducked < alone * 0.6, "{ducked} vs {alone}");
    assert!(ducked > 0.0);
}

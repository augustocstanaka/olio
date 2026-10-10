//! Decoding every supported container and codec, using the fixtures in
//! `tests/fixtures`: a 440 Hz tone peaking at 1/16 of full scale (ffmpeg's
//! `sine` source at 1/8, halved).

use std::path::{Path, PathBuf};

use olio_audio::{Clip, DecodeError, WavFormat, decode_file};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn peak(clip: &Clip) -> f32 {
    clip.samples()
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
}

#[track_caller]
fn assert_tone(name: &str, channels: u16, seconds: f64, tolerance: f64) {
    let clip = decode_file(&fixture(name)).unwrap_or_else(|err| panic!("{name}: {err}"));
    assert_eq!(clip.rate(), 22_050, "{name}");
    assert_eq!(clip.channels(), channels, "{name}");
    let duration = clip.duration().as_secs_f64();
    assert!(
        (duration - seconds).abs() <= tolerance,
        "{name}: {duration} s, expected {seconds} s"
    );
    // Only a scale check: stereo fixtures are 3 dB down from ffmpeg's upmix,
    // and lossy codecs overshoot at the onset. Unnormalised integer samples
    // would be thousands of times larger.
    let peak = peak(&clip);
    assert!((0.03..=0.15).contains(&peak), "{name}: peak {peak}");
}

#[test]
fn lossless_formats_decode_exactly() {
    assert_tone("tone.flac", 1, 0.5, 0.0);
    assert_tone("tone-stereo.aiff", 2, 0.1, 0.0);
}

#[test]
fn lossy_formats_decode_to_about_the_right_length() {
    // Encoders add priming and padding frames, so the length is approximate.
    assert_tone("tone.mp3", 1, 0.5, 0.1);
    assert_tone("tone.m4a", 1, 0.5, 0.1);
    assert_tone("tone-stereo.ogg", 2, 0.5, 0.1);
}

#[test]
fn a_float_wav_round_trips_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("round-trip.wav");
    let samples: Vec<f32> = (0..2000).map(|i| ((i as f32) * 0.01).sin() * 0.9).collect();
    let clip = Clip::new(44_100, 2, samples);
    clip.write_wav(&path, WavFormat::Float32).unwrap();
    assert_eq!(decode_file(&path).unwrap(), clip);
}

#[test]
fn a_16_bit_wav_round_trips_to_within_its_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pcm16.wav");
    let clip = Clip::new(8_000, 1, vec![0.0, 0.5, -0.5, 1.0, -1.0, 0.123]);
    clip.write_wav(&path, WavFormat::Int16).unwrap();
    let decoded = decode_file(&path).unwrap();
    assert_eq!((decoded.rate(), decoded.channels()), (8_000, 1));
    for (a, b) in decoded.samples().iter().zip(clip.samples()) {
        assert!((a - b).abs() < 1.0 / 16_000.0, "{a} vs {b}");
    }
}

#[test]
fn the_audio_track_of_a_video_is_decoded() {
    assert_tone("tone-video.mp4", 1, 0.5, 0.1);
    assert!(olio_audio::is_video(&fixture("tone-video.mp4")));
    assert!(!olio_audio::is_video(&fixture("tone.m4a")));
}

#[test]
fn files_that_are_not_audio_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.wav");
    std::fs::write(&empty, b"").unwrap();

    assert!(matches!(
        decode_file(&fixture("not-audio.mp3")),
        Err(DecodeError::Format(_) | DecodeError::Empty)
    ));
    assert!(decode_file(&empty).is_err());
    assert!(matches!(
        decode_file(&dir.path().join("missing.wav")),
        Err(DecodeError::Io(_))
    ));
}

#[test]
fn supported_files_are_recognised_by_extension() {
    assert!(olio_audio::is_supported(Path::new("a/b/Anthem.MP3")));
    assert!(olio_audio::is_supported(Path::new("loop.aiff")));
    assert!(olio_audio::is_supported(Path::new("clip.MOV")));
    assert!(!olio_audio::is_supported(Path::new("notes.txt")));
    assert!(!olio_audio::is_supported(Path::new("no-extension")));
}

//! Decoded sounds held in memory.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::ops::Range;
use std::path::Path;
use std::time::Duration;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// A sound decoded to interleaved `f32` samples at its own sample rate.
///
/// Clips are shared with the audio thread as `Arc<Clip>` and are immutable
/// once built.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    rate: u32,
    channels: u16,
    samples: Vec<f32>,
}

impl Clip {
    /// Builds a clip from interleaved samples.
    ///
    /// # Panics
    ///
    /// If `rate` or `channels` is zero, or `samples` does not hold a whole
    /// number of frames.
    pub fn new(rate: u32, channels: u16, samples: Vec<f32>) -> Self {
        assert!(rate > 0, "a clip needs a sample rate");
        assert!(channels > 0, "a clip needs at least one channel");
        assert!(
            samples.len().is_multiple_of(usize::from(channels)),
            "{} samples is not a whole number of {channels}-channel frames",
            samples.len()
        );
        Self {
            rate,
            channels,
            samples,
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Interleaved samples, `channels` per frame.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Number of frames (samples per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels)
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.frames() as f64 / f64::from(self.rate))
    }

    /// The frame at `index` as a stereo pair: mono is duplicated, and
    /// channels beyond the second are ignored.
    #[inline]
    pub fn stereo_frame(&self, index: usize) -> (f32, f32) {
        let channels = usize::from(self.channels);
        let base = index * channels;
        let left = self.samples[base];
        let right = if channels > 1 {
            self.samples[base + 1]
        } else {
            left
        };
        (left, right)
    }

    /// The frames worth playing: everything from shortly before the first
    /// sample louder than `threshold` to shortly after the last, so silence
    /// at either end is skipped. A clip with nothing above the threshold is
    /// returned whole.
    pub fn audible_range(&self, threshold: f32, padding: Duration) -> Range<usize> {
        let channels = usize::from(self.channels);
        let loud = |frame: &[f32]| frame.iter().any(|sample| sample.abs() > threshold);
        let mut frames = self.samples.chunks_exact(channels);
        let Some(first) = frames.position(loud) else {
            return 0..self.frames();
        };
        let last = self
            .samples
            .chunks_exact(channels)
            .rposition(loud)
            .unwrap_or(first);
        let pad = (padding.as_secs_f64() * f64::from(self.rate)) as usize;
        first.saturating_sub(pad)..(last + 1 + pad).min(self.frames())
    }

    /// Writes the clip as a WAV file.
    pub fn write_wav(&self, path: &Path, format: WavFormat) -> io::Result<()> {
        let (tag, bytes): (u16, u16) = match format {
            WavFormat::Float32 => (3, 4), // WAVE_FORMAT_IEEE_FLOAT
            WavFormat::Int16 => (1, 2),   // WAVE_FORMAT_PCM
        };
        let mut out = BufWriter::new(File::create(path)?);
        let data_len = u32::try_from(self.samples.len() * usize::from(bytes))
            .ok()
            .filter(|len| *len <= u32::MAX - 36)
            .ok_or_else(|| io::Error::other("clip too long for a WAV file"))?;
        let block_align = self.channels * bytes;
        out.write_all(b"RIFF")?;
        out.write_all(&(36 + data_len).to_le_bytes())?;
        out.write_all(b"WAVEfmt ")?;
        out.write_all(&16u32.to_le_bytes())?;
        out.write_all(&tag.to_le_bytes())?;
        out.write_all(&self.channels.to_le_bytes())?;
        out.write_all(&self.rate.to_le_bytes())?;
        out.write_all(&(self.rate * u32::from(block_align)).to_le_bytes())?;
        out.write_all(&block_align.to_le_bytes())?;
        out.write_all(&(bytes * 8).to_le_bytes())?;
        out.write_all(b"data")?;
        out.write_all(&data_len.to_le_bytes())?;
        for &sample in &self.samples {
            match format {
                WavFormat::Float32 => out.write_all(&sample.to_le_bytes())?,
                WavFormat::Int16 => {
                    let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
                    out.write_all(&value.to_le_bytes())?;
                }
            }
        }
        out.flush()
    }
}

/// File extensions (lowercase) of the audio formats [`decode_file`] reads.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "wave", "aif", "aiff", "aifc", "caf", "flac", "mp3", "mp2", "mp1", "m4a", "aac", "ogg",
    "oga", "mka",
];

/// File extensions (lowercase) of video containers whose audio track
/// [`decode_file`] reads (when it is AAC, ALAC, MP3, FLAC, Vorbis or PCM).
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "mkv"];

fn extension_in(path: &Path, list: &[&str]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| list.contains(&extension.to_ascii_lowercase().as_str()))
}

/// True if `path` has the extension of an audio or video format
/// [`decode_file`] reads.
pub fn is_supported(path: &Path) -> bool {
    extension_in(path, AUDIO_EXTENSIONS) || extension_in(path, VIDEO_EXTENSIONS)
}

/// True for video containers: their audio is extracted on import rather
/// than the whole file being kept.
pub fn is_video(path: &Path) -> bool {
    extension_in(path, VIDEO_EXTENSIONS)
}

/// Sample formats for [`Clip::write_wav`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavFormat {
    /// 32-bit float: decodes back to the identical samples.
    Float32,
    /// 16-bit integer: half the size, for audio taken from lossy sources.
    Int16,
}

/// Why a file could not be decoded.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("could not read the file: {0}")]
    Io(#[from] io::Error),
    #[error("unsupported or damaged audio: {0}")]
    Format(#[from] SymphoniaError),
    #[error("the file has no audio track")]
    NoAudioTrack,
    #[error("the file contains no audio")]
    Empty,
    #[error("the audio changes sample rate or channel count part way through")]
    FormatChanges,
}

/// Decodes a whole sound file into memory.
///
/// Any format and codec Symphonia supports works (WAV, AIFF, CAF, FLAC, MP3,
/// AAC and ALAC in M4A, Ogg Vorbis). Damaged packets are skipped, as players
/// usually do. Runs on a background thread, never on the audio thread.
pub fn decode_file(path: &Path) -> Result<Clip, DecodeError> {
    let file = File::open(path)?;
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(extension);
    }
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut format = symphonia::default::get_probe().probe(
        &hint,
        source,
        FormatOptions::default(),
        MetadataOptions::default(),
    )?;

    let (track_id, mut decoder) = {
        let track = format
            .default_track(TrackType::Audio)
            .ok_or(DecodeError::NoAudioTrack)?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|params| params.audio())
            .ok_or(DecodeError::NoAudioTrack)?;
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())?;
        (track.id, decoder)
    };

    let mut spec: Option<(u32, u16)> = None;
    let mut samples = Vec::new();
    let mut packet_samples: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            // A chained stream (a new Ogg stream, say): keep the first one.
            Err(SymphoniaError::ResetRequired) => break,
            Err(SymphoniaError::IoError(err)) if err.kind() == io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(err) => return Err(err.into()),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_) | SymphoniaError::IoError(_)) => continue,
            Err(err) => return Err(err.into()),
        };
        let packet_spec = (
            decoded.spec().rate(),
            u16::try_from(decoded.spec().channels().count()).unwrap_or(u16::MAX),
        );
        match spec {
            None => spec = Some(packet_spec),
            Some(first) if first != packet_spec => return Err(DecodeError::FormatChanges),
            Some(_) => {}
        }
        decoded.copy_to_vec_interleaved(&mut packet_samples);
        samples.extend_from_slice(&packet_samples);
    }

    let (rate, channels) = spec.ok_or(DecodeError::Empty)?;
    if rate == 0 || channels == 0 || samples.is_empty() {
        return Err(DecodeError::Empty);
    }
    Ok(Clip::new(rate, channels, samples))
}

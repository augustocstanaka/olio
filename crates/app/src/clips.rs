//! Decoded clips, loaded on a background thread and cached by sound.

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use olio_audio::{Clip, Peaks, Spectrogram, decode_file};

/// Slices of waveform kept per clip: enough for a tile several hundred
/// pixels wide.
const PEAK_SLICES: usize = 1024;

/// Quieter than this counts as silence when trimming: -60 dBFS.
const SILENCE_THRESHOLD: f32 = 0.001;

/// Kept either side of the audible part, so soft attacks and tails survive.
const TRIM_PADDING: Duration = Duration::from_millis(10);

/// The spectrogram's size: columns of time by bands of frequency.
const SPECTROGRAM_COLUMNS: usize = 256;
const SPECTROGRAM_BANDS: usize = 24;

/// The loudness Normalize brings every sound to: -18 dBFS RMS.
const TARGET_RMS: f32 = 0.126;
/// The loudest a normalized sound may peak: -1 dBFS, so raising a quiet
/// sound never clips it.
const PEAK_CEILING: f32 = 0.891;
/// Quieter than this (-80 dBFS RMS) is silence, left as it is.
const SILENT_RMS: f32 = 0.0001;

pub enum ClipState {
    Loading,
    Ready(Ready),
    Failed(String),
}

pub struct Ready {
    pub clip: Arc<Clip>,
    pub peaks: Peaks,
    /// The largest absolute sample, for drawing quiet sounds at full height.
    pub loudest: f32,
    /// The frames between the silence at either end.
    pub audible: Range<usize>,
    pub spectrogram: Spectrogram,
    /// RMS level of the audible part and of the whole clip, linear.
    rms_audible: f32,
    rms_whole: f32,
    /// The spectrogram as a texture, made the first time it is drawn.
    spectrogram_texture: OnceLock<TextureHandle>,
}

impl Ready {
    /// The frames a tile plays: the audible part, or all of it.
    pub fn region(&self, trim_silence: bool) -> Range<usize> {
        if trim_silence {
            self.audible.clone()
        } else {
            0..self.clip.frames()
        }
    }

    /// The gain Normalize gives: the target loudness over the sound's own,
    /// lowered if it would push the sound's peak past the ceiling. 1 when
    /// not normalizing or for silence.
    pub fn gain(&self, normalize: bool, trim_silence: bool) -> f32 {
        let rms = if trim_silence {
            self.rms_audible
        } else {
            self.rms_whole
        };
        if !normalize || rms < SILENT_RMS || self.loudest <= 0.0 {
            return 1.0;
        }
        (TARGET_RMS / rms).min(PEAK_CEILING / self.loudest)
    }

    /// The spectrogram as a texture: white, with each cell's level as its
    /// opacity, so it can be painted in any colour. Low bands at the bottom.
    pub fn spectrogram_texture(&self, ctx: &egui::Context) -> &TextureHandle {
        self.spectrogram_texture.get_or_init(|| {
            let spectrogram = &self.spectrogram;
            let (width, height) = (spectrogram.columns(), spectrogram.bands());
            let mut pixels = vec![Color32::TRANSPARENT; width * height];
            for column in 0..width {
                for band in 0..height {
                    let level = spectrogram.cell(column, band);
                    pixels[(height - 1 - band) * width + column] =
                        Color32::from_rgba_unmultiplied(255, 255, 255, level);
                }
            }
            let image = ColorImage::new([width, height], pixels);
            ctx.load_texture("spectrogram", image, TextureOptions::LINEAR)
        })
    }
}

impl Ready {
    fn new(clip: Clip) -> Self {
        let peaks = Peaks::of(&clip, PEAK_SLICES);
        let loudest = peaks
            .slices()
            .iter()
            .fold(0.0f32, |loudest, &(low, high)| loudest.max(-low).max(high));
        let audible = clip.audible_range(SILENCE_THRESHOLD, TRIM_PADDING);
        Self {
            spectrogram: Spectrogram::of(&clip, SPECTROGRAM_COLUMNS, SPECTROGRAM_BANDS),
            rms_audible: clip.rms(audible.clone()),
            rms_whole: clip.rms(0..clip.frames()),
            spectrogram_texture: OnceLock::new(),
            audible,
            clip: Arc::new(clip),
            peaks,
            loudest,
        }
    }
}

type Job = (String, PathBuf);
type Done = (String, Result<Ready, String>);

/// Clips by sound hash. Requests are decoded one at a time on a worker
/// thread, which wakes the UI when each finishes.
pub struct Clips {
    states: HashMap<String, ClipState>,
    jobs: mpsc::Sender<Job>,
    done: mpsc::Receiver<Done>,
}

impl Clips {
    pub fn new(ctx: egui::Context) -> Self {
        let (jobs, job_queue) = mpsc::channel::<Job>();
        let (finished, done) = mpsc::channel::<Done>();
        thread::Builder::new()
            .name("olio-decoder".to_owned())
            .spawn(move || {
                for (hash, path) in job_queue {
                    let result = decode_file(&path)
                        .map(Ready::new)
                        .map_err(|err| err.to_string());
                    if finished.send((hash, result)).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("the decoder thread starts");
        Self {
            states: HashMap::new(),
            jobs,
            done,
        }
    }

    /// Starts decoding a sound unless it is loaded or loading already.
    pub fn request(&mut self, hash: &str, path: impl FnOnce() -> PathBuf) {
        if self.states.contains_key(hash) {
            return;
        }
        let state = match self.jobs.send((hash.to_owned(), path())) {
            Ok(()) => ClipState::Loading,
            Err(_) => ClipState::Failed("the decoder has stopped".to_owned()),
        };
        self.states.insert(hash.to_owned(), state);
    }

    pub fn get(&self, hash: &str) -> Option<&ClipState> {
        self.states.get(hash)
    }

    pub fn is_loading(&self, hash: &str) -> bool {
        matches!(self.states.get(hash), None | Some(ClipState::Loading))
    }

    /// Takes in every clip decoded since the last call.
    pub fn poll(&mut self) {
        while let Ok((hash, result)) = self.done.try_recv() {
            let state = match result {
                Ok(ready) => ClipState::Ready(ready),
                Err(message) => ClipState::Failed(message),
            };
            self.states.insert(hash, state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(level: f32, frames: usize) -> Ready {
        let samples = (0..frames)
            .map(|n| level * (n as f32 * 0.05).sin())
            .collect();
        Ready::new(Clip::new(48_000, 1, samples))
    }

    #[test]
    fn normalize_raises_quiet_sounds_and_lowers_loud_ones() {
        let quiet = tone(0.05, 48_000);
        let loud = tone(0.85, 48_000);
        // A sine's RMS is its peak over the square root of two.
        let quiet_gain = quiet.gain(true, true);
        let loud_gain = loud.gain(true, true);
        assert!(quiet_gain > 1.0 && loud_gain < 1.0);
        let quiet_rms = 0.05 / 2f32.sqrt() * quiet_gain;
        let loud_rms = 0.85 / 2f32.sqrt() * loud_gain;
        assert!(
            (quiet_rms - loud_rms).abs() < 0.005,
            "{quiet_rms} vs {loud_rms}"
        );
        assert_eq!(quiet.gain(false, true), 1.0);
    }

    #[test]
    fn normalize_never_pushes_a_peak_past_the_ceiling() {
        // Mostly silence with one spike: its RMS is tiny, its peak is not.
        let mut samples = vec![0.001f32; 48_000];
        samples[100] = 0.8;
        let ready = Ready::new(Clip::new(48_000, 1, samples));
        let gain = ready.gain(true, false);
        assert!(0.8 * gain <= PEAK_CEILING + 1e-6);
    }

    #[test]
    fn silence_is_left_alone() {
        let ready = Ready::new(Clip::new(48_000, 1, vec![0.0; 4800]));
        assert_eq!(ready.gain(true, true), 1.0);
    }
}

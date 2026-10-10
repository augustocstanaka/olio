//! Decoded clips, loaded on a background thread and cached by sound.

use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use olio_audio::{Clip, Peaks, decode_file};

/// Slices of waveform kept per clip: enough for a tile several hundred
/// pixels wide.
const PEAK_SLICES: usize = 1024;

/// Quieter than this counts as silence when trimming: -60 dBFS.
const SILENCE_THRESHOLD: f32 = 0.001;

/// Kept either side of the audible part, so soft attacks and tails survive.
const TRIM_PADDING: Duration = Duration::from_millis(10);

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
}

impl Ready {
    fn new(clip: Clip) -> Self {
        let peaks = Peaks::of(&clip, PEAK_SLICES);
        let loudest = peaks
            .slices()
            .iter()
            .fold(0.0f32, |loudest, &(low, high)| loudest.max(-low).max(high));
        Self {
            audible: clip.audible_range(SILENCE_THRESHOLD, TRIM_PADDING),
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

//! The app's handle on the mixer, and the output stream that drives it.

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use crate::ambience::AmbienceSettings;
use crate::clip::Clip;
use crate::mixer::{Event, MAX_VOICES, Message, Mixer, Shared, VoiceId, VoiceSettings};

/// Room in each queue between the app and the audio thread. The mixer
/// drains its queue every buffer (a few milliseconds), so this is far more
/// than a burst of key presses needs.
const QUEUE_CAPACITY: usize = 1024;

/// Frames rendered per pass when the device's sample type is not `f32`.
const SCRATCH_FRAMES: usize = 4096;

/// How a voice plays.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayOptions {
    /// Linear gain, 1.0 for the clip as recorded.
    pub gain: f32,
    /// Play the region again from its start until stopped.
    pub looping: bool,
    /// The frames of the clip to play (and loop over); `None` for all of it.
    pub region: Option<Range<usize>>,
    /// Rise from silence over this long at the start (first pass only).
    pub fade_in: Duration,
    /// Sink to silence over this long before the region ends (not when
    /// looping). Stopping a voice early takes its own fade; see
    /// [`Engine::stop`].
    pub fade_out: Duration,
    /// Share of the voice sent to the reverb, 0 (dry) to 1.
    pub reverb: f32,
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            gain: 1.0,
            looping: false,
            region: None,
            fade_in: Duration::ZERO,
            fade_out: Duration::ZERO,
            reverb: 0.0,
        }
    }
}

/// The output device the engine is playing through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputInfo {
    pub device: String,
    pub rate: u32,
    pub channels: u16,
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no audio output device is available")]
    NoDevice,
    #[error("the output device reported no usable format: {0}")]
    Config(cpal::Error),
    #[error("could not open the output device: {0}")]
    Stream(cpal::Error),
    #[error("the output device uses an unsupported sample format ({0})")]
    SampleFormat(SampleFormat),
}

struct Output {
    // Dropping the stream stops the device; it is never touched otherwise.
    _stream: cpal::Stream,
    info: OutputInfo,
    failed: Arc<AtomicBool>,
}

/// Starts and stops voices on the mixer, and reports when they end.
///
/// Lives on the app's thread. Every call returns immediately: requests are
/// queued for the audio thread, which picks them up at its next buffer.
pub struct Engine {
    messages: rtrb::Producer<Message>,
    events: rtrb::Consumer<Event>,
    shared: Arc<Shared>,
    next_voice: u64,
    /// Voices started and not yet reported ended.
    live: usize,
    output: Option<Output>,
}

impl Engine {
    fn with_mixer(rate: u32) -> (Self, Mixer) {
        let (message_tx, message_rx) = rtrb::RingBuffer::new(QUEUE_CAPACITY);
        let (event_tx, event_rx) = rtrb::RingBuffer::new(QUEUE_CAPACITY);
        let shared = Arc::new(Shared::new());
        let mixer = Mixer::new(rate, message_rx, event_tx, Arc::clone(&shared));
        let engine = Self {
            messages: message_tx,
            events: event_rx,
            shared,
            next_voice: 1,
            live: 0,
            output: None,
        };
        (engine, mixer)
    }

    /// An engine with no device: the caller renders the returned mixer
    /// itself. Used by tests, and anywhere audio must be produced headlessly.
    pub fn offline(rate: u32) -> (Self, Mixer) {
        Self::with_mixer(rate)
    }

    /// An engine playing through the system's default output device, at the
    /// device's preferred rate and format.
    pub fn open_default_output() -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(AudioError::NoDevice)?;
        let supported = device.default_output_config().map_err(AudioError::Config)?;
        let config = supported.config();
        let info = OutputInfo {
            device: device
                .description()
                .map(|description| description.name().to_owned())
                .unwrap_or_else(|_| device.to_string()),
            rate: config.sample_rate,
            channels: config.channels,
        };
        let (mut engine, mixer) = Self::with_mixer(config.sample_rate);
        let failed = Arc::new(AtomicBool::new(false));
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build_stream::<f32>(&device, config, mixer, &failed),
            SampleFormat::F64 => build_stream::<f64>(&device, config, mixer, &failed),
            SampleFormat::I16 => build_stream::<i16>(&device, config, mixer, &failed),
            SampleFormat::I24 => build_stream::<cpal::I24>(&device, config, mixer, &failed),
            SampleFormat::I32 => build_stream::<i32>(&device, config, mixer, &failed),
            SampleFormat::U16 => build_stream::<u16>(&device, config, mixer, &failed),
            SampleFormat::U8 => build_stream::<u8>(&device, config, mixer, &failed),
            SampleFormat::I8 => build_stream::<i8>(&device, config, mixer, &failed),
            other => return Err(AudioError::SampleFormat(other)),
        }
        .map_err(AudioError::Stream)?;
        stream.play().map_err(AudioError::Stream)?;
        engine.output = Some(Output {
            _stream: stream,
            info,
            failed,
        });
        Ok(engine)
    }

    /// The device being played through; `None` for an offline engine.
    pub fn output(&self) -> Option<&OutputInfo> {
        self.output.as_ref().map(|output| &output.info)
    }

    /// True once the output stream has reported an error it cannot recover
    /// from, such as its device being unplugged.
    pub fn output_failed(&self) -> bool {
        self.output
            .as_ref()
            .is_some_and(|output| output.failed.load(Ordering::Relaxed))
    }

    /// Starts a voice playing `clip` from the beginning. `None` when
    /// [`MAX_VOICES`] are already sounding.
    pub fn play(&mut self, clip: Arc<Clip>, options: PlayOptions) -> Option<VoiceId> {
        if self.live >= MAX_VOICES {
            return None;
        }
        let voice = VoiceId::new(self.next_voice);
        let settings = VoiceSettings {
            gain: options.gain,
            looping: options.looping,
            region: options.region.unwrap_or(0..clip.frames()),
            fade_in_seconds: options.fade_in.as_secs_f32(),
            fade_out_seconds: options.fade_out.as_secs_f32(),
            reverb: options.reverb,
        };
        let message = Message::Play {
            voice,
            clip,
            settings,
        };
        if self.send(message) {
            self.next_voice += 1;
            self.live += 1;
            Some(voice)
        } else {
            None
        }
    }

    /// Fades a voice out over `fade` (at least a few milliseconds, so it
    /// never clicks) and ends it. Unknown or ended voices are ignored.
    pub fn stop(&mut self, voice: VoiceId, fade: Duration) {
        self.send(Message::Stop {
            voice,
            fade_seconds: fade.as_secs_f32(),
        });
    }

    /// Fades every voice out over `fade` and ends them.
    pub fn stop_all(&mut self, fade: Duration) {
        self.send(Message::StopAll {
            fade_seconds: fade.as_secs_f32(),
        });
    }

    /// Turns looping on or off for a voice that is already playing.
    pub fn set_looping(&mut self, voice: VoiceId, looping: bool) {
        self.send(Message::SetLooping { voice, looping });
    }

    /// Changes how much of a playing voice goes to the reverb (0 to 1).
    pub fn set_reverb(&mut self, voice: VoiceId, reverb: f32) {
        self.send(Message::SetReverb { voice, reverb });
    }

    /// Turns the ambience on or off, or changes how it sounds.
    pub fn set_ambience(&mut self, settings: AmbienceSettings) {
        self.send(Message::Ambience(settings));
    }

    /// Plays a Generator instrument from recordings, or with `None` from
    /// its synthesised voice again. The old recordings come back through
    /// [`poll`](Self::poll) to be freed here, once their notes have faded.
    pub fn set_instrument(
        &mut self,
        instrument: crate::ambience::Instrument,
        samples: Option<Arc<crate::instruments::Multisample>>,
    ) {
        self.send(Message::Instrument {
            instrument,
            samples,
        });
    }

    /// Sets the gain applied to the whole mix (linear, 1.0 is unity).
    pub fn set_master_gain(&mut self, gain: f32) {
        self.send(Message::MasterGain { gain });
    }

    /// How far into its clip a voice is, in clip frames, as of the last
    /// buffer the mixer rendered. `None` once the voice has ended.
    pub fn position(&self, voice: VoiceId) -> Option<u64> {
        self.shared.position(voice)
    }

    /// What a building or evolving ambience plays now (its key,
    /// background, an instrument resting, whether it is still building),
    /// as of the last buffer; `None` when it is off or steady.
    pub fn ambience_now(&self) -> Option<crate::ambience::Now> {
        self.shared.ambience_now()
    }

    /// The loudest output sample on the first two channels since the last
    /// call, linear: 1.0 is full scale, and above it the output clipped.
    /// Measured after the master gain. For level meters.
    pub fn take_output_peaks(&self) -> [f32; 2] {
        self.shared.take_output_peaks()
    }

    /// A voice's loudest sample since the last call, after its gain and
    /// fades and before the master gain; `None` once it has ended.
    pub fn take_voice_peak(&self, voice: VoiceId) -> Option<f32> {
        self.shared.take_voice_peak(voice)
    }

    /// Voices started and not yet reported ended by [`poll`](Self::poll).
    pub fn live_voices(&self) -> usize {
        self.live
    }

    /// Collects the voices that have ended since the last call. Their clips
    /// are released here, on the caller's thread.
    pub fn poll(&mut self) -> Vec<VoiceId> {
        let mut ended = Vec::new();
        while let Ok(event) = self.events.pop() {
            match event {
                Event::Ended { voice, clip } => {
                    drop(clip);
                    self.live -= 1;
                    ended.push(voice);
                }
                Event::Released(samples) => drop(samples),
            }
        }
        ended
    }

    fn send(&mut self, message: Message) -> bool {
        // On a full queue the message comes back inside the error and is
        // dropped here, on the app's thread.
        self.messages.push(message).is_ok()
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut mixer: Mixer,
    failed: &Arc<AtomicBool>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    let mut scratch = vec![0.0f32; SCRATCH_FRAMES * channels];
    let failed = Arc::clone(failed);
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _info| {
            for chunk in data.chunks_mut(scratch.len()) {
                let mixed = &mut scratch[..chunk.len()];
                mixer.render(mixed, channels);
                for (out, &sample) in chunk.iter_mut().zip(mixed.iter()) {
                    *out = T::from_sample(sample);
                }
            }
        },
        move |err| {
            use cpal::ErrorKind;
            // Underruns and default-device switches are reported but the
            // stream keeps running.
            if !matches!(
                err.kind(),
                ErrorKind::Xrun | ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied
            ) {
                failed.store(true, Ordering::Relaxed);
            }
        },
        None,
    )
}

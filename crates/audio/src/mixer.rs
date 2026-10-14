//! The mixer: everything that runs on the audio thread.
//!
//! The rules for this file are in `CODING_STANDARDS.md`: nothing here
//! allocates, locks, does I/O or frees memory. Messages arrive through a
//! wait-free queue from the [`Engine`](crate::Engine); every clip leaves
//! through another queue inside an `Ended` event, so the last reference to
//! it is dropped on the app's thread, never here.

use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::ambience::{Ambience, AmbienceSettings, Instrument, Now};
use crate::clip::Clip;
use crate::instruments::Multisample;
use crate::reverb::Reverb;

/// The most voices that can sound at once. The engine refuses further
/// plays, so the mixer always has a free slot for a voice it is sent.
pub const MAX_VOICES: usize = 64;

/// Every level change is spread over at least this long, so stopping a
/// voice mid-waveform does not click.
const DECLICK_SECONDS: f32 = 0.005;

/// The mixer works through long buffers this many frames at a time, the
/// size of its preallocated reverb bus.
const CHUNK_FRAMES: usize = 4096;

/// How long the reverb keeps running after its last input: longer than its
/// audible tail.
const REVERB_TAIL_SECONDS: f32 = 6.0;

/// One playing instance of a clip, named by the engine when it starts it.
/// Ids are never reused during a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VoiceId(u64);

impl VoiceId {
    pub(crate) fn new(raw: u64) -> Self {
        debug_assert!(raw != 0, "0 marks an empty voice slot");
        Self(raw)
    }

    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

/// How a voice plays, fixed when it starts unless a message changes it.
#[derive(Clone, Debug)]
pub(crate) struct VoiceSettings {
    pub gain: f32,
    pub looping: bool,
    /// Frames of the clip to play, and to loop over.
    pub region: Range<usize>,
    pub fade_in_seconds: f32,
    /// Applied as the region's end approaches (unless looping).
    pub fade_out_seconds: f32,
    /// Share sent to the reverb, 0 to 1.
    pub reverb: f32,
}

/// What the engine asks of the mixer.
pub(crate) enum Message {
    Play {
        voice: VoiceId,
        clip: Arc<Clip>,
        settings: VoiceSettings,
    },
    Stop {
        voice: VoiceId,
        fade_seconds: f32,
    },
    SetLooping {
        voice: VoiceId,
        looping: bool,
    },
    SetReverb {
        voice: VoiceId,
        reverb: f32,
    },
    StopAll {
        fade_seconds: f32,
    },
    MasterGain {
        gain: f32,
    },
    Ambience(AmbienceSettings),
    /// An instrument's recordings, or none to synthesise it again.
    Instrument {
        instrument: Instrument,
        samples: Option<Arc<Multisample>>,
    },
}

/// What the mixer tells the engine.
pub(crate) enum Event {
    /// The voice has finished (played out or stopped); its clip travels back
    /// so it is dropped off the audio thread.
    Ended { voice: VoiceId, clip: Arc<Clip> },
    /// Recordings the ambience no longer plays, to be freed off the audio
    /// thread.
    Released(Arc<Multisample>),
}

/// State the mixer publishes after every buffer for the app to read:
/// which voice occupies each slot, how far into its clip it is, and how
/// loud it and the output have been.
pub(crate) struct Shared {
    slots: [PublishedVoice; MAX_VOICES],
    /// The loudest sample of the first two output channels since the app
    /// last read them.
    output_peaks: [Peak; 2],
    /// What a building or evolving ambience plays now, packed; 0 when
    /// there is nothing to show.
    ambience: AtomicU64,
}

#[derive(Default)]
struct PublishedVoice {
    voice: AtomicU64,
    position: AtomicU64,
    peak: Peak,
}

/// The loudest absolute sample level since the reader last took it. The
/// mixer only raises it and the reader swaps it back to zero, so neither
/// side ever waits and no peak between two reads is lost.
///
/// Stored as `f32` bits: for values of zero and above, the bit patterns
/// sort in the same order as the numbers, so `fetch_max` on the bits is a
/// maximum of the levels.
#[derive(Default)]
struct Peak(AtomicU32);

impl Peak {
    fn raise(&self, level: f32) {
        if level > 0.0 {
            self.0.fetch_max(level.to_bits(), Ordering::Relaxed);
        }
    }

    fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }

    fn clear(&self) {
        self.0.store(0, Ordering::Relaxed);
    }
}

impl Shared {
    pub(crate) fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| PublishedVoice::default()),
            output_peaks: Default::default(),
            ambience: AtomicU64::new(0),
        }
    }

    /// What a building or evolving ambience plays now, as of the last
    /// buffer.
    pub(crate) fn ambience_now(&self) -> Option<Now> {
        Now::unpack(self.ambience.load(Ordering::Relaxed))
    }

    /// The loudest output sample on the first two channels since the last
    /// call (a mono output reports the same level twice), after the master
    /// gain and before the output is clipped: above 1.0 means it clipped.
    pub(crate) fn take_output_peaks(&self) -> [f32; 2] {
        [self.output_peaks[0].take(), self.output_peaks[1].take()]
    }

    /// The voice's loudest sample since the last call, after its gain and
    /// fades; `None` once it has ended.
    pub(crate) fn take_voice_peak(&self, voice: VoiceId) -> Option<f32> {
        self.slots
            .iter()
            .find(|slot| slot.voice.load(Ordering::Relaxed) == voice.get())
            .map(|slot| slot.peak.take())
    }

    /// The voice's position in frames of its clip, if it is still sounding.
    pub(crate) fn position(&self, voice: VoiceId) -> Option<u64> {
        self.slots
            .iter()
            .find(|slot| slot.voice.load(Ordering::Relaxed) == voice.get())
            .map(|slot| slot.position.load(Ordering::Relaxed))
    }
}

/// A gain that moves linearly to a target over a number of frames.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ramp {
    value: f32,
    target: f32,
    step: f32,
    remaining: u32,
}

impl Ramp {
    pub(crate) fn at(value: f32) -> Self {
        Self {
            value,
            target: value,
            step: 0.0,
            remaining: 0,
        }
    }

    pub(crate) fn towards(&mut self, target: f32, frames: u32) {
        self.target = target;
        self.remaining = frames;
        if frames == 0 {
            self.value = target;
            self.step = 0.0;
        } else {
            self.step = (target - self.value) / frames as f32;
        }
    }

    /// The gain for the next frame.
    pub(crate) fn target(&self) -> f32 {
        self.target
    }

    #[inline]
    pub(crate) fn next(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            self.value = if self.remaining == 0 {
                self.target
            } else {
                self.value + self.step
            };
        }
        self.value
    }

    pub(crate) fn is_silent(&self) -> bool {
        self.remaining == 0 && self.value == 0.0
    }
}

struct Voice {
    id: VoiceId,
    clip: Arc<Clip>,
    /// Position in frames of the clip; fractional when the clip's rate
    /// differs from the output's.
    position: f64,
    /// Clip frames per output frame.
    step: f64,
    /// The region played, in clip frames.
    start: f64,
    end: f64,
    looping: bool,
    /// False once a looping voice has wrapped round: the fade-in applies to
    /// the first pass only.
    first_pass: bool,
    /// Fade lengths in clip frames.
    fade_in: f64,
    fade_out: f64,
    gain: Ramp,
    send: Ramp,
    stopping: bool,
    finished: bool,
    /// The loudest sample mixed since the last publish.
    peak: f32,
}

impl Voice {
    fn new(id: VoiceId, clip: Arc<Clip>, settings: &VoiceSettings, output_rate: u32) -> Self {
        let frames = clip.frames();
        let end = settings.region.end.min(frames);
        let start = settings.region.start.min(end);
        let clip_rate = f64::from(clip.rate());
        Self {
            id,
            step: clip_rate / f64::from(output_rate),
            position: start as f64,
            start: start as f64,
            end: end as f64,
            looping: settings.looping,
            first_pass: true,
            fade_in: f64::from(settings.fade_in_seconds.max(0.0)) * clip_rate,
            fade_out: f64::from(settings.fade_out_seconds.max(0.0)) * clip_rate,
            gain: Ramp::at(settings.gain.max(0.0)),
            send: Ramp::at(settings.reverb.clamp(0.0, 1.0)),
            stopping: false,
            finished: false,
            peak: 0.0,
            clip,
        }
    }

    fn stop(&mut self, frames: u32) {
        self.stopping = true;
        self.gain.towards(0.0, frames);
    }

    /// The fade-in and fade-out envelope at the current position.
    #[inline]
    fn envelope(&self) -> f32 {
        let mut envelope = 1.0;
        if self.first_pass && self.fade_in > 0.0 {
            envelope *= ((self.position - self.start) / self.fade_in).min(1.0);
        }
        if !self.looping && self.fade_out > 0.0 {
            envelope *= ((self.end - self.position) / self.fade_out).clamp(0.0, 1.0);
        }
        envelope as f32
    }

    /// Adds this voice into `out` (dry) and into the stereo `bus` (its
    /// reverb send) until the buffer is full or the voice ends.
    fn mix(&mut self, out: &mut [f32], bus: &mut [f32], channels: usize) {
        let length = self.end - self.start;
        if length <= 0.0 {
            self.finished = true;
            return;
        }
        let (sends, _) = bus.as_chunks_mut::<2>();
        for (frame, send) in out.chunks_exact_mut(channels).zip(sends) {
            if self.position >= self.end {
                if self.looping {
                    self.position = self.start + (self.position - self.end) % length;
                    self.first_pass = false;
                } else {
                    self.finished = true;
                    return;
                }
            }
            let (left, right) = self.sample();
            let gain = self.gain.next() * self.envelope();
            let (left, right) = (left * gain, right * gain);
            self.peak = self.peak.max(left.abs()).max(right.abs());
            if channels == 1 {
                frame[0] += 0.5 * (left + right);
            } else {
                frame[0] += left;
                frame[1] += right;
            }
            let amount = self.send.next();
            send[0] += left * amount;
            send[1] += right * amount;
            self.position += self.step;
            if self.stopping && self.gain.is_silent() {
                self.finished = true;
                return;
            }
        }
    }

    /// The stereo sample at the current position: read directly when the
    /// rates match, Catmull-Rom interpolated otherwise.
    #[inline]
    fn sample(&self) -> (f32, f32) {
        if self.step == 1.0 {
            return self.clip.stereo_frame(self.position as usize);
        }
        let index = self.position.floor();
        let t = (self.position - index) as f32;
        let index = index as i64;
        let p0 = self.frame_at(index - 1);
        let p1 = self.frame_at(index);
        let p2 = self.frame_at(index + 1);
        let p3 = self.frame_at(index + 2);
        (
            catmull_rom(p0.0, p1.0, p2.0, p3.0, t),
            catmull_rom(p0.1, p1.1, p2.1, p3.1, t),
        )
    }

    /// The frame at `index`: wrapping round the region when looping, and
    /// silent beyond the clip's ends otherwise.
    #[inline]
    fn frame_at(&self, index: i64) -> (f32, f32) {
        let (start, end) = (self.start as i64, self.end as i64);
        let index = if self.looping && !(start..end).contains(&index) {
            start + (index - start).rem_euclid(end - start)
        } else {
            index
        };
        if (0..self.clip.frames() as i64).contains(&index) {
            self.clip.stereo_frame(index as usize)
        } else {
            (0.0, 0.0)
        }
    }
}

#[inline]
fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * (2.0 * p1
        + (p2 - p0) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

/// Mixes every voice into the output. Owned by the audio thread (or by a
/// test, through [`Engine::offline`](crate::Engine::offline)).
pub struct Mixer {
    rate: u32,
    declick: u32,
    /// Fixed at [`MAX_VOICES`] slots when built and never resized.
    voices: Vec<Option<Voice>>,
    master: Ramp,
    /// The voices' reverb sends for the current chunk, stereo interleaved.
    bus: Vec<f32>,
    reverb: Reverb,
    ambience: Ambience,
    /// Frames the reverb keeps running for without new input.
    reverb_tail: u32,
    reverb_remaining: u32,
    /// The loudest sample of the first two output channels since the last
    /// publish.
    output_peaks: [f32; 2],
    messages: rtrb::Consumer<Message>,
    events: rtrb::Producer<Event>,
    shared: Arc<Shared>,
}

impl Mixer {
    pub(crate) fn new(
        rate: u32,
        messages: rtrb::Consumer<Message>,
        events: rtrb::Producer<Event>,
        shared: Arc<Shared>,
    ) -> Self {
        assert!(rate > 0, "the mixer needs an output sample rate");
        Self {
            rate,
            declick: ((DECLICK_SECONDS * rate as f32) as u32).max(1),
            voices: (0..MAX_VOICES).map(|_| None).collect(),
            master: Ramp::at(1.0),
            bus: vec![0.0; CHUNK_FRAMES * 2],
            reverb: Reverb::new(rate),
            ambience: Ambience::new(rate, 0x0110),
            reverb_tail: (REVERB_TAIL_SECONDS * rate as f32) as u32,
            reverb_remaining: 0,
            output_peaks: [0.0; 2],
            messages,
            events,
            shared,
        }
    }

    /// The output sample rate the mixer renders at.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Replaces `out` with the next `out.len() / channels` frames of the mix,
    /// interleaved. Mono and stereo clips fill the first two channels; a
    /// mono output gets the average of left and right.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        assert!(channels > 0, "an output has at least one channel");
        debug_assert!(out.len().is_multiple_of(channels));
        self.receive();
        for chunk in out.chunks_mut(CHUNK_FRAMES * channels) {
            self.render_chunk(chunk, channels);
        }
        self.publish();
    }

    fn render_chunk(&mut self, out: &mut [f32], channels: usize) {
        let frames = out.len() / channels;
        let bus = &mut self.bus[..frames * 2];
        out.fill(0.0);
        bus.fill(0.0);
        for voice in self.voices.iter_mut().flatten() {
            if !voice.finished {
                voice.mix(out, bus, channels);
            }
        }

        // The ambience, under everything, quieter while anything plays.
        let sounding = self.voices.iter().flatten().any(|voice| !voice.finished);
        self.ambience.render(out, channels, sounding);

        if bus.iter().any(|&sample| sample != 0.0) {
            self.reverb_remaining = self.reverb_tail;
        }
        if self.reverb_remaining > 0 {
            let (sends, _) = bus.as_chunks::<2>();
            for (frame, send) in out.chunks_exact_mut(channels).zip(sends) {
                let (left, right) = self.reverb.process(send[0], send[1]);
                if channels == 1 {
                    frame[0] += 0.5 * (left + right);
                } else {
                    frame[0] += left;
                    frame[1] += right;
                }
            }
            self.reverb_remaining = self.reverb_remaining.saturating_sub(frames as u32);
        }

        let [mut left_peak, mut right_peak] = self.output_peaks;
        for frame in out.chunks_exact_mut(channels) {
            let gain = self.master.next();
            let left = (frame[0] * gain).abs();
            let right = frame.get(1).map_or(left, |&right| (right * gain).abs());
            left_peak = left_peak.max(left);
            right_peak = right_peak.max(right);
            for sample in frame {
                // A safety net, not a limiter: the limiter arrives in M1.
                *sample = (*sample * gain).clamp(-1.0, 1.0);
            }
        }
        self.output_peaks = [left_peak, right_peak];
    }

    fn fade_frames(&self, seconds: f32) -> u32 {
        ((seconds.max(0.0) * self.rate as f32) as u32).max(self.declick)
    }

    fn voice_mut(&mut self, id: VoiceId) -> Option<&mut Voice> {
        self.voices
            .iter_mut()
            .flatten()
            .find(|voice| voice.id == id && !voice.finished)
    }

    fn receive(&mut self) {
        while let Ok(message) = self.messages.pop() {
            self.apply(message);
        }
    }

    fn apply(&mut self, message: Message) {
        match message {
            Message::Play {
                voice,
                clip,
                settings,
            } => {
                let new = Voice::new(voice, clip, &settings, self.rate);
                match self.voices.iter().position(Option::is_none) {
                    Some(index) => {
                        // The slot's last voice may have left a level behind.
                        self.shared.slots[index].peak.clear();
                        self.voices[index] = Some(new);
                    }
                    // The engine admits at most MAX_VOICES live voices, so
                    // this cannot happen; if it did, the clip would be
                    // freed here.
                    None => debug_assert!(false, "no free voice slot"),
                }
            }
            Message::Stop {
                voice,
                fade_seconds,
            } => {
                let frames = self.fade_frames(fade_seconds);
                if let Some(voice) = self.voice_mut(voice) {
                    voice.stop(frames);
                }
            }
            Message::SetLooping { voice, looping } => {
                if let Some(voice) = self.voice_mut(voice) {
                    voice.looping = looping;
                }
            }
            Message::SetReverb { voice, reverb } => {
                let frames = self.declick;
                if let Some(voice) = self.voice_mut(voice) {
                    voice.send.towards(reverb.clamp(0.0, 1.0), frames);
                }
            }
            Message::StopAll { fade_seconds } => {
                let frames = self.fade_frames(fade_seconds);
                for voice in self.voices.iter_mut().flatten() {
                    voice.stop(frames);
                }
            }
            Message::Ambience(settings) => self.ambience.set(settings),
            Message::Instrument {
                instrument,
                samples,
            } => self.ambience.set_samples(instrument, samples),
            Message::MasterGain { gain } => {
                let frames = self.declick;
                self.master.towards(gain.max(0.0), frames);
            }
        }
    }

    /// Publishes positions and levels, and hands finished voices back to
    /// the engine.
    fn publish(&mut self) {
        let now = self.ambience.now().map_or(0, Now::pack);
        self.shared.ambience.store(now, Ordering::Relaxed);
        while self.events.slots() > 0 {
            let Some(samples) = self.ambience.take_released() else {
                break;
            };
            let pushed = self.events.push(Event::Released(samples));
            debug_assert!(pushed.is_ok(), "checked for a free slot first");
        }
        for (peak, published) in self.output_peaks.iter_mut().zip(&self.shared.output_peaks) {
            published.raise(*peak);
            *peak = 0.0;
        }
        for (slot, published) in self.voices.iter_mut().zip(&self.shared.slots) {
            let Some(voice) = slot else {
                published.voice.store(0, Ordering::Relaxed);
                continue;
            };
            published.voice.store(voice.id.get(), Ordering::Relaxed);
            published
                .position
                .store(voice.position as u64, Ordering::Relaxed);
            published.peak.raise(voice.peak);
            voice.peak = 0.0;
            // A finished voice keeps its slot until its event fits in the
            // queue, so its clip is never dropped here.
            if voice.finished && self.events.slots() > 0 {
                let Some(voice) = slot.take() else {
                    continue;
                };
                published.voice.store(0, Ordering::Relaxed);
                let pushed = self.events.push(Event::Ended {
                    voice: voice.id,
                    clip: voice.clip,
                });
                debug_assert!(pushed.is_ok(), "checked for a free slot first");
            }
        }
    }
}

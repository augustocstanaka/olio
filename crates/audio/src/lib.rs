//! Olio's audio engine.
//!
//! - [`clip`]: decode a sound file into a [`Clip`] held in memory, and write
//!   one back out as WAV.
//! - [`peaks`]: the waveform overview drawn on tiles.
//! - [`reverb`]: the room reverb voices can send to.
//! - [`mixer`]: the code that runs on the audio thread. It mixes voices into
//!   an output buffer and talks to the rest of the app only through
//!   lock-free queues (ADR 0002).
//! - [`engine`]: the app's handle on the mixer, and the output device stream
//!   that drives it.
//!
//! Nothing here depends on the UI or on the library model, so every mixing
//! rule is tested headlessly with [`Engine::offline`].

pub mod clip;
pub mod engine;
pub mod mixer;
pub mod peaks;
pub mod reverb;

pub use clip::{
    AUDIO_EXTENSIONS, Clip, DecodeError, VIDEO_EXTENSIONS, WavFormat, decode_file, is_supported,
    is_video,
};
pub use engine::{AudioError, Engine, OutputInfo, PlayOptions};
pub use mixer::{MAX_VOICES, Mixer, VoiceId};
pub use peaks::Peaks;
pub use reverb::Reverb;

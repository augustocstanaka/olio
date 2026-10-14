---
status: accepted
date: 2026-10-10
---

# Audio engine: cpal, Symphonia, an in-memory lock-free mixer

A soundboard must start a sound the instant its key is pressed, play many
at once without glitches, and never click. We built the engine from three
pieces:

- **cpal** for output (CoreAudio on macOS, WASAPI on Windows), at the
  device's own rate and format.
- **Symphonia** to decode every common format and the audio track of
  MP4/MOV video, in pure Rust with no system codecs.
- **Our own mixer** on the device's real-time thread: a fixed pool of 64
  voices, commands in and events out over wait-free ring buffers (`rtrb`),
  positions and peak levels published in atomics (levels as `f32` bits
  raised with `fetch_max` and taken with `swap`, so no peak between two
  reads is lost and neither side waits).

The ambience bed is synthesised in the mixer rather than played from
files: everything it needs (delay lines, voice pools, noise filters) is
allocated when the mixer is built, its composer plans with fixed-size
queues, and settings arrive as one `Message::Ambience` value. The mixing
rules apply to it unchanged. Its instruments go through a space of their
own (`ambience/hall.rs`: an ensemble chorus and a long feedback-delay-
network hall, also allocated up front) rather than the tiles' room, and
the ambience stays audible until the hall's tail has died away. A generated track is the same ambience
rendered by a mixer of its own on another thread (`Engine::offline`), so
it sounds like the live bed, reverb included; nothing new runs on the
audio thread for it.

Sounds are decoded fully into memory (`Clip`, interleaved `f32` at the
file's rate) on a background thread before they are played, so starting a
voice costs nothing but a queue push. Voices resample on the fly
(Catmull-Rom) when a clip's rate differs from the device's.

Per-tile shaping happens in the mixer, not in the file: the region (from
trim silence), the normalizing gain (worked out once per clip on the
decoder thread and passed as the voice's gain), fade in, fade out, gain
ramps (5 ms minimum, so stops never
click), and a send to one shared stereo reverb (Freeverb design) whose
output is mixed in. Farrago 2 moved fades into its editor as destructive
edits; we keep them as settings so they can change at any time without
re-encoding, and the planned editor (M3) adds destructive edits on top.

## Considered options

- **rodio / kira.** Higher-level crates with mixers of their own. Kira in
  particular is good, but we want exact control of voice lifetime (clips
  freed off the audio thread), per-device streams for preview and
  per-tile outputs (M2), and sample-accurate scheduling for list-mode
  cross-fades (M4).
- **Streaming from disk.** Lower memory for long tracks, but a disk read
  stands between the key press and the sound. In-memory clips are simpler
  and instant; a streaming voice for very long files can be added behind
  the same `Message::Play` later.
- **Per-voice reverb.** Each tile with its own reverb instance costs 64
  times the CPU for little audible gain; a shared send is how mixing desks
  do it.

## Consequences

- Memory grows with the decoded length of the sounds in use (about 10 MB
  per stereo minute at 44.1 kHz).
- The audio-thread rules in `CODING_STANDARDS.md` are the price of
  glitch-free playback: no allocation, locks, I/O or freeing there.
- Every mixing rule is tested by rendering the mixer offline; CI needs no
  audio device.
- Until the limiter (M1) lands, a hard clip at full scale guards the output.

# Architecture

Olio is four crates with one-way dependencies. Decisions and their reasons
live in [`adr/`](adr/); this document describes the shape that resulted.

```
 crates/app (olio)                               crates/control (olio-control)
 ┌─────────────────────────────────────┐        ┌───────────────────────────────┐
 │ app.rs       frame loop, execute()  │───────►│ command.rs  Command, setting  │
 │ grid.rs      tiles and waveforms    │        │ keymap.rs   slot <-> key      │
 │ sidebar.rs   sets                   │        └──────────────┬────────────────┘
 │ inspector.rs tile settings          │                       ▼
 │ import.rs    files, folders, boards │        crates/library (olio-library)
 │ cli.rs       `olio import`          │        ┌───────────────────────────────┐
 │ clips.rs     background decoding    │───────►│ model.rs    Library/Set/Tile  │
 └──────────────┬──────────────────────┘        │ store.rs    folder, imports   │
                │                               │ manifest.rs board JSON        │
                ▼                               │ folder.rs   natural order     │
 crates/audio (olio-audio)                      └───────────────────────────────┘
 ┌─────────────────────────────────────┐
 │ clip.rs     decode, trim, WAV out   │
 │ peaks.rs    waveform overview       │
 │ engine.rs   Engine + cpal stream ───┼──┐ ring buffers
 │ mixer.rs    voices, fades, bus  ◄───┼──┘ (audio thread)
 │ reverb.rs   room reverb             │
 └─────────────────────────────────────┘
```

## Threads

1. **The UI thread** runs egui's frame loop (`OlioApp::ui`). It owns the
   `Library`, the `Engine` and the clip cache. Every frame it collects
   commands (keys, clicks, toolbar), carries them out with
   `execute(Command)`, and saves the library half a second after the last
   edit.
2. **The decoder thread** (`clips.rs`) decodes sounds of the front set into
   `Clip`s, computes their peaks and audible region, and wakes the UI.
3. **The audio thread** belongs to the output device (cpal). It calls
   `Mixer::render` for every buffer.

The UI and audio threads share no locks. The engine sends `Message`s to the
mixer over a wait-free ring buffer (`rtrb`); the mixer sends `Event`s back
over another, and publishes each voice's position in atomics for progress
bars.

## Playing a tile

1. A key press becomes `Command::ToggleTile(tile)` (`keyboard_commands`
   maps the physical key to a slot with `TileKey`).
2. `execute` looks up the tile's clip; if it is decoded, it builds
   `PlayOptions` from the tile (region from *trim silence*, fades, reverb,
   loop) and calls `Engine::play`, which queues `Message::Play` with an
   `Arc<Clip>`.
3. On its next buffer the mixer puts the voice in a free slot (there are
   always enough: the engine refuses more than `MAX_VOICES`) and mixes it:
   it reads the region (interpolating when the clip's rate differs from the
   device's), applies gain ramps and the fade envelope, adds the dry signal
   to the output and the reverb share to the stereo send bus. The reverb
   processes the bus and is added in; the master gain and a hard clip
   finish the buffer.
4. When the voice ends, the mixer queues `Event::Ended` with the clip, so
   the last reference drops on the UI thread. `Engine::poll` hands the
   ended voice back and the tile stops showing as playing.

## Importing

`import.rs` is shared by the window (drop, Add Sound, Import menu) and
`olio import`. Loose files go into a set; a folder becomes a set named
after it, files in natural order (`folder.rs`), subfolders becoming their
own sets; a `.json` board (`manifest.rs`) adds or replaces whole sets.
`Store::import_sound` copies an audio file into `sounds/<hash>.<ext>`;
video files go through `Store::import_converted`, with a closure that
decodes the audio track and writes it as WAV, keeping the library crate
free of audio code.

While the window has a library open it holds `olio.lock` in it; `olio
import` refuses to write such a library (the window would save over the
import) unless forced.

## Where a feature goes

- A new way to shape playback (a pan, a filter): a field on `Tile` with a
  serde default, a `TileSetting` variant, a `PlayOptions` field, the mixer
  code with offline tests, then the inspector control.
- A new control surface (MIDI, OSC, Stream Deck): translate its input into
  `Command`s and send them to `execute`; never call the engine directly.
- A new import source: a function in `import.rs` that ends in
  `Store::import_sound` or `import_converted`, with tests in that file.

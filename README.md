# Olio

A keyboard-driven soundboard for macOS and Windows, in the spirit of Rogue
Amoeba's [Farrago](https://rogueamoeba.com/farrago/): sets of tiles laid out
on your keyboard, one key per sound, for live shows, podcasts, streams and
theatre. Rust audio engine, [egui](https://github.com/emilk/egui) desktop
app.

![Olio's main window: sets on the left, a grid of tiles, the inspector on the right](docs/images/window.png)

> *Olio* is a medley, the same meaning as *farrago*.

## Status

Milestone 0, the playable skeleton, is done. Olio can:

- **Play sounds from the keyboard.** Each set is a grid that follows the
  keyboard: `1`–`0`, `Q`–`P`, `A`–`;`, `Z`–`/`, and the same keys with
  Option (Alt) for four more rows. Press a key or double-click a tile to
  play it, press again to stop. Hold Esc to stop everything.
- **Shape each sound.** The inspector sets a tile's name, colour, loop, fade
  in, fade out (also used when you stop it) and reverb. Silence at the
  start and end of a sound is trimmed automatically. None of this changes
  the audio file.
- **Import in bulk.** Drop or pick files, whole folders (each becomes a set,
  in alphabetical order), or a board written in JSON. Video files (MP4,
  MOV, M4V, MKV) are imported as their audio track.
- **Script boards.** `olio import board.json` builds sets from a JSON file
  without opening a window; see [docs/BOARD-JSON.md](docs/BOARD-JSON.md).
- **Read most formats.** WAV, AIFF, CAF, FLAC, MP3, AAC and ALAC (M4A), Ogg
  Vorbis, and the audio of MP4/MOV video.

Next up ([docs/ROADMAP.md](docs/ROADMAP.md)): solo, pausable and
hold-to-play tiles, volume A/B, Pause/Fade All, list mode, a waveform
editor, MIDI, OSC and **Stream Deck** support.

## Build and run

The only prerequisite is a Rust toolchain from [rustup](https://rustup.rs);
`rust-toolchain.toml` pins the stable channel on every machine. The first
build compiles the GPU and UI stack and takes a few minutes.

```bash
cargo xtask run
```

To try it with sounds straight away, generate the demo pack (twelve
synthesised sounds and a board) into a scratch library and open it:

```bash
cargo xtask demo
```

```bash
cargo xtask run --library target/demo/library --set "Demo Board"
```

`cargo xtask run` builds the app and runs a copy of the binary, so it can be
rebuilt while a window is open (Windows locks a running executable).

## Your library

Olio keeps everything in one folder: `~/Library/Application Support/Olio`
on macOS, `%APPDATA%\Olio` on Windows. Override it with `--library DIR` or
the `OLIO_LIBRARY` environment variable.

- `library.json` holds your sets and tiles; the previous version is kept as
  `library.json.bak`.
- `sounds/` holds a copy of every imported sound, named by a hash of its
  contents, so moving or deleting the originals breaks nothing and a sound
  used twice is stored once.

## Development

```bash
cargo xtask check
```

Formatting, clippy with warnings denied, every test (the mixer, decoding of
each format, the library, the importer, and the window itself driven
headlessly through its accessibility tree), and a headless import of the
demo board through the real binary. CI runs the same command on macOS and
Windows. Enable the pre-commit hook once per clone with
`git config core.hooksPath .githooks`.

`OLIO_RENDER=1 cargo test -p olio-app --test window` renders the window
offscreen to `target/screenshots/window.png`.

Read [CLAUDE.md](CLAUDE.md) for the map of the code and
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how it fits together.

## License

MIT or Apache-2.0, at your option.

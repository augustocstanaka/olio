# Olio

A keyboard-driven soundboard for macOS and Windows, in the spirit of Rogue
Amoeba's [Farrago](https://rogueamoeba.com/farrago/): sets of tiles laid out
on your keyboard, one key per sound, for live shows, podcasts, streams and
theatre. Rust audio engine, [egui](https://github.com/emilk/egui) desktop
app.

![Olio's main window: sets on the left, a grid of tiles, the inspector on the right](docs/images/window.png)

The window follows the system theme; there is a [light version](docs/images/window-light.png)
too, or pick one in the View menu. View ▸ Soft Tiles draws tiles as
raised buttons that press in while they play:

![Soft tiles: raised at rest, pressed in while playing](docs/images/window-soft-light.png)

> *Olio* is a medley, the same meaning as *farrago*.

## Status

Milestone 0, the playable skeleton, is done. Olio can:

- **Play sounds from the keyboard.** Each set is a grid that follows the
  keyboard: `1`–`0`, `Q`–`P`, `A`–`;`, `Z`–`/`, and the same keys with
  Option (Alt) for four more rows. Press a key or double-click a tile to
  play it, press again to stop. Hold Esc to stop everything, `-` to fade
  everything out. The arrow keys move the selection and Return plays it.
- **See what is loud.** A stereo level meter beside the master volume,
  with a lamp that lights when the output clips; a meter on every playing
  tile; and the selected tile's level in the inspector.
- **Mark tiles with emoji.** Click the well beside a tile's title (or
  ⌘E / Ctrl+E) to pick one from a searchable picker; boards can set them
  too. Emoji are drawn in colour from Twemoji, the same on both
  platforms.
- **Polish in one click.** ✨ Polish in the inspector (⌘P / Ctrl+P, or
  right-click a tile) gives a tile a 1 s fade in, a 1 s fade out and
  30 % reverb; click again to take it off. Tile ▸ Polish All Tiles in Set
  does a whole set, handy after importing a folder. Change what Polish
  gives in Playback ▸ Polish Preset.
- **Stay in control.** A master volume and mute in the toolbar, a bubble
  for every playing sound, Fade All and Stop All. Removing a tile or a set
  asks first. Menus list every command with its shortcut (Help ▸ Keyboard
  Shortcuts has them all), written the way the platform writes them.
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
- `preferences.ron` remembers the window's size and position, the panel
  widths, the theme and tile style, the master volume, the Polish preset
  and which panels are shown.

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
offscreen in both themes and both tile styles to `target/screenshots/`
(`window.png`, `window-light.png`, `window-soft.png`,
`window-soft-light.png`).

Read [CLAUDE.md](CLAUDE.md) for the map of the code and
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how it fits together.

## License

MIT or Apache-2.0, at your option.

Emoji graphics are from [Twemoji](https://github.com/jdecked/twemoji),
Copyright 2020 Twitter, Inc and other contributors, licensed under
[CC-BY 4.0](https://creativecommons.org/licenses/by/4.0/), through the
[twemoji-assets](https://crates.io/crates/twemoji-assets) crate. Emoji
names and search data come from the
[emojis](https://crates.io/crates/emojis) crate (Unicode CLDR and GitHub
gemoji).

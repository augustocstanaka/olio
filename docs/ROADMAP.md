# Roadmap

Milestones are vertical slices: each is usable on its own and verified by
tests. The checklist follows Rogue Amoeba's Farrago 2 (manual and release
notes up to 2.2.0) as the bar for a complete soundboard; later milestones
are intentions, reordered as real use dictates.

## M0 - Playable skeleton (done)

- Workspace, docs, ADRs, CI on macOS and Windows, `cargo xtask`.
- Audio engine: cpal output, Symphonia decoding (WAV, AIFF, CAF, FLAC, MP3,
  AAC/ALAC, Vorbis, and the audio of MP4/MOV video), a real-time mixer with
  64 voices, rate conversion, de-click ramps, looping, offline tests.
- Library: sets of up to 80 tiles on the keyboard grid, sounds copied into
  the library by content hash, `library.json` saved atomically with a
  backup, never overwritten when unreadable.
- Window: sets sidebar (new, rename, remove, Cmd/Ctrl-1 to 9), tile grid
  with waveforms, key badges and progress, double-click or key to toggle,
  hold Esc to stop all, Stop All button, playing bubbles, drag and drop.
- Pulled forward on request: the **inspector** (title, colour, loop,
  trim silence, fade in, fade out, reverb), **automatic trimming of
  silence**, **fades**, a **room reverb** send, **folder import** in
  natural alphabetical order, **video audio extraction**, **board JSON**
  and the headless **`olio import`** for scripts.

## M1 - Core playback

- Solo (stops other tiles), Pausable (second press pauses), Only Play While
  Pressed (hold to play).
- Per-tile volume with A and B levels; master volume with A/B flags, a fade
  between them, and mute.
- Pause All (Space), Fade All (Dash, 2 s), Stop All and reset (Cmd-Esc);
  Shift+Ctrl solo, Shift+Ctrl+Cmd fade-and-solo.
- Selection: Cmd-click and drag selection, Return plays the selection.
- A peak limiter on the output instead of the hard clip; level meters in
  the toolbar and inspector.
- Tile context menu: duplicate, play counts and reset; drag tiles between
  slots and to other sets (copy, Cmd to move).
- Emoji on tiles (picker with search), the trigger bounce, single-click
  playback setting.
- Peaks cached on disk so large sets draw before decoding; decoding of all
  sets in the background; garbage collection of sounds no tile uses.

## M2 - Devices and settings

- Main and Preview output devices; Shift previews a tile on the Preview
  device (loop ignored, headphones badge).
- Per-tile output device and channel pair for multichannel interfaces.
- Following the default device and recovering when one is unplugged.
- Settings window (General, Controllers, Mappings, Search); theme choice;
  global Show Olio hotkey and Cmd-0.
- Background imports with progress, so long videos do not pause the window.

## M3 - Editor

- A waveform editor window: selection with handles and the I/O keys,
  remove and crop selection, destructive fades, zoom, undo/redo, revert to
  original, scrubbing (arrows: 1 s, Shift 5 s, Option 0.1 s), loop
  playback, playing through the Preview device. Edits are saved as a new
  copy; the original stays.

## M4 - Sets

- List mode: tiles in a column with notes, played/playing/unplayed marks,
  Continuous or Queued playback, cross-fade between tiles, Reset List,
  Return plays the queued tile.
- Smart sets from rules (title, colour, settings, play counts, duration),
  Favorites.
- Locked sets with an optional password.
- Library search across sets with highlights and Show in Set.
- `.olioset` export and import of a set with its sounds; Export All.
- Secondary set windows; previous/next set; reorder sets by drag and
  Option-Up/Down; sample sets.
- Export a set or library as board JSON, to edit and re-import.

## M5 - Control surfaces

- A local control API: WebSocket with JSON commands in and state
  snapshots and changes out (ADR 0004).
- **Stream Deck plugin** (Rust, Elgato SDK v2) on that API: Play Tile
  (with Follow Set), Selected Tile Controls, Select Set, Playback Control,
  Master Volume (with Stream Deck+ dials), Bring Olio Forward; keys show
  the tile's colour, title, progress and time left; install and remove
  from Settings.
- OSC with Farrago's address space (`/tile/<uuid>/play`,
  `/set/selected/tile/<x>/<y>/<action>`, `/transport`, `/master`,
  `/inspector`, `/list`, `/global`, `/ping`, pattern matching).
- MIDI: Learn per tile and set, global mappings, output feedback.
- Automations: tile and set events (started, paused, stopped, set
  selected, ...) run a command, a script with the event as JSON, an OSC
  message, or a macOS Shortcut. Shift at launch is Safe Mode.

## M6 - Distribution and polish

- Freesound search (with the user's API key), CC0/BY licences written to
  the tile's notes.
- `cargo xtask dist`: a signed macOS `.app` in a `.dmg`, a Windows
  installer; release workflow on GitHub; update check against releases;
  app icon.
- Accessibility review with VoiceOver and Narrator, building on the
  existing kittest window tests.

Out of scope: Apple Loops (macOS-only and licence-bound).

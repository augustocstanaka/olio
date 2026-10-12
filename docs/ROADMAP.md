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
- Window ergonomics: a menu bar (File, Edit, Set, Tile, Playback, View,
  Help) with
  every shortcut written the platform's way and a Keyboard Shortcuts
  window; master volume and mute; Fade All (`-`) beside Stop All
  (Cmd-Esc); arrow keys move the selection, Return plays it, Delete
  removes it after an in-window confirmation (sets too); right-click
  menus on tiles (rename, colour, remove) and sets; the single-click
  playback setting; the Sets and Inspector panels can be hidden; theme
  choice (system, light, dark); window size, panel widths and
  preferences remembered per library; tiles show the time left while
  playing.
- **Polish**: one click (⌘P) for a 1 s fade in, 1 s fade out and 30 %
  reverb, toggled; Polish All Tiles in Set; an adjustable Polish Preset.
- **Arranging**: drag tiles between slots (onto a tile swaps, with a
  preview; Option/Ctrl drops a copy) and onto sets in the sidebar (moved
  there in the same slot if free, or copied; held there, the set opens
  for a drop on a chosen slot), Duplicate (⌘D), Shift+arrows to
  move the selected tile; all undoable.
- **Grid size**: columns per set (⌘] / ⌘[, Fit to Window ⌘\\, Set ▸
  Columns, the inspector), never narrower than the tiles allow, undoable; Compact,
  Regular and Large tile sizes.
- **Undo** for removing tiles and sets (⌘Z, ⇧⌘Z, an Undo button in the
  status bar); tiles are removed without a confirmation.
- **Normalize** on by default: every sound at -18 dBFS RMS, peaks under
  -1 dBFS, as a voice gain.
- **Spectrograms** under each waveform and in the inspector, in the
  tile's colour.
- **Level meters**: the output in the toolbar (stereo, with a clip lamp),
  every playing tile, and the selected tile in the inspector.
- **Emoji on tiles**: colour Twemoji, a picker with search and groups,
  boards accept emoji or shortcodes.
- **Soft tiles** (View menu): raised tiles that press in while playing,
  a restrained take on neumorphism that keeps contrast in both themes;
  every tile dips briefly when it starts (the trigger bounce).

## M1 - Core playback

- Solo (stops other tiles), Pausable (second press pauses), Only Play While
  Pressed (hold to play).
- Per-tile volume with A and B levels; master volume A/B flags and a fade
  between them.
- Pause All (Space); Shift+Ctrl solo, Shift+Ctrl+Cmd fade-and-solo.
- Selection: Cmd-click and drag selection of several tiles.
- A peak limiter on the output instead of the hard clip.
- Play counts and reset.
- Peaks cached on disk so large sets draw before decoding; decoding of all
  sets in the background; garbage collection of sounds no tile uses.

## M2 - Devices and settings

- Main and Preview output devices; Shift previews a tile on the Preview
  device (loop ignored, headphones badge).
- Per-tile output device and channel pair for multichannel interfaces.
- Following the default device and recovering when one is unplugged.
- Settings window (General, Controllers, Mappings, Search); global Show
  Olio hotkey and Cmd-0.
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

---
status: accepted
date: 2026-10-10
---

# Control surfaces: one command vocabulary; Stream Deck through a local API

Olio will be driven from the keyboard and mouse, MIDI controllers, OSC apps
(TouchOSC), Elgato Stream Decks and automations. Each of these turns its
input into an `olio_control::Command` (toggle, play or stop a tile, stop
all, select a set, change a tile setting), and the app carries out every
command in one function, `OlioApp::execute`. Tiles and sets are addressed
by UUID, so a command means the same thing whichever set is in front.

**Stream Deck** support is a committed feature (M5). The plan:

1. The app serves a **local control API**: a WebSocket on localhost
   speaking JSON. Commands come in; a full state snapshot (sets, tiles,
   colours, keys, what is playing) goes out on connect, then changes and
   playback progress.
2. A **Stream Deck plugin** written in Rust (`crates/streamdeck-plugin`,
   packaged as `io.github.augustocstanaka.olio.sdPlugin`, SDK v2 manifest with a
   native `CodePath` for each platform) registers with the Elgato software
   over its WebSocket (`-port -pluginUUID -registerEvent -info`) and talks
   to Olio's API.
3. Actions mirror Farrago's: Play Tile (with Follow Set), Selected Tile
   Controls, Select Set, Playback Control, Master Volume (dials on Stream
   Deck+), Bring Olio Forward. Keys are drawn by the plugin from state
   updates: the tile's colour, title, a progress bar and the time left.
4. Settings installs and removes the plugin by copying it into the Elgato
   plugins folder.

OSC (UDP, Farrago's address space) and MIDI (`midir`, Learn and global
mappings) become two more translators into commands.

## Considered options

- **Plugin talks OSC.** Reuses one protocol, but OSC has no good way to
  send the state snapshot the keys need to draw themselves.
- **Driving the Stream Deck directly over HID** (`elgato-streamdeck`
  crate). Works without Elgato's software, but most users have it running
  and it owns the device. Kept as an optional later mode.
- **A Node.js plugin with the official SDK.** Elgato's recommended path;
  rejected to keep one language and toolchain.

## Consequences

- Nothing outside `execute` may start or stop voices; review enforces it.
- The control API needs an authentication story before it listens beyond
  localhost.

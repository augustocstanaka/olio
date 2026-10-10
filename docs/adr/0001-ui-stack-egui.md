---
status: accepted
date: 2026-10-10
---

# UI stack: egui + eframe

Olio needs a desktop app for macOS and Windows that one person can
maintain, with a real-time audio engine in Rust and a custom-drawn grid of
tiles with live waveforms and progress. We chose a pure-Rust
immediate-mode stack: `eframe`/`egui` for the window and widgets (the same
stack as the author's other project, Skein), `rfd` for native dialogs, and
`egui_kittest` to drive the real window in tests. One toolchain builds
everything on both machines, and the UI and engine are plain Rust in one
process.

## Considered options

- **Tauri with a web UI.** The closest visual match to Farrago and the
  richest widget ecosystem. Rejected for now: a second toolchain
  (Node/npm), and an IPC and serialisation boundary between the UI and an
  engine that reports positions many times a second. The engine and
  library crates would move over unchanged if egui's look becomes the
  limiting factor.
- **Slint, iced.** Good Rust toolkits; the tile grid and waveform editor
  would be custom-drawn anyway, multi-window support (secondary set
  windows, the editor) and accessibility are less mature, and they are new
  to us.
- **Native per platform (SwiftUI + WinUI).** The most native result at twice
  the UI code.

## Consequences

- Immediate mode: the UI is rebuilt every frame from plain structs, so
  state is ordinary Rust data and easy to test. Frames are only requested
  while something moves (playing tiles, held Esc, pending saves).
- The look is "app-like", not native; tiles and waveforms are painted with
  `egui::Painter`.
- Screen readers get labels through AccessKit, which the window tests also
  use to find widgets.
- egui releases about quarterly with breaking changes: all egui-family
  crates are pinned to one generation and upgraded together
  (`docs/EGUI-NOTES.md`).

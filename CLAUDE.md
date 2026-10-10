# Olio

Keyboard-driven soundboard for macOS and Windows, Farrago-class. Rust audio
engine plus egui desktop app. The vocabulary is fixed in `GLOSSARY.md`
(set, tile, slot, sound, clip, voice, ...); use those words in code, docs
and UI text.

## Map

- `crates/audio` (`olio-audio`): decoding (`clip.rs`), waveform peaks,
  the real-time `mixer.rs` (voices, fades, regions, reverb send), the
  `reverb.rs` effect, and `engine.rs` (the app's handle on the mixer plus
  the cpal output stream). No UI or library dependency.
- `crates/library` (`olio-library`): `model.rs` (Library, Set, Tile,
  Sound, Slot), `store.rs` (the library directory, imports, lock file),
  `manifest.rs` (board JSON), `folder.rs` (natural-order listings). No UI
  or audio dependency.
- `crates/control` (`olio-control`): the `Command` vocabulary every control
  surface speaks, and `keymap.rs` (slot <-> key).
- `crates/app` (`olio-app`, binary `olio`): `app.rs` frame loop and
  `execute(Command)`, `grid.rs` tiles, `sidebar.rs` sets, `inspector.rs`
  tile settings, `import.rs` files/folders/boards (shared with `cli.rs`,
  the headless `olio import`), `clips.rs` background decoding.
- `xtask/`: the `cargo xtask` commands below; `demo.rs` synthesises the
  demo pack.
- `docs/ARCHITECTURE.md`: threads, data flow, where a feature goes.
- `docs/EGUI-NOTES.md`: read before changing UI code; egui 0.36 API traps.
- `docs/BOARD-JSON.md`: the board format users script against. Keep it in
  step with `manifest.rs`.
- `docs/adr/`: read the matching ADR before changing the UI stack (0001),
  the audio engine (0002), the library format (0003), or control surfaces
  and the Stream Deck plan (0004).
- `docs/ROADMAP.md`: milestones and the Farrago parity checklist.
- `CODING_STANDARDS.md`: the rules review enforces, above all the
  audio-thread rules.

## Commands

- `cargo fmt --all && cargo xtask check` before committing; CI runs
  `cargo xtask check` on Windows and macOS.
- `cargo xtask run [args]` to try the app (runs a copy of the binary, so
  you can rebuild while a window is open). `cargo xtask demo` builds a demo
  library to open with `--library target/demo/library`.
- `OLIO_RENDER=1 cargo test -p olio-app --test window` renders the window
  to `target/screenshots/window.png`; use it for visual changes. The
  window's own `--screenshot` only works while the window is visible on
  screen.

## Platform notes

Developed on an Apple-silicon Mac and Windows. Audio goes through cpal
(CoreAudio, WASAPI); CI runners have no audio device, so every mixing rule
is tested by rendering the mixer offline (`Engine::offline`), and window
tests run with `Options { silent: true }`. Native dialogs come from `rfd`
and block the frame; that is intended.

## Agent skills

### Issue tracker

GitHub Issues on augustocstanaka/olio via `gh`. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default labels, unchanged. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` at the root.

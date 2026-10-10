# Coding standards

Read by `/code-review` on every diff. Formatting, lints and tests are
enforced by `cargo xtask check`; this file holds the judgement calls no
check can make.

## Vocabulary

- Names in code, tests, comments, UI text and docs use the terms in
  `GLOSSARY.md`. A word on a term's _Avoid_ list appears only where it
  names a third-party type.
- A new domain concept gets its glossary entry in the same change.

## The audio thread

`Mixer::render` and everything it calls run on the device's real-time
thread. A glitch there is audible on stage, so:

- No allocation, no freeing, no locks, no I/O, no logging, no panics on
  input data. Buffers are sized when the mixer is built; clips arrive in
  `Arc`s and leave in `Event::Ended` so their memory is freed on the app's
  thread.
- The app reaches the mixer only through `Message`s on the ring buffer
  and reads it only through `Event`s and the `Shared` atomics.
- Every level change is ramped (at least the 5 ms de-click), so nothing
  clicks.
- Every mixing rule has a test that renders through `Engine::offline` and
  checks samples. Listening is not a test.

## Crates

- `olio-audio` and `olio-library` depend on no UI crate and not on each
  other. Where an operation needs both (extracting a video's audio on
  import), the app composes them (`import.rs`), passing a closure into the
  library.
- Every playback request goes through `olio_control::Command` and
  `OlioApp::execute`, so keyboard, mouse and future control surfaces
  cannot drift apart.

## Library and boards

- A format change older builds cannot read bumps `FORMAT_VERSION`; an
  additive field with a serde default does not. Either way, a test loads
  the older shape (`the_first_format_loads_with_defaults_for_fields_added_since`).
- Olio never overwrites a library it failed to read.
- Tile settings (fades, trim, reverb, loop) are applied at playback; they
  never rewrite the stored sound.
- Board JSON rejects unknown fields and out-of-range values with a message
  naming the set and file. A change to the board format changes
  `docs/BOARD-JSON.md` and the manifest tests in the same diff.

## Decisions

- A change that contradicts an ADR in `docs/adr/` supersedes it with a new
  ADR in the same change; otherwise the contradiction is a finding.

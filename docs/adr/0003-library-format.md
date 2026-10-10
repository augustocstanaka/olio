---
status: accepted
date: 2026-10-10
---

# Library format: one folder, JSON, sounds by content hash; boards for scripts

The library lives in one folder (`~/Library/Application Support/Olio`,
`%APPDATA%\Olio`, or `--library` / `OLIO_LIBRARY`):

- `library.json`: sets and tiles, `serde_json`, with a `format_version`.
  Saved through a temporary file and a rename, keeping the previous file as
  `library.json.bak`. A library Olio cannot read is never overwritten.
- `sounds/<blake3>.<ext>`: a copy of every imported sound, named by the
  hash of its bytes, so originals can move and identical files are stored
  once. Video imports store their extracted audio as WAV instead.
- `olio.lock` while the window has the library open.

Tiles and sets have UUIDs, the stable addresses control surfaces will use
(ADR 0004). New tile fields are added with serde defaults, so older
libraries keep loading without a version bump; a change older builds could
not read bumps `FORMAT_VERSION`, and newer files are refused with a clear
message rather than misread.

`library.json` is for Olio, not for people: UUIDs and hashes make it
awkward to write. People and scripts write **boards** instead
(`docs/BOARD-JSON.md`): sets of tiles by file path, with optional key,
colour, loop, trim, fades and reverb, imported in one go from the window
or with `olio import`. Boards reject unknown fields so a typo is reported,
and `"replace": true` makes re-importing a board idempotent.

## Considered options

- **SQLite.** Better for thousands of sets and concurrent writers; overkill
  for a document a few hundred kilobytes long, and harder to inspect.
- **Referencing original files in place.** No copies, but moving a file
  breaks a show; Farrago copies for the same reason.
- **Making `library.json` itself hand-editable.** Paths and keys instead of
  hashes and slots would make it friendlier, but it would then have two
  audiences with different needs; boards keep the scripting format small
  and stable.

## Consequences

- Deleting a tile does not yet delete its sound; garbage collection of
  unused sounds is planned (M1).
- `olio import` refuses a locked library unless `--force`, since the window
  would save over the import.

---
status: accepted
date: 2026-11-02
---

# Soft UI: one skin for the whole window, and panels in sections

Olio drew with egui's default look, plus a choice of Flat or Soft tiles.
Its inspector and Generator were long columns of equal-weight headings,
and the Generator hid half its settings behind one global Details switch,
which scattered them through every group at once. People found both
panels confusing. The sibling projects Sala and Toca share a soft-UI
skin that reads well and is tested for contrast.

## Decision

- **One skin, Sala's and Toca's, for the whole window.** `skin.rs` and
  `theme.rs` are ported from Toca (copied: there is no shared crate):
  one matte surface lit from the top left, every part raised out of it or
  pressed into it, the palette computed in OKLCH from one hue (a cool
  grey, the accent its amber complement) in a lightness table per mode,
  kept in the egui context and installed each frame for the theme egui
  resolves. Window code paints no colour literal, gradient or shadow; a
  new part goes in `skin.rs` and `docs/DESIGN.md`.
- **Soft tiles only.** The Flat/Soft choice goes: a tile is raised at
  rest, pressed in and tinted while it plays, ringed in its colour when
  selected. Two tile styles meant two looks for every state to design and
  test; the soft one already was the more legible one. Old preferences
  that name a tile style still load (the field is ignored).
- **Tile colours stay the user's.** They are not palette tokens; each is
  moved lighter on a dark surface and darker on a light one until it
  reads 3:1, tested.
- **Panels in sections.** Each panel is a short column of sections that
  open and close (Sound, How it plays, Polish; Style, Key and mood,
  Energy and time, Background, Instruments), kept in the
  preferences per panel, all open at first.
- **A More per section, not a global Details.** What refines a group
  (the character's amounts, the form, each instrument's level and notes)
  sits at the end of that group behind its own More, so it opens where it
  applies. Preferences saved with Details on open every More once.
- **The section is a skin part, not egui's `CollapsingHeader`.** Its
  look is egui's, not the skin's; its arrow glyph renders as a box in
  egui's fonts; its state lives in egui's memory rather than the
  preferences; and a section's title is an uppercase legend. The part
  reports itself as a toggled button named by its title, and keeps its
  body in one scope so the ids of what follows never move.

## Consequences

- Every look is checked in two themes instead of four combinations.
- Contrast is a test: text on every surface, legends, the accent, the
  alarm, and the nine tile colours.
- Hand-made sliders take AccessKit's set-value and step actions, as
  egui's do, so the window tests and assistive technology drive them.
- A collapsed section's controls are absent from the accessibility tree;
  tests open the section (or its More) first.
- The look follows Sala and Toca; a change to their skin is not picked up
  automatically. Porting a fix is a copy, noted in the commit.
- Considered and rejected: keeping egui's look with only regrouped
  panels (the user asked for the sibling projects' look); a shared skin
  crate (three repositories, three release cadences, for ~1,000 lines);
  tabs instead of sections in the Generator (hides the relation between
  groups and costs a click to compare them).

# Design guidelines

How Olio's window looks and why. `crates/app/src/skin.rs` draws every
part (surfaces, tiles' bodies, buttons, chips, tabs, switches, sliders,
readouts, sections) and `crates/app/src/theme.rs` computes the colours it
draws with; change them and this together. The words are the glossary's
(`GLOSSARY.md`, "The window's look"). The model is Sala's and Toca's,
carried over; what Olio adds and leaves out is noted at the end, and the
decision is ADR 0006.

## Principle

Olio is soft UI: one matte surface, and every control is that surface
pushed out or pressed in. A tile at rest is a block raised out of the
grid, a tile playing is the same block pressed in and tinted with its
colour, an empty slot is a well, a key is a small dent in its tile, a
chosen chip is pressed in and lit. Nothing floats, nothing is glass or
metal, there is no texture. The eye reads depth from two soft shadows and
state from colour.

Test for any part: with the shading removed, does the hierarchy still
read? If a shadow is the only thing saying what is pressed, lit or chosen,
it is not enough (Malewicz's rule; see Sources). So every state is a
colour as well as a shape, and the text never dims for style.

## Light

One light, up and left (135°). Everything is derived from it:

- A **raised** part (a tile, a button, a chip, a tab, a slider's thumb, a
  window) casts the surface lit (`light`) up-left and the surface shaded
  (`shade`) down-right, both soft, at equal offsets. Its body is the
  surface, a touch lighter at its top-left corner than at its
  bottom-right.
- An **inset** part (a well, a field, a readout, a slider's track, a
  playing tile, a chosen chip, the set in front) has the dark shadow
  inside its top and left edges and the light inside its bottom and
  right. Pressed is inset: the same part, dented in.

Shadows never stack: a part is raised or inset, once. There is no rim
line, engraving or sheen; the one rim is the selected tile's ring, in the
tile's own colour, because a selection must read at a glance on any of
nine colours.

| Elevation (`skin::Elevation`) | Parts | Shadows (offset, blur) |
|---|---|---|
| Panel | windows, menus, a tile's ghost while it is dragged | 6 pt, 16 pt |
| Low | tiles, buttons, chips, tabs, bubbles | 3 pt, 8 pt |
| Thumb | slider and switch thumbs | 2 pt, 5 pt |
| Inset | wells, fields, readouts, tracks, a playing tile, a chosen chip | 4 pt bands inside the edges, blurred |

An inset's bands are clipped to its rectangle; its rounded corners are put
back in the colour around it (`skin::inset_on`), so a pill stays a pill.

## Material and colour

There is one material, the **surface**, and every colour in the window
but the tiles' is that surface's hue at another lightness and chroma
(`theme.rs`). The palette is computed in OKLCH, so lightness, and with it
every contrast, is set once per mode and holds.

| Token (`Palette`) | What it is | Use |
|---|---|---|
| `surface` | the material | the body, every raised part |
| `sunken` | the surface a little deeper | the floor of a field, a readout, the emoji well |
| `groove` | deeper still | slider tracks, the level meters' tracks, a switch that is off |
| `light` / `shade` | the surface lit and shaded | the two shadows |
| `text` / `text_weak` / `legend` | the surface's hue, nearly grey, far from it in lightness | text, secondary text, the uppercase legends and empty slots' keys |
| `accent` | the surface hue's complement, amber | lit things: a chosen chip or tab and its lamp, a slider's travel, a switch that is on, a More that is open, a readout, the level meters |
| `accent_dim` | the accent faded into the surface | a drop target, a selection |
| `on_accent` | text that reads on the accent | reserved |
| `alarm` | red | errors, a meter near full scale, the clip lamp, Remove Tile and every confirming verb |

One accent and no more. Grey is everything else.

The **tile colours** (`palette.rs`, nine of them) are the user's, not the
skin's: a tile's title, waveform and selected ring are its colour, moved
lighter on a dark surface and darker on a light one so each reads 3:1 on
the surface (`palette::tests::tile_colours_read_on_the_surface`). A tile's
body is the surface with a hint of its colour (6 %), more while it plays
(18 %).

### Themes

The theme preference (Match System, Light, Dark; the View menu) picks
egui's theme, and `skin::sync` installs the palette for the theme egui
resolves to at the top of every frame, so a change of the Mac's
appearance takes effect at once. Light and dark are one lightness table
each (`theme::LIGHT`, `theme::DARK`) over one hue (`theme::HUE`, 250°, a
cool grey; the accent follows at 180°): a light surface sits at OKLCH L
0.91 and a dark one at 0.30.

Contrast is tested in both modes (`theme::tests::text_reads_on_every_surface`):
7:1 for the main text on the surface and on a field, 4.5:1 for secondary
text, legends, the accent on the surface and on a readout, and text on a
selection; 3:1 for the alarm (WCAG 1.4.3 and 1.4.11). A new token is a new
row in both lightness tables and a new line in that test.

Radii: panel 16, well 10 (tiles and slots too), button 9, readout 7, row 5
(`PANEL_RADIUS` and friends). Nothing else; a new radius is a new token.

## Parts

Every interactive part has a rest, hover, pressed and chosen look, and
the chosen look is never shading alone: it is lit (accent) as well. Every
part carries an AccessKit label (`Response::widget_info`), so assistive
technology and the window tests find it by name.

- **Tile** (`grid.rs`, from `raised_with` and `inset`): raised at rest,
  pressed in and tinted while it plays, the ring in its colour when
  selected; lifted to a window's height while dragged.
- **Key badge**: a pill dented into its tile, tinted with its colour.
- **Empty slot**: a well, its key in the legend colour; lit in
  `accent_dim` with an accent ring where files would land.
- **Button** (`skin::button`, `wide_button`, `picture_button`): raised;
  level with the surface while held; inset, tinted and lettered in the
  accent when it is the current state (a polished tile's Polish); flat
  and in weak text when disabled. A wide button fills its group and
  carries its shortcut at the right (Play, Polish, Generate Track).
- **Alarm button** (`skin::alarm_button`): a button lettered in the alarm
  colour, for what takes something away.
- **Chip** (`skin::chip`): one of a few choices side by side (a mood, a
  length, an instrument, a background with its picture), a small button
  pressed in and lit when chosen.
- **Tab** (`skin::tab`): the status bar's Sets, Generator and Inspector;
  the chosen one pressed in, lit, with a lamp.
- **Bubble** (`skin::bubble`): a playing tile in the toolbar, raised and
  tinted with its colour; a click stops it.
- **Switch** (`skin::switch`): a raised thumb on an inset track that fills
  with the accent when on. Every on/off setting in the panels.
- **Slider** (`skin::slider`) and **slider row** (`skin::slider_row`): a
  raised thumb on an inset track, the travel lit; in a row, the name at
  the left and the value in a readout at the right, where the eye lands.
  A click or a drag sets it; with focus the arrows step it by a
  hundredth; assistive technology sets or steps it as egui's own.
- **Readout** (`skin::readout_label`): monospace text in the accent on a
  small inset: a percentage, a fade's seconds, a level.
- **Legend** (`skin::legend_label`): uppercase 11 pt in `legend`, flat:
  Sets, Track, Mood, Over time, Form.
- **Section** (`skin::section`): a legend with a chevron, the whole row a
  toggle, the group's controls under it while open. Open or closed is
  kept in the preferences, per panel. Its name is a title for assistive
  technology (a toggled button), and it names the first control in it
  that has none (the Style drop-down).
- **More** (`skin::more`): the last thing in a section with settings
  that refine it: "More" with a chevron while they are hidden, "Less" in
  the accent while they show. Known as "More in {section}".
- **Field** (`skin::field`): egui's text field on an inset of its own.
- **Lamp** (`skin::lamp`): lit, a dot with a glow; off, a dimple.
- egui's own widgets (menus, drop-downs, progress bars, tooltips,
  windows, the confirmation) are restyled by `skin::install` to the same
  tokens.

## Panels

The inspector and the Generator are each a short column of sections, so a
person sees groups before controls and opens only what they need.

- What identifies the thing stays in view above the sections: the tile's
  emoji, title, colour and file; the Generator's live switch and level.
- A section holds one job: **Sound** (hear it), **How it plays**,
  **Polish** (the fades and reverb); **Style**, **Key and mood**,
  **Energy and time**, **Background**, **Instruments**. Every section
  starts open.
- What refines a section, and most people leave alone, sits under its
  **More**: the character's three amounts, the form, each instrument's
  level, notes and register.
- What the Generator makes stays pinned under the sections, always in
  reach: the Track panel.
- Gaps, not lines, between groups; legends name groups inside a section.
- A control that is not obvious says what it does on hover, in a full
  sentence that names its effect, and its shortcut where it has one.
  Obvious ones (a section's title, Cancel) say nothing.

## Do and don't

- Do give every state a colour as well as a shape. Don't rely on an inset
  alone to mean "chosen".
- Do keep the light at 135°. Don't draw a highlight at the bottom or a
  shadow at the top of anything.
- Do make a part from `raised`, `inset` and the palette tokens. Don't
  paint a colour literal, a gradient or a shadow in `app.rs`, `grid.rs`,
  `inspector.rs`, `generator.rs`, `sidebar.rs` or the other window code;
  a one-off tint is a token blended towards another token, and a new part
  goes in `skin.rs` and in this file.
- Do leave the surface between groups empty. Don't fill a gap with a
  frame or a separator.
- Do keep text at least 11 pt. Don't dim text for style: a low-contrast
  surface is the whole point of soft UI, low-contrast text is its
  commonest failure.
- Do put a value where the eye lands: a readout beside its slider. Don't
  show a value only while dragging.
- Do draw with shapes (chevrons are lines: `▾` is a box in egui's
  fonts). Don't ship bitmaps for parts.
- Do keep hit areas at least 24 pt (WCAG 2.5.8). Don't let a hit area
  drift from the drawn part.
- Do check a change in both appearances. Don't tune a colour for one.

## Checking a change

`OLIO_RENDER=1 cargo test -p olio-app --test window` renders the window in
both themes, the inspector with and without a tile, and the Generator
with a More open, to `target/screenshots/`; look at
them before calling a change done, and copy them to `docs/images/` when
they change. Check the narrowest panels too: the inspector at 240 pt, the
Generator at 320 pt, the window at its 720 × 460 minimum.

## Olio's own, and what it leaves out

Added to Sala's and Toca's model: the tile (raised, pressed in while
playing, the ring), the key badge and empty slot, the chip, the bubble,
the wide and alarm buttons, the slider row, the section and its More.
Left out: Sala's knobs and theme variants, Toca's screens, visualizer and
heat ramp.

## Sources

- Michal Malewicz, "Neumorphism in user interfaces" (Hype4 Academy, 2019):
  two shadows, a mid-tone background, pressed as inverted shadows, and the
  warning that the pressed state alone has too little contrast.
  https://hype4.academy/articles/design/neumorphism-in-user-interfaces
- Setproduct, "Neumorphism design guide": dark-mode values, where the
  style fits and where it does not.
  https://www.setproduct.com/blog/neumorphism-design-guide
- Axess Lab, "Neumorphism will kill your product's accessibility": the
  3:1 rule for edges and states. https://axesslab.com/neumorphism/
- Björn Ottosson, "A perceptual color space for image processing" (OKLab,
  2020). https://bottosson.github.io/posts/oklab/
- WCAG 2.2: 1.4.3 (text 4.5:1), 1.4.11 (non-text 3:1), 2.5.8 (24 px
  targets).
- Nielsen Norman Group, "Progressive Disclosure": the essentials first,
  the rest one step away. https://www.nngroup.com/articles/progressive-disclosure/

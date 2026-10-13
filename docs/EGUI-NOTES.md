# egui notes

The egui family (`eframe`, `egui`, `egui_kittest`) is pinned to one
generation in the workspace `Cargo.toml` and upgraded together; these crates
break APIs every release. This file caches the traps that cost time on the
current generation. For anything not here, the installed source is the
truth: `~/.cargo/registry/src/index.crates.io-*/<crate>-<version>/`.

On a generation bump, re-check every line below and update the versions.

## eframe 0.36 / egui 0.36

- The app entry point is `App::ui(&mut self, ui: &mut Ui, frame)`, not
  `update(ctx, frame)`. With the default wgpu renderer, `on_exit(&mut
  self)` takes no arguments (the glow variant takes a context).
- Panels are `egui::Panel::top/bottom/left/right(id)`, shown with
  `.show(ui, ...)`, not `TopBottomPanel`/`SidePanel`.
- `DroppedFile` is a trait: read the path with `file.path()`.
- Key events carry `key` (logical) and `physical_key`. Tile keys use the
  physical key, so the grid keeps its shape on AZERTY or Dvorak and with
  Option held on macOS (where the logical key becomes a symbol).
- Screenshots: send `ViewportCommand::Screenshot(UserData)`; the image
  arrives frames later as `Event::Screenshot`. eframe only paints, and so
  only captures, while the window is visible: a covered window never
  delivers one. `--screenshot` opens the window on top for that reason;
  for reliable images render offscreen with kittest instead.
- Text in a fixed width: `LayoutJob::simple_singleline` plus
  `TextWrapping::truncate_at_width`, then `painter.layout_job`.
- Glyphs outside the default fonts (such as `▾`) draw as boxes; the emoji
  font covers `🔁`, `🔊`, `🔇`, `■`, `▶`, `⏹`, `●`, and `✔` (the full list
  is in egui's `lib.rs` under "special emojis").
- Menus: `egui::MenuBar::new().ui(ui, ...)` with `ui.menu_button` inside;
  a `menu_button` inside a menu becomes a submenu. `ui.close()` closes the
  menu after a click. `Button::shortcut_text` puts the shortcut on the
  right; it becomes part of the accessibility label ("Add Sound… ⌘O"), so
  tests query it with `get_by_label_contains`. Menus have no check marks:
  `menu.rs` prefixes `✔ ` itself.
- Shortcuts: declare `KeyboardShortcut`s once, show them with
  `ctx.format_shortcut` and take them with
  `input_mut(|i| i.consume_shortcut(..))`, which removes the key event so
  nothing else sees it. `Modifiers::COMMAND` means Cmd on macOS and Ctrl
  on Windows. `gui_zoom::zoom_menu_buttons` gives the zoom items and egui
  handles the zoom keys itself. `format_shortcut` writes ⌘⇧⌥ only if the
  Body font has those glyphs, else "Cmd+Shift+O"; egui's bundled fonts do
  not, so `fonts.rs` adds macOS's Apple Symbols as a fallback family.
- `egui::Modal` draws in an `Area` with a backdrop; `should_close()`
  consumes Escape, so check your own keys (Return) after it. Buttons in a
  modal are found by label like any other.
- Shadows: `egui::Shadow { offset, blur, spread, color }.as_shape(rect,
  radius)` gives a blurred rectangle; the blur reaches half its width on
  each side of the (offset) rect. egui has no inner shadow: the Soft
  pressed tile paints a dark rim colour, then a blurred lighter shape
  shrunk and offset inside it, through `painter.with_clip_rect(rect)`.
  Shadows reach past their tile, so a Soft grid is inset to leave room.
- Per-widget animation state that egui does not keep (when a tile
  started, for the dip) lives in `ctx.data_mut(|d| d.insert_temp(id, ..))`
  and is read back with `get_temp`; time comes from `input.time`, which
  kittest advances by `step_dt`, so animations are deterministic in tests.
- Pictures: decode once (`image` crate), `ctx.load_texture(name,
  ColorImage, options)`, keep the `TextureHandle` (dropping it frees the
  texture) and paint with `painter.image(id, rect, uv, WHITE)`. Use
  `mipmap_mode: Some(Linear)` for pictures drawn smaller than they are.
  In a `Button`, an `Image` is an atom: `Button::new((text, image,
  text))`; the accessible label joins the text atoms with spaces.
- One texture, any colour: make it white with the data in the alpha
  channel, then `painter.image(id, rect, uv, tint)` multiplies by the tint
  (the spectrogram strips). The `uv` rect crops it (a trimmed region).
- Modals and windows take a few frames to size and place themselves: in
  tests, step about ten frames after opening one before clicking its
  buttons, or the click lands where the button was in the sizing pass
  and counts as a click outside.
- Shortcut matching ignores extra Shift and Alt: ⌘Z also matches a
  press of ⇧⌘Z. `menu.rs` checks shortcuts with more modifiers first.
- A focused `Slider` reads the arrows with `num_presses`, so a consumed
  arrow shortcut never reaches it, and egui keeps its claim on them
  (`set_focus_lock_filter`) private. Wrap every slider in
  `menu::keeps_arrows(ui.add(..))`: while it has focus, the tile arrows
  leave the keys to it. Sliders take focus from Tab, never from a click.
- Drag and drop: give the source `Sense::click_and_drag()` (clicks and
  double-clicks still work under the drag threshold) and call
  `response.dnd_set_drag_payload(id)` every frame; read it anywhere with
  `DragAndDrop::payload::<T>(ctx)`. egui clears it after the frame the
  button is released and on Escape. Take the drop point and modifiers
  from the `PointerButton { pressed: false }` event: kittest's `drop_at`
  removes the pointer in the same frame. A ghost goes on an
  `Order::Tooltip` layer painter; `painter.multiply_opacity` fades a copy
  of the painter, not the original. While something is dragged, other
  widgets' `hovered()` is false: use `contains_pointer()` for drop
  targets (the sidebar's set rows). `DragAndDrop::has_any_payload` turns
  false the frame after a drag ends, dropped or cancelled: the app uses
  that to leave a spring-loaded set. A hold timer must call
  `request_repaint` itself, since a still pointer sends no events.
- Long grids: `ScrollArea::show_rows(ui, row_height, rows, |ui, range|)`
  only lays out the visible rows (the emoji picker's 1,900 emoji).
- A panel inside a panel: `Panel::bottom(id).frame(Frame::NONE).show(ui,
  ..)` called in a side panel's `ui` pins a part to its bottom (the
  Generator's Track section); `show_inside` is deprecated. Call it before
  the scroll area that fills the rest. A scroll area's bar floats over
  its content: `ui.set_max_width(ui.available_width() - 12.0)` inside
  keeps buttons clear of it.
- Persistence: the `persistence` feature of eframe stores egui's memory
  (panel widths), the window geometry and whatever `App::save` writes
  with `eframe::set_value`. `persistence_path` names the file; it must be
  set, or the default `olio/app.ron` lands in the library folder on
  case-insensitive file systems.

- Widgets without an explicit id take one from their place among their
  siblings, so a widget that appears only sometimes (a button shown once
  settings turn Custom) changes the ids of every widget after it, and a
  focused slider below loses its focus. Wrap the part that varies in
  `ui.scope`, which takes one id whatever it holds (the Generator's Style
  block does).

## egui_kittest 0.36

- `Harness::builder().build_eframe(|cc| App::new(cc, ...))` runs the real
  app; `harness.state()` reaches it.
- Each `step()` advances input time by `step_dt`, 0.25 s by default: two
  clicks one step apart are not a double-click. The window tests use
  `with_step_dt(1.0 / 60.0)`.
- Queries match accessibility labels: give custom-painted widgets one with
  `response.widget_info(...)`, as tiles and set rows do. Calling it on a
  built-in widget's response replaces its label (the mute button reads
  "Mute", not "🔊").
- `harness.ctx.set_theme(egui::Theme::Light)` renders the light theme.
- A widget scrolled out of a `ScrollArea` is still in the tree but a
  click on it misses: call `node.scroll_to_me()`, step a few frames, and
  query it again before clicking (the Generator tests do).
- A `ComboBox` has no name of its own (its value is the selected text):
  name it with `combo.response.labelled_by(label.id)`, find it with
  `get_by_role_and_label(Role::ComboBox, "Style")`, click it, step, then
  click the entry by label in the popup (`pick_style` in the window
  tests). Sliders in a grid are named the same way.
- A popup's entry is only clicked if the pointer was over the popup the
  frame before: `hover()`, step, then `click()`. A long list scrolls
  inside the popup, so `scroll_to_me()` first. A `Modal` needs about ten
  frames to place itself before its buttons can be clicked.
- `.wgpu()` on the builder plus `harness.render()` gives an image; it needs
  a GPU adapter, so rendering tests are opt-in (`OLIO_RENDER=1`).

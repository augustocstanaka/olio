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
- Long grids: `ScrollArea::show_rows(ui, row_height, rows, |ui, range|)`
  only lays out the visible rows (the emoji picker's 1,900 emoji).
- Persistence: the `persistence` feature of eframe stores egui's memory
  (panel widths), the window geometry and whatever `App::save` writes
  with `eframe::set_value`. `persistence_path` names the file; it must be
  set, or the default `olio/app.ron` lands in the library folder on
  case-insensitive file systems.

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
- `.wgpu()` on the builder plus `harness.render()` gives an image; it needs
  a GPU adapter, so rendering tests are opt-in (`OLIO_RENDER=1`).

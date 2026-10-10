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
  font covers `🔁`, `🔊`, `■`, `▶`, `⏹`.

## egui_kittest 0.36

- `Harness::builder().build_eframe(|cc| App::new(cc, ...))` runs the real
  app; `harness.state()` reaches it.
- Each `step()` advances input time by `step_dt`, 0.25 s by default: two
  clicks one step apart are not a double-click. The window tests use
  `with_step_dt(1.0 / 60.0)`.
- Queries match accessibility labels: give custom-painted widgets one with
  `response.widget_info(...)`, as tiles and set rows do.
- `.wgpu()` on the builder plus `harness.render()` gives an image; it needs
  a GPU adapter, so rendering tests are opt-in (`OLIO_RENDER=1`).

//! The front set's grid of tiles.

use std::collections::HashMap;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, Color32, FontId, Id, Painter, Pos2, Rect, RichText, Sense, Shadow, Stroke, StrokeKind,
    Ui, Vec2, WidgetInfo, WidgetType,
};
use olio_control::{Command, TileKey, TileSetting};
use olio_library::{Set, SetId, Slot, Tile, TileId};

use crate::clips::{ClipState, Clips, Ready};
use crate::emoji::EmojiImages;
use crate::meter::{self, Meter};
use crate::prefs::{Polish, TileSize, TileStyle};
use crate::{format, palette};

const ROUNDING: u8 = 10;
/// Room on a tile's right for its level meter.
const METER_STRIP: f32 = 9.0;
/// Room around a Soft grid, so the shadows of the outer tiles show.
const SOFT_INSET: f32 = 10.0;
/// How long a tile dips when it starts playing, in seconds.
const BOUNCE: f64 = 0.16;
/// How far it dips: the share of its size it loses at the bottom.
const BOUNCE_DEPTH: f32 = 0.035;

/// Sizes on the grid for each tile size.
#[derive(Clone, Copy, Debug)]
struct Metrics {
    /// Between tiles.
    gap: f32,
    /// Tiles shrink to fit the window down to this; then the grid scrolls.
    min: Vec2,
    max_height: f32,
    /// Font sizes: the title, and the times and key.
    title: f32,
    small: f32,
    /// The emoji beside the title.
    emoji: f32,
    /// Room inside a tile's edge.
    padding: Vec2,
    /// Waveform, spectrogram and elapsed time; compact tiles have none.
    detail: bool,
}

impl Metrics {
    fn of(size: TileSize) -> Self {
        match size {
            TileSize::Compact => Self {
                gap: 6.0,
                min: Vec2::new(80.0, 48.0),
                max_height: 64.0,
                title: 13.0,
                small: 11.5,
                emoji: 16.0,
                padding: Vec2::new(7.0, 6.0),
                detail: false,
            },
            TileSize::Regular => Self {
                gap: 12.0,
                min: Vec2::new(120.0, 96.0),
                max_height: 170.0,
                title: 15.0,
                small: 12.5,
                emoji: 22.0,
                padding: Vec2::new(10.0, 8.0),
                detail: true,
            },
            TileSize::Large => Self {
                gap: 14.0,
                min: Vec2::new(170.0, 140.0),
                max_height: 260.0,
                title: 19.0,
                small: 14.5,
                emoji: 28.0,
                padding: Vec2::new(12.0, 10.0),
                detail: true,
            },
        }
    }
}

/// How far a playing tile has got.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    pub elapsed_seconds: f64,
    pub total_seconds: f64,
    /// 0 at the start of the clip, 1 at its end.
    pub fraction: f32,
}

pub struct GridView<'a> {
    pub set: &'a Set,
    pub clips: &'a Clips,
    pub playing: &'a HashMap<TileId, Progress>,
    pub selected: Option<TileId>,
    /// Scroll so the selected tile is in view.
    pub reveal_selected: bool,
    /// A single click plays a tile; otherwise it only selects it and a
    /// double-click plays it.
    pub click_plays: bool,
    /// The Polish preset, to label the context menu item.
    pub polish: Polish,
    pub style: TileStyle,
    pub size: TileSize,
    /// Draw a spectrogram under each waveform.
    pub spectrograms: bool,
    pub images: &'a EmojiImages,
    /// Level meters of tiles that are playing or have just stopped.
    pub levels: &'a HashMap<TileId, Meter>,
    /// The time the meters were fed, in seconds.
    pub now: f64,
    /// Files are being dragged over the window.
    pub files_hovering: bool,
    /// The tile being dragged, from any set, and the set it is in. From
    /// another set it was brought here by holding it over this set.
    pub dragged: Option<(&'a Tile, SetId)>,
}

#[derive(Default)]
pub struct GridOutput {
    pub commands: Vec<Command>,
    /// Tiles whose removal was asked for (the app confirms first).
    pub remove: Vec<TileId>,
    /// A tile whose title should be edited.
    pub rename: Option<TileId>,
    pub recolor: Vec<(TileId, olio_library::TileColor)>,
    /// Tiles to polish, or unpolish if they already are.
    pub polish: Vec<TileId>,
    /// A tile whose emoji should be chosen.
    pub choose_emoji: Option<TileId>,
    /// A tile to copy into the next free slot.
    pub duplicate: Option<TileId>,
    /// Tiles dropped on a slot by dragging.
    pub drops: Vec<Drop>,
    /// `Some` when the selection changed: the tile clicked, or `None` for
    /// a click on an empty slot.
    pub select: Option<Option<TileId>>,
    pub layout: Option<GridLayout>,
}

/// A tile dragged onto a slot: moved there (swapping with a tile already
/// there), or copied there when the copy modifier was held.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drop {
    pub tile: TileId,
    /// The set in front, where it was dropped.
    pub set: SetId,
    pub slot: Slot,
    pub copy: bool,
}

/// The modifier that makes a drag drop a copy: Option on macOS, Ctrl
/// elsewhere, as in each platform's file manager.
pub fn copy_modifier(modifiers: egui::Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.alt
    } else {
        modifiers.ctrl || modifiers.alt
    }
}

/// The release of the primary button, if it came up this frame: where,
/// and with which modifiers. Read from the event, since the pointer may
/// already be gone.
pub fn release(ui: &Ui) -> Option<(Pos2, egui::Modifiers)> {
    ui.input(|input| {
        input.events.iter().find_map(|event| match event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            } => Some((*pos, *modifiers)),
            _ => None,
        })
    })
}

/// Where the grab point sits inside a dragged tile, kept from the start of
/// the drag so the ghost does not jump.
fn grab_offset_id() -> Id {
    Id::new("olio-drag-offset")
}

/// Where the grid was drawn this frame, for finding the slot under a drop.
#[derive(Clone, Copy, Debug)]
pub struct GridLayout {
    origin: Pos2,
    cell: Vec2,
    gap: f32,
    columns: u8,
    rows: u8,
}

impl GridLayout {
    fn new(available: Vec2, columns: u8, rows: u8, metrics: &Metrics) -> Self {
        let (columns_f, rows_f) = (f32::from(columns), f32::from(rows));
        let gap = metrics.gap;
        let width = ((available.x - gap * (columns_f - 1.0)) / columns_f).max(metrics.min.x);
        let height = ((available.y - gap * (rows_f - 1.0)) / rows_f)
            .clamp(metrics.min.y, metrics.max_height);
        Self {
            origin: Pos2::ZERO,
            cell: Vec2::new(width, height),
            gap,
            columns,
            rows,
        }
    }

    fn size(&self) -> Vec2 {
        let (columns, rows) = (f32::from(self.columns), f32::from(self.rows));
        Vec2::new(
            columns * self.cell.x + (columns - 1.0) * self.gap,
            rows * self.cell.y + (rows - 1.0) * self.gap,
        )
    }

    pub fn rect(&self, slot: Slot) -> Rect {
        let offset = Vec2::new(
            f32::from(slot.column) * (self.cell.x + self.gap),
            f32::from(slot.row) * (self.cell.y + self.gap),
        );
        Rect::from_min_size(self.origin + offset, self.cell)
    }

    /// The slot under `pos`; the gaps between tiles belong to none.
    pub fn slot_at(&self, pos: Pos2) -> Option<Slot> {
        (0..self.rows)
            .flat_map(|row| (0..self.columns).map(move |column| Slot::new(row, column)))
            .find(|&slot| self.rect(slot).contains(pos))
    }
}

pub fn show(ui: &mut Ui, view: &GridView) -> GridOutput {
    let mut output = GridOutput::default();
    let columns = view.set.columns.max(1);
    let rows = view.set.visible_rows();
    let inset = match view.style {
        TileStyle::Flat => 0.0,
        TileStyle::Soft => SOFT_INSET,
    };
    let metrics = Metrics::of(view.size);
    let mut layout = GridLayout::new(
        ui.available_size() - Vec2::splat(2.0 * inset),
        columns,
        rows,
        &metrics,
    );
    egui::ScrollArea::both()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            let size = layout.size() + Vec2::splat(2.0 * inset);
            let (grid_rect, _) = ui.allocate_exact_size(size, Sense::hover());
            layout.origin = grid_rect.min + Vec2::splat(inset);
            let dark = ui.visuals().dark_mode;
            let pointer = ui.ctx().pointer_latest_pos();
            let release = release(ui);
            let copying = match release {
                Some((_, modifiers)) => copy_modifier(modifiers),
                None => ui.input(|input| copy_modifier(input.modifiers)),
            };
            let held_over = view
                .dragged
                .and(release.map(|(pos, _)| pos).or(pointer))
                .and_then(|pos| layout.slot_at(pos));
            // A tile of this set, moved without copying, swaps with any
            // tile where it is dropped. A copy, or a tile from another set,
            // lands without displacing one: there, or the next free slot.
            let own = view
                .dragged
                .filter(|&(_, set)| set == view.set.id)
                .map(|(tile, _)| tile);
            let swapping = own.is_some() && !copying;
            let target = if swapping {
                held_over
            } else {
                held_over.and_then(|slot| view.set.landing_slot(slot))
            };
            let dragging = own.map(|tile| tile.id);
            for row in 0..rows {
                for column in 0..columns {
                    let slot = Slot::new(row, column);
                    let rect = layout.rect(slot);
                    match view.set.tile_at(slot) {
                        Some(tile) => {
                            let lifted = Lifted {
                                lifted: dragging == Some(tile.id),
                                // Hovering another tile: it shows where it
                                // will go, the dragged tile's old slot.
                                swap_in: target
                                    .filter(|&target| swapping && target != tile.slot)
                                    .and_then(|target| view.set.tile_at(target)),
                            };
                            tile_ui(ui, rect, tile, view, dark, lifted, &mut output);
                        }
                        None => {
                            let targeted = pointer.is_some_and(|pos| rect.contains(pos));
                            let response =
                                ui.interact(rect, Id::new(("slot", slot)), Sense::click());
                            if let Some(key) = TileKey::for_slot(slot) {
                                response.widget_info(|| {
                                    WidgetInfo::labeled(
                                        WidgetType::Button,
                                        true,
                                        format!("Empty slot {}", key.label()),
                                    )
                                });
                            }
                            if response.clicked() {
                                output.select = Some(None);
                            }
                            empty_slot(ui, rect, slot, view, targeted);
                        }
                    }
                }
            }
            if let Some((tile, _)) = view.dragged {
                drag_feedback(ui, view, &layout, tile, target, pointer, copying);
                if let (Some(_), Some(slot)) = (release, target) {
                    let in_place = swapping && own.is_some_and(|own| own.slot == slot);
                    if !in_place {
                        output.drops.push(Drop {
                            tile: tile.id,
                            set: view.set.id,
                            slot,
                            copy: copying,
                        });
                    }
                }
            }
            if view.set.tiles.is_empty() && !view.files_hovering && view.dragged.is_none() {
                let add = ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::O,
                ));
                ui.painter().text(
                    grid_rect.center(),
                    Align2::CENTER_CENTER,
                    format!("Drop audio files or folders here, or press {add} to add sounds"),
                    FontId::proportional(16.0),
                    ui.visuals().weak_text_color(),
                );
            }
        });
    output.layout = Some(layout);
    output
}

/// While a tile is dragged: the target slot outlined (a tile there will
/// swap), the grabbing hand, and a ghost of the tile under the pointer.
fn drag_feedback(
    ui: &Ui,
    view: &GridView,
    layout: &GridLayout,
    tile: &Tile,
    target: Option<Slot>,
    pointer: Option<Pos2>,
    copying: bool,
) {
    let home = view.set.tile(tile.id).is_some();
    ui.ctx().set_cursor_icon(if copying {
        egui::CursorIcon::Copy
    } else {
        egui::CursorIcon::Grabbing
    });
    let dark = ui.visuals().dark_mode;
    if let Some(slot) = target.filter(|&slot| !(home && slot == tile.slot)) {
        let rect = layout.rect(slot);
        let visuals = ui.visuals();
        if view.set.is_free(slot) {
            ui.painter().rect_filled(
                rect,
                ROUNDING,
                visuals.selection.bg_fill.gamma_multiply(0.3),
            );
        }
        ui.painter().rect_stroke(
            rect.expand(2.0),
            ROUNDING + 2,
            Stroke::new(2.5, visuals.selection.stroke.color),
            StrokeKind::Outside,
        );
    }
    let Some(pointer) = pointer else {
        return;
    };
    if !ui.clip_rect().contains(pointer) {
        // Off the grid (over the sets, say): a small label beside the
        // pointer, so what is under it stays in view.
        drag_label(ui, view, tile, pointer);
        return;
    }
    let offset = ui
        .ctx()
        .data(|data| data.get_temp::<Vec2>(grab_offset_id()))
        .unwrap_or(layout.cell / 2.0);
    let ghost = Rect::from_min_size(pointer - offset, layout.cell);
    let layer = egui::LayerId::new(egui::Order::Tooltip, Id::new("olio-drag-ghost"));
    let mut painter = ui.ctx().layer_painter(layer);
    let shadow = Shadow {
        offset: [0, 8],
        blur: 22,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 140 } else { 70 }),
    };
    painter.add(shadow.as_shape(ghost, ROUNDING));
    painter.multiply_opacity(0.92);
    let look = Look {
        selected: false,
        hovered: true,
        dark,
        style: view.style,
        scale: 1.0,
        meter: None,
        spectrogram: view.spectrograms,
        metrics: Metrics::of(view.size),
    };
    let state = view.clips.get(&tile.sound.hash);
    let progress = view.playing.get(&tile.id).copied();
    // With the key it would have where it is held.
    let mut ghost_tile = tile.clone();
    if let Some(slot) = target {
        ghost_tile.slot = slot;
    }
    paint_tile(
        &painter,
        ghost,
        &ghost_tile,
        state,
        progress,
        look,
        view.images,
    );
}

/// How a tile is shown while it is being dragged.
#[derive(Clone, Copy)]
struct Lifted<'a> {
    lifted: bool,
    /// The tile it would swap with, previewed in its place.
    swap_in: Option<&'a Tile>,
}

/// The dragged tile as a small label: its emoji and title.
fn drag_label(ui: &Ui, view: &GridView, tile: &Tile, pointer: Pos2) {
    let dark = ui.visuals().dark_mode;
    let strong = palette::strong(tile.color, dark);
    let layer = egui::LayerId::new(egui::Order::Tooltip, Id::new("olio-drag-ghost"));
    let painter = ui.ctx().layer_painter(layer);
    let title = painter.layout_no_wrap(tile.title.clone(), FontId::proportional(14.0), strong);
    let emoji_room = if tile.emoji.is_empty() { 0.0 } else { 24.0 };
    let size = Vec2::new(title.size().x + emoji_room + 24.0, 30.0);
    let rect = Rect::from_min_size(pointer + Vec2::new(14.0, 12.0), size);
    let shadow = Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 140 } else { 60 }),
    };
    painter.add(shadow.as_shape(rect, 15));
    painter.rect_filled(rect, 15, palette::tile_background(dark));
    painter.rect(
        rect,
        15,
        palette::tint(tile.color, dark, 40),
        Stroke::new(1.5, strong),
        StrokeKind::Inside,
    );
    let mut left = rect.left() + 12.0;
    if !tile.emoji.is_empty() {
        let picture =
            Rect::from_center_size(Pos2::new(left + 9.0, rect.center().y), Vec2::splat(18.0));
        view.images.paint(&painter, picture, &tile.emoji, strong);
        left += emoji_room;
    }
    painter.galley(
        Pos2::new(left, rect.center().y - title.size().y / 2.0),
        title,
        strong,
    );
}

fn tile_ui(
    ui: &mut Ui,
    rect: Rect,
    tile: &Tile,
    view: &GridView,
    dark: bool,
    lifted: Lifted,
    output: &mut GridOutput,
) {
    let response = ui.interact(rect, Id::new(("tile", tile.id)), Sense::click_and_drag());
    response.dnd_set_drag_payload(tile.id);
    if response.drag_started()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let offset = pointer - rect.min;
        ui.ctx()
            .data_mut(|data| data.insert_temp(grab_offset_id(), offset));
    }
    let progress = view.playing.get(&tile.id).copied();
    let state = view.clips.get(&tile.sound.hash);
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::Button, true, &tile.title);
        info.selected = Some(progress.is_some());
        info
    });
    if view.reveal_selected && view.selected == Some(tile.id) {
        ui.scroll_to_rect(rect, None);
    }
    // With click-to-play, the click plays and the double-click's second
    // click must not stop it again.
    let plays = if view.click_plays {
        response.clicked() && !response.double_clicked()
    } else {
        response.double_clicked()
    };
    if plays {
        output.commands.push(Command::ToggleTile(tile.id));
    }
    if response.clicked() || response.secondary_clicked() {
        output.select = Some(Some(tile.id));
    }
    response.context_menu(|ui| {
        let label = if progress.is_some() { "Stop" } else { "Play" };
        if ui.button(label).clicked() {
            output.commands.push(Command::ToggleTile(tile.id));
            ui.close();
        }
        if ui.button("Rename…").clicked() {
            output.rename = Some(tile.id);
            ui.close();
        }
        if ui.button("Emoji…").clicked() {
            output.choose_emoji = Some(tile.id);
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            output.duplicate = Some(tile.id);
            ui.close();
        }
        let polish = if view.polish.applies_to(tile) {
            "Remove Polish"
        } else {
            "✨ Polish"
        };
        if ui.button(polish).clicked() {
            output.polish.push(tile.id);
            ui.close();
        }
        ui.separator();
        let mut looping = tile.looping;
        if ui.checkbox(&mut looping, "Loop").changed() {
            output
                .commands
                .push(Command::SetTile(tile.id, TileSetting::Looping(looping)));
        }
        let mut trim = tile.trim_silence;
        if ui.checkbox(&mut trim, "Trim Silence").changed() {
            output
                .commands
                .push(Command::SetTile(tile.id, TileSetting::TrimSilence(trim)));
        }
        let mut normalize = tile.normalize;
        if ui.checkbox(&mut normalize, "Normalize Level").changed() {
            output
                .commands
                .push(Command::SetTile(tile.id, TileSetting::Normalize(normalize)));
        }
        ui.menu_button("Color", |ui| {
            for color in olio_library::TileColor::ALL {
                let label = if color == tile.color {
                    format!("✔ {}", color.name())
                } else {
                    format!("    {}", color.name())
                };
                let swatch = RichText::new("●").color(palette::strong(color, dark));
                let button = egui::Button::new((swatch, label));
                if ui.add(button).clicked() {
                    output.recolor.push((tile.id, color));
                    ui.close();
                }
            }
        });
        ui.separator();
        if ui.button("Remove Tile").clicked() {
            output.remove.push(tile.id);
            ui.close();
        }
    });
    if let Some(ClipState::Failed(message)) = state {
        response.clone().on_hover_text(message);
    }
    let look = Look {
        selected: view.selected == Some(tile.id),
        hovered: response.hovered(),
        dark,
        style: view.style,
        scale: bounce(ui, tile.id, progress.is_some()),
        spectrogram: view.spectrograms,
        metrics: Metrics::of(view.size),
        meter: view
            .levels
            .get(&tile.id)
            .filter(|meter| !meter.is_resting(view.now))
            .copied(),
    };
    let mut painter = ui.painter().clone();
    if !lifted.lifted {
        paint_tile(&painter, rect, tile, state, progress, look, view.images);
        return;
    }
    match lifted.swap_in {
        Some(other) => {
            // The tile it would swap with, faintly, where it would land.
            painter.multiply_opacity(0.5);
            let state = view.clips.get(&other.sound.hash);
            let progress = view.playing.get(&other.id).copied();
            // With the key it would have there.
            let mut other = other.clone();
            other.slot = tile.slot;
            let other = &other;
            let look = Look {
                selected: false,
                hovered: false,
                meter: None,
                ..look
            };
            paint_tile(&painter, rect, other, state, progress, look, view.images);
        }
        None => {
            // Left behind faintly while its ghost follows the pointer.
            painter.multiply_opacity(0.3);
            paint_tile(&painter, rect, tile, state, progress, look, view.images);
        }
    }
}

/// How a tile is drawn this frame.
#[derive(Clone, Copy)]
struct Look {
    selected: bool,
    hovered: bool,
    dark: bool,
    style: TileStyle,
    /// 1 at rest; a little less while the tile dips as it starts.
    scale: f32,
    /// The tile's level, while there is one to show.
    meter: Option<Meter>,
    spectrogram: bool,
    metrics: Metrics,
}

/// The tile's size this frame: a short dip when it starts playing, like a
/// button pressed. Remembers in egui's memory when each tile started.
fn bounce(ui: &Ui, tile: TileId, playing: bool) -> f32 {
    let ctx = ui.ctx();
    let id = Id::new(("bounce", tile));
    let now = ctx.input(|input| input.time);
    let started = ctx.data_mut(|data| {
        let was: Option<(bool, f64)> = data.get_temp(id);
        let started = match was {
            Some((true, at)) if playing => at,
            _ if playing => now,
            _ => f64::NEG_INFINITY,
        };
        data.insert_temp(id, (playing, started));
        started
    });
    let t = now - started;
    if !(0.0..BOUNCE).contains(&t) {
        return 1.0;
    }
    ctx.request_repaint();
    1.0 - BOUNCE_DEPTH * (std::f64::consts::PI * t / BOUNCE).sin() as f32
}

fn paint_tile(
    painter: &Painter,
    rect: Rect,
    tile: &Tile,
    state: Option<&ClipState>,
    progress: Option<Progress>,
    look: Look,
    images: &EmojiImages,
) {
    let dark = look.dark;
    let rect = Rect::from_center_size(rect.center(), rect.size() * look.scale);
    let strong = palette::strong(tile.color, dark);
    let resting = match look.style {
        TileStyle::Flat => 90,
        TileStyle::Soft => 55,
    };
    let border = if look.selected {
        Stroke::new(3.0, strong)
    } else if look.hovered || progress.is_some() {
        Stroke::new(1.5, palette::tint(tile.color, dark, 190))
    } else {
        Stroke::new(1.0, palette::tint(tile.color, dark, resting))
    };
    match look.style {
        TileStyle::Flat => {
            painter.rect_filled(rect, ROUNDING, palette::tile_background(dark));
        }
        TileStyle::Soft => soft_background(painter, rect, progress.is_some(), dark),
    }
    let metrics = look.metrics;
    if let Some(progress) = progress {
        painter.rect_filled(rect, ROUNDING, palette::tint(tile.color, dark, 28));
        if !metrics.detail {
            // No waveform to show progress: the tile fills as it plays.
            let played = Rect::from_min_size(
                rect.min,
                Vec2::new(
                    rect.width() * progress.fraction.clamp(0.0, 1.0),
                    rect.height(),
                ),
            );
            painter.rect_filled(played, ROUNDING, palette::tint(tile.color, dark, 45));
        }
    }
    painter.rect_stroke(rect, ROUNDING, border, StrokeKind::Inside);
    let inner = rect.shrink2(metrics.padding);

    // The emoji, the title beside it, and the loop mark on the right.
    let loop_room = if tile.looping { 22.0 } else { 0.0 };
    let emoji_room = if tile.emoji.is_empty() {
        0.0
    } else {
        metrics.emoji + 5.0
    };
    let mut title = LayoutJob::simple_singleline(
        tile.title.clone(),
        FontId::proportional(metrics.title),
        strong,
    );
    title.wrap =
        TextWrapping::truncate_at_width((inner.width() - loop_room - emoji_room).max(10.0));
    let title = painter.layout_job(title);
    // The emoji is centred on the title's line, so titles and waveforms
    // line up across tiles with and without one.
    let title_bottom = inner.top() + title.size().y;
    if !tile.emoji.is_empty() {
        let center = Pos2::new(
            inner.left() + metrics.emoji / 2.0,
            inner.top() + title.size().y / 2.0,
        );
        let picture = Rect::from_center_size(center, Vec2::splat(metrics.emoji));
        images.paint(painter, picture, &tile.emoji, strong);
    }
    painter.galley(
        Pos2::new(inner.left() + emoji_room, inner.top()),
        title,
        strong,
    );
    if tile.looping {
        painter.text(
            inner.right_top(),
            Align2::RIGHT_TOP,
            "🔁",
            FontId::proportional(metrics.small + 0.5),
            strong,
        );
    }
    if !metrics.detail {
        compact_footer(painter, inner, rect, tile, state, progress, look);
        return;
    }

    // Along the bottom: the key in the middle; while playing, the time
    // elapsed on the left and the time left on the right; otherwise just
    // the duration on the right.
    let times = FontId::proportional(metrics.small);
    match progress {
        Some(progress) => {
            painter.text(
                inner.left_bottom(),
                Align2::LEFT_BOTTOM,
                format::clock(progress.elapsed_seconds),
                times.clone(),
                strong,
            );
            let left = (progress.total_seconds - progress.elapsed_seconds).max(0.0);
            painter.text(
                inner.right_bottom(),
                Align2::RIGHT_BOTTOM,
                format!("-{}", format::clock(left)),
                times,
                strong,
            );
        }
        None => {
            if let Some(ClipState::Ready(ready)) = state {
                let region = ready.region(tile.trim_silence);
                let seconds = region.len() as f64 / f64::from(ready.clip.rate());
                painter.text(
                    inner.right_bottom(),
                    Align2::RIGHT_BOTTOM,
                    format::clock(seconds),
                    times,
                    palette::tint(tile.color, dark, 200),
                );
            }
        }
    }
    if let Some(key) = TileKey::for_slot(tile.slot) {
        key_badge(painter, inner.center_bottom(), &key.label(), tile, look);
    }

    // The waveform fills the space between, leaving a strip on the right
    // for the level meter, so the waveform keeps its width when playing.
    let badge = metrics.small + 8.0;
    let wave = Rect::from_min_max(
        Pos2::new(rect.left() + 4.0, title_bottom + 6.0),
        Pos2::new(
            rect.right() - 4.0 - METER_STRIP,
            inner.bottom() - badge - 6.0,
        ),
    );
    if wave.height() < 6.0 {
        return;
    }
    if let Some(meter) = &look.meter {
        let bar = Rect::from_x_y_ranges(
            wave.right() + 4.0..=wave.right() + METER_STRIP - 2.0,
            wave.y_range(),
        );
        let color = palette::tint(tile.color, dark, 210);
        meter::paint_bar(painter, bar, meter, meter::Colors::Tinted(color), dark);
    }
    let note = |text: &str, color: Color32| {
        painter.text(
            wave.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(12.0),
            color,
        );
    };
    match state {
        Some(ClipState::Ready(ready)) => {
            let played = progress.map(|progress| progress.fraction);
            // The spectrogram takes the bottom of the space when there is
            // room for both.
            let strip_height = (wave.height() * 0.3).max(10.0);
            if look.spectrogram && wave.height() - strip_height >= 20.0 {
                let (top, strip) = wave.split_top_bottom_at_y(wave.bottom() - strip_height);
                let top = top.with_max_y(top.max.y - 3.0);
                waveform(painter, top, ready, played, tile, dark);
                spectrogram(painter, strip, ready, played, tile, dark);
            } else {
                waveform(painter, wave, ready, played, tile, dark);
            }
        }
        Some(ClipState::Failed(_)) => note("Can't play this sound", Color32::from_rgb(220, 60, 60)),
        Some(ClipState::Loading) | None => note("Loading…", palette::tint(tile.color, dark, 150)),
    }
}

/// The sound's spectrogram over the part that plays, glowing in the tile's
/// colour: low frequencies at the bottom.
pub fn spectrogram(
    painter: &Painter,
    rect: Rect,
    ready: &Ready,
    played: Option<f32>,
    tile: &Tile,
    dark: bool,
) {
    let frames = ready.clip.frames().max(1) as f32;
    let region = ready.region(tile.trim_silence);
    let uv = Rect::from_x_y_ranges(
        region.start as f32 / frames..=region.end as f32 / frames,
        0.0..=1.0,
    );
    painter.rect_filled(rect, 3, palette::tint(tile.color, dark, 14));
    let texture = ready.spectrogram_texture(painter.ctx());
    let glow = palette::tint(tile.color, dark, if dark { 235 } else { 255 });
    painter.image(texture.id(), rect, uv, glow);
    if let Some(played) = played {
        let x = rect.left() + played.clamp(0.0, 1.0) * rect.width();
        painter.vline(
            x,
            rect.y_range(),
            Stroke::new(1.5, palette::strong(tile.color, dark)),
        );
    }
}

fn waveform(
    painter: &Painter,
    rect: Rect,
    ready: &Ready,
    played: Option<f32>,
    tile: &Tile,
    dark: bool,
) {
    let frames = ready.clip.frames().max(1) as f64;
    let region = ready.region(tile.trim_silence);
    let (from, to) = (region.start as f64 / frames, region.end as f64 / frames);
    let seconds = region.len() as f32 / ready.clip.rate() as f32;
    let columns = (rect.width() / 3.0).floor().max(1.0) as usize;
    let step = rect.width() / columns as f32;
    let scale = if ready.loudest > 1e-6 {
        1.0 / ready.loudest
    } else {
        1.0
    };
    let half = rect.height() / 2.0;
    let middle = rect.center().y;
    let unplayed = palette::tint(tile.color, dark, if dark { 95 } else { 110 });
    let played_color = palette::tint(tile.color, dark, 235);
    let peaks = ready.peaks.columns_between(columns, from, to);
    for (column, (low, high)) in peaks.into_iter().enumerate() {
        let reached = (column as f32 + 0.5) / columns as f32;
        // Draw the fades as the shape they give the sound.
        let at = reached * seconds;
        let mut envelope = 1.0f32;
        if tile.fade_in_seconds > 0.0 {
            envelope *= (at / tile.fade_in_seconds).min(1.0);
        }
        if tile.fade_out_seconds > 0.0 && !tile.looping {
            envelope *= ((seconds - at) / tile.fade_out_seconds).clamp(0.0, 1.0);
        }
        let x = rect.left() + (column as f32 + 0.5) * step;
        let reach = |sample: f32| (sample * scale * envelope).clamp(-1.0, 1.0) * half;
        let top = (middle - reach(high)).min(middle - 0.5);
        let bottom = (middle - reach(low)).max(middle + 0.5);
        let color = if played.is_some_and(|played| reached <= played) {
            played_color
        } else {
            unplayed
        };
        painter.vline(x, top..=bottom, Stroke::new(step * 0.6, color));
    }
    if let Some(played) = played {
        let x = rect.left() + played.clamp(0.0, 1.0) * rect.width();
        painter.vline(
            x,
            rect.y_range(),
            Stroke::new(1.5, palette::strong(tile.color, dark)),
        );
    }
}

/// A raised tile on the Soft canvas, or a pressed one: dark at the rim
/// where the inner shadow falls, lit towards the bottom right.
fn soft_background(painter: &Painter, rect: Rect, pressed: bool, dark: bool) {
    let soft = palette::soft(dark);
    if pressed {
        painter.rect_filled(rect, ROUNDING, soft.pressed_rim);
        let lit = Shadow {
            offset: [2, 2],
            blur: 10,
            spread: 0,
            color: soft.pressed_fill,
        };
        painter
            .with_clip_rect(rect)
            .add(lit.as_shape(rect.shrink(6.0), ROUNDING - 4));
    } else {
        painter.add(soft.light_shadow.as_shape(rect, ROUNDING));
        painter.add(soft.dark_shadow.as_shape(rect, ROUNDING));
        painter.rect_filled(rect, ROUNDING, soft.raised);
    }
}

/// The bottom of a compact tile: the key on the left, the time on the
/// right (the duration, or while playing the time left), and the level as
/// a thin bar along the bottom edge.
fn compact_footer(
    painter: &Painter,
    inner: Rect,
    rect: Rect,
    tile: &Tile,
    state: Option<&ClipState>,
    progress: Option<Progress>,
    look: Look,
) {
    let dark = look.dark;
    let strong = palette::strong(tile.color, dark);
    let metrics = look.metrics;
    if let Some(key) = TileKey::for_slot(tile.slot) {
        let size = key_badge_size(painter, &key.label(), metrics);
        let at = Pos2::new(inner.left() + size.x / 2.0, inner.bottom());
        key_badge(painter, at, &key.label(), tile, look);
    }
    let times = FontId::proportional(metrics.small);
    let (text, color) = match (progress, state) {
        (Some(progress), _) => {
            let left = (progress.total_seconds - progress.elapsed_seconds).max(0.0);
            (format!("-{}", format::clock(left)), strong)
        }
        (None, Some(ClipState::Ready(ready))) => {
            let region = ready.region(tile.trim_silence);
            let seconds = region.len() as f64 / f64::from(ready.clip.rate());
            (format::clock(seconds), palette::tint(tile.color, dark, 200))
        }
        (None, Some(ClipState::Failed(_))) => {
            ("can't play".to_owned(), Color32::from_rgb(220, 60, 60))
        }
        (None, _) => ("…".to_owned(), palette::tint(tile.color, dark, 150)),
    };
    painter.text(
        inner.right_bottom(),
        Align2::RIGHT_BOTTOM,
        text,
        times,
        color,
    );
    if let Some(meter) = &look.meter {
        let bar = Rect::from_min_max(
            Pos2::new(rect.left() + 8.0, rect.bottom() - 4.0),
            Pos2::new(rect.right() - 8.0, rect.bottom() - 2.0),
        );
        let color = palette::tint(tile.color, dark, 210);
        meter::paint_bar(painter, bar, meter, meter::Colors::Tinted(color), dark);
    }
}

fn key_badge_size(painter: &Painter, label: &str, metrics: Metrics) -> Vec2 {
    let font = FontId::proportional(metrics.small + 0.5);
    let width = painter
        .layout_no_wrap(label.to_owned(), font, Color32::WHITE)
        .size()
        .x;
    let height = metrics.small + 8.0;
    Vec2::new((width + height).max(height * 1.9), height)
}

fn key_badge(painter: &Painter, bottom_center: Pos2, label: &str, tile: &Tile, look: Look) {
    let dark = look.dark;
    let strong = palette::strong(tile.color, dark);
    let metrics = look.metrics;
    let galley = painter.layout_no_wrap(
        label.to_owned(),
        FontId::proportional(metrics.small + 0.5),
        strong,
    );
    let size = key_badge_size(painter, label, metrics);
    let rect = Rect::from_center_size(bottom_center - Vec2::new(0.0, size.y / 2.0 - 1.0), size);
    let round = (size.y / 2.0) as u8;
    if look.style == TileStyle::Soft {
        // A small well the key sits in.
        let soft = palette::soft(dark);
        painter.rect_filled(rect, round, soft.pressed_rim);
        let lit = Shadow {
            offset: [1, 1],
            blur: 4,
            spread: 0,
            color: soft.pressed_fill,
        };
        painter.add(lit.as_shape(rect.shrink(2.0), round.saturating_sub(2)));
    }
    painter.rect_filled(rect, round, palette::tint(tile.color, dark, 45));
    painter.galley(rect.center() - galley.size() / 2.0, galley, strong);
}

fn empty_slot(ui: &Ui, rect: Rect, slot: Slot, view: &GridView, targeted: bool) {
    let files_hovering = view.files_hovering;
    let visuals = ui.visuals();
    let weak = visuals.weak_text_color();
    if view.style == TileStyle::Soft && !files_hovering {
        ui.painter()
            .rect_filled(rect, ROUNDING, palette::soft(visuals.dark_mode).well);
    }
    let (fill, stroke) = match (files_hovering, targeted) {
        (true, true) => (
            visuals.selection.bg_fill.gamma_multiply(0.35),
            Stroke::new(2.0, visuals.selection.stroke.color),
        ),
        (true, false) => (Color32::TRANSPARENT, Stroke::new(1.0, weak)),
        (false, _) => (
            Color32::TRANSPARENT,
            Stroke::new(1.0, weak.gamma_multiply(0.3)),
        ),
    };
    let painter = ui.painter();
    painter.rect(rect, ROUNDING, fill, stroke, StrokeKind::Inside);
    if let Some(key) = TileKey::for_slot(slot) {
        let title = Metrics::of(view.size).title;
        let (size, color) = if files_hovering {
            (title + 3.0, weak)
        } else {
            (title - 1.0, weak.gamma_multiply(0.5))
        };
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            key.label(),
            FontId::proportional(size),
            color,
        );
    }
}

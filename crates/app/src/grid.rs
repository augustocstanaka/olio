//! The front set's grid of tiles.

use std::collections::HashMap;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, Color32, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2,
    WidgetInfo, WidgetType,
};
use olio_control::{Command, TileKey, TileSetting};
use olio_library::{Set, Slot, Tile, TileId};

use crate::clips::{ClipState, Clips, Ready};
use crate::{format, palette};

const GAP: f32 = 12.0;
const MIN_TILE: Vec2 = Vec2::new(120.0, 96.0);
const MAX_TILE_HEIGHT: f32 = 170.0;
const ROUNDING: u8 = 10;

/// How far a playing tile has got.
#[derive(Clone, Copy, Debug)]
pub struct Progress {
    pub elapsed_seconds: f64,
    /// 0 at the start of the clip, 1 at its end.
    pub fraction: f32,
}

pub struct GridView<'a> {
    pub set: &'a Set,
    pub clips: &'a Clips,
    pub playing: &'a HashMap<TileId, Progress>,
    pub selected: Option<TileId>,
    /// Files are being dragged over the window.
    pub files_hovering: bool,
}

#[derive(Default)]
pub struct GridOutput {
    pub commands: Vec<Command>,
    pub remove: Vec<TileId>,
    /// `Some` when the selection changed: the tile clicked, or `None` for
    /// a click on an empty slot.
    pub select: Option<Option<TileId>>,
    pub layout: Option<GridLayout>,
}

/// Where the grid was drawn this frame, for finding the slot under a drop.
#[derive(Clone, Copy, Debug)]
pub struct GridLayout {
    origin: Pos2,
    cell: Vec2,
    columns: u8,
    rows: u8,
}

impl GridLayout {
    fn new(available: Vec2, columns: u8, rows: u8) -> Self {
        let (columns_f, rows_f) = (f32::from(columns), f32::from(rows));
        let width = ((available.x - GAP * (columns_f - 1.0)) / columns_f).max(MIN_TILE.x);
        let height =
            ((available.y - GAP * (rows_f - 1.0)) / rows_f).clamp(MIN_TILE.y, MAX_TILE_HEIGHT);
        Self {
            origin: Pos2::ZERO,
            cell: Vec2::new(width, height),
            columns,
            rows,
        }
    }

    fn size(&self) -> Vec2 {
        let (columns, rows) = (f32::from(self.columns), f32::from(self.rows));
        Vec2::new(
            columns * self.cell.x + (columns - 1.0) * GAP,
            rows * self.cell.y + (rows - 1.0) * GAP,
        )
    }

    pub fn rect(&self, slot: Slot) -> Rect {
        let offset = Vec2::new(
            f32::from(slot.column) * (self.cell.x + GAP),
            f32::from(slot.row) * (self.cell.y + GAP),
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
    let mut layout = GridLayout::new(ui.available_size(), columns, rows);
    egui::ScrollArea::both()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            let (grid_rect, _) = ui.allocate_exact_size(layout.size(), Sense::hover());
            layout.origin = grid_rect.min;
            let dark = ui.visuals().dark_mode;
            let pointer = ui.ctx().pointer_latest_pos();
            for row in 0..rows {
                for column in 0..columns {
                    let slot = Slot::new(row, column);
                    let rect = layout.rect(slot);
                    match view.set.tile_at(slot) {
                        Some(tile) => tile_ui(ui, rect, tile, view, dark, &mut output),
                        None => {
                            let targeted = pointer.is_some_and(|pos| rect.contains(pos));
                            let response =
                                ui.interact(rect, Id::new(("slot", slot)), Sense::click());
                            if response.clicked() {
                                output.select = Some(None);
                            }
                            empty_slot(ui, rect, slot, view.files_hovering, targeted);
                        }
                    }
                }
            }
            if view.set.tiles.is_empty() && !view.files_hovering {
                ui.painter().text(
                    grid_rect.center(),
                    Align2::CENTER_CENTER,
                    "Drop audio files here, or use Add Sound",
                    FontId::proportional(16.0),
                    ui.visuals().weak_text_color(),
                );
            }
        });
    output.layout = Some(layout);
    output
}

fn tile_ui(
    ui: &mut Ui,
    rect: Rect,
    tile: &Tile,
    view: &GridView,
    dark: bool,
    output: &mut GridOutput,
) {
    let response = ui.interact(rect, Id::new(("tile", tile.id)), Sense::click());
    let progress = view.playing.get(&tile.id).copied();
    let state = view.clips.get(&tile.sound.hash);
    response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::Button, true, &tile.title);
        info.selected = Some(progress.is_some());
        info
    });
    if response.double_clicked() {
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
    };
    paint_tile(ui.painter(), rect, tile, state, progress, look);
}

/// How a tile is drawn this frame.
#[derive(Clone, Copy)]
struct Look {
    selected: bool,
    hovered: bool,
    dark: bool,
}

fn paint_tile(
    painter: &Painter,
    rect: Rect,
    tile: &Tile,
    state: Option<&ClipState>,
    progress: Option<Progress>,
    look: Look,
) {
    let dark = look.dark;
    let strong = palette::strong(tile.color, dark);
    let border = if look.selected {
        Stroke::new(3.0, strong)
    } else if look.hovered || progress.is_some() {
        Stroke::new(1.5, palette::tint(tile.color, dark, 190))
    } else {
        Stroke::new(1.0, palette::tint(tile.color, dark, 90))
    };
    painter.rect_filled(rect, ROUNDING, palette::tile_background(dark));
    if progress.is_some() {
        painter.rect_filled(rect, ROUNDING, palette::tint(tile.color, dark, 28));
    }
    painter.rect_stroke(rect, ROUNDING, border, StrokeKind::Inside);
    let inner = rect.shrink2(Vec2::new(10.0, 8.0));

    // Title, and the loop mark beside it.
    let loop_room = if tile.looping { 22.0 } else { 0.0 };
    let mut title =
        LayoutJob::simple_singleline(tile.title.clone(), FontId::proportional(15.0), strong);
    title.wrap = TextWrapping::truncate_at_width((inner.width() - loop_room).max(10.0));
    let title = painter.layout_job(title);
    let title_bottom = inner.top() + title.size().y;
    painter.galley(inner.left_top(), title, strong);
    if tile.looping {
        painter.text(
            inner.right_top(),
            Align2::RIGHT_TOP,
            "🔁",
            FontId::proportional(13.0),
            strong,
        );
    }

    // Elapsed time, key and duration along the bottom.
    let times = FontId::proportional(12.5);
    let elapsed = progress.map_or(0.0, |progress| progress.elapsed_seconds);
    painter.text(
        inner.left_bottom(),
        Align2::LEFT_BOTTOM,
        format::clock(elapsed),
        times.clone(),
        strong,
    );
    if let Some(ClipState::Ready(ready)) = state {
        let region = ready.region(tile.trim_silence);
        let seconds = region.len() as f64 / f64::from(ready.clip.rate());
        painter.text(
            inner.right_bottom(),
            Align2::RIGHT_BOTTOM,
            format::clock(seconds),
            times,
            strong,
        );
    }
    if let Some(key) = TileKey::for_slot(tile.slot) {
        key_badge(painter, inner.center_bottom(), &key.label(), tile, dark);
    }

    // The waveform fills the space between.
    let wave = Rect::from_min_max(
        Pos2::new(rect.left() + 4.0, title_bottom + 6.0),
        Pos2::new(rect.right() - 4.0, inner.bottom() - 26.0),
    );
    if wave.height() < 6.0 {
        return;
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
        Some(ClipState::Ready(ready)) => waveform(
            painter,
            wave,
            ready,
            progress.map(|progress| progress.fraction),
            tile,
            dark,
        ),
        Some(ClipState::Failed(_)) => note("Can't play this sound", Color32::from_rgb(220, 60, 60)),
        Some(ClipState::Loading) | None => note("Loading…", palette::tint(tile.color, dark, 150)),
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
    let unplayed = palette::tint(tile.color, dark, if dark { 95 } else { 75 });
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

fn key_badge(painter: &Painter, bottom_center: Pos2, label: &str, tile: &Tile, dark: bool) {
    let strong = palette::strong(tile.color, dark);
    let galley = painter.layout_no_wrap(label.to_owned(), FontId::proportional(13.0), strong);
    let size = Vec2::new((galley.size().x + 22.0).max(38.0), 20.0);
    let rect = Rect::from_center_size(bottom_center - Vec2::new(0.0, size.y / 2.0 - 1.0), size);
    painter.rect_filled(rect, 10, palette::tint(tile.color, dark, 45));
    painter.galley(rect.center() - galley.size() / 2.0, galley, strong);
}

fn empty_slot(ui: &Ui, rect: Rect, slot: Slot, files_hovering: bool, targeted: bool) {
    let visuals = ui.visuals();
    let weak = visuals.weak_text_color();
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
        let (size, color) = if files_hovering {
            (18.0, weak)
        } else {
            (14.0, weak.gamma_multiply(0.5))
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

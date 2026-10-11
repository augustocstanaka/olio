//! The front set's grid of tiles.

use std::collections::HashMap;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, Color32, FontId, Id, Painter, Pos2, Rect, RichText, Sense, Shadow, Stroke, StrokeKind,
    Ui, Vec2, WidgetInfo, WidgetType,
};
use olio_control::{Command, TileKey, TileSetting};
use olio_library::{Set, Slot, Tile, TileId};

use crate::clips::{ClipState, Clips, Ready};
use crate::emoji::EmojiImages;
use crate::meter::{self, Meter};
use crate::prefs::{Polish, TileStyle};
use crate::{format, palette};

const GAP: f32 = 12.0;
const MIN_TILE: Vec2 = Vec2::new(120.0, 96.0);
const MAX_TILE_HEIGHT: f32 = 170.0;
const ROUNDING: u8 = 10;
/// The emoji beside a tile's title.
const EMOJI_SIZE: f32 = 22.0;
/// Room on a tile's right for its level meter.
const METER_STRIP: f32 = 9.0;
/// Room around a Soft grid, so the shadows of the outer tiles show.
const SOFT_INSET: f32 = 10.0;
/// How long a tile dips when it starts playing, in seconds.
const BOUNCE: f64 = 0.16;
/// How far it dips: the share of its size it loses at the bottom.
const BOUNCE_DEPTH: f32 = 0.035;

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
    /// Draw a spectrogram under each waveform.
    pub spectrograms: bool,
    pub images: &'a EmojiImages,
    /// Level meters of tiles that are playing or have just stopped.
    pub levels: &'a HashMap<TileId, Meter>,
    /// The time the meters were fed, in seconds.
    pub now: f64,
    /// Files are being dragged over the window.
    pub files_hovering: bool,
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
    let inset = match view.style {
        TileStyle::Flat => 0.0,
        TileStyle::Soft => SOFT_INSET,
    };
    let mut layout = GridLayout::new(
        ui.available_size() - Vec2::splat(2.0 * inset),
        columns,
        rows,
    );
    egui::ScrollArea::both()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            let size = layout.size() + Vec2::splat(2.0 * inset);
            let (grid_rect, _) = ui.allocate_exact_size(size, Sense::hover());
            layout.origin = grid_rect.min + Vec2::splat(inset);
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
                            empty_slot(ui, rect, slot, view, targeted);
                        }
                    }
                }
            }
            if view.set.tiles.is_empty() && !view.files_hovering {
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
        meter: view
            .levels
            .get(&tile.id)
            .filter(|meter| !meter.is_resting(view.now))
            .copied(),
    };
    paint_tile(ui.painter(), rect, tile, state, progress, look, view.images);
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
    if progress.is_some() {
        painter.rect_filled(rect, ROUNDING, palette::tint(tile.color, dark, 28));
    }
    painter.rect_stroke(rect, ROUNDING, border, StrokeKind::Inside);
    let inner = rect.shrink2(Vec2::new(10.0, 8.0));

    // The emoji, the title beside it, and the loop mark on the right.
    let loop_room = if tile.looping { 22.0 } else { 0.0 };
    let emoji_room = if tile.emoji.is_empty() {
        0.0
    } else {
        EMOJI_SIZE + 6.0
    };
    let mut title =
        LayoutJob::simple_singleline(tile.title.clone(), FontId::proportional(15.0), strong);
    title.wrap =
        TextWrapping::truncate_at_width((inner.width() - loop_room - emoji_room).max(10.0));
    let title = painter.layout_job(title);
    // The emoji is centred on the title's line, so titles and waveforms
    // line up across tiles with and without one.
    let title_bottom = inner.top() + title.size().y;
    if !tile.emoji.is_empty() {
        let center = Pos2::new(
            inner.left() + EMOJI_SIZE / 2.0,
            inner.top() + title.size().y / 2.0,
        );
        let picture = Rect::from_center_size(center, Vec2::splat(EMOJI_SIZE));
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
            FontId::proportional(13.0),
            strong,
        );
    }

    // Along the bottom: the key in the middle; while playing, the time
    // elapsed on the left and the time left on the right; otherwise just
    // the duration on the right.
    let times = FontId::proportional(12.5);
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
    let wave = Rect::from_min_max(
        Pos2::new(rect.left() + 4.0, title_bottom + 6.0),
        Pos2::new(rect.right() - 4.0 - METER_STRIP, inner.bottom() - 26.0),
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

fn key_badge(painter: &Painter, bottom_center: Pos2, label: &str, tile: &Tile, look: Look) {
    let dark = look.dark;
    let strong = palette::strong(tile.color, dark);
    let galley = painter.layout_no_wrap(label.to_owned(), FontId::proportional(13.0), strong);
    let size = Vec2::new((galley.size().x + 22.0).max(38.0), 20.0);
    let rect = Rect::from_center_size(bottom_center - Vec2::new(0.0, size.y / 2.0 - 1.0), size);
    if look.style == TileStyle::Soft {
        // A small well the key sits in.
        let soft = palette::soft(dark);
        painter.rect_filled(rect, 10, soft.pressed_rim);
        let lit = Shadow {
            offset: [1, 1],
            blur: 4,
            spread: 0,
            color: soft.pressed_fill,
        };
        painter.add(lit.as_shape(rect.shrink(2.0), 8));
    }
    painter.rect_filled(rect, 10, palette::tint(tile.color, dark, 45));
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

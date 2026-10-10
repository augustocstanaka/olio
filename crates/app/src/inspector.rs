//! The panel on the right: the selected tile's sound, playback, fades and
//! reverb.

use egui::{Align, Layout, RichText, Sense, Stroke, Ui, Vec2};
use olio_control::{Command, TileKey, TileSetting};
use olio_library::{MAX_FADE_SECONDS, Set, Tile, TileColor, TileId};

use crate::clips::ClipState;
use crate::{format, palette};

pub enum InspectorAction {
    Command(Command),
    Rename(TileId, String),
    Recolor(TileId, TileColor),
    Remove(TileId),
}

pub fn show(
    ui: &mut Ui,
    set: &Set,
    tile: Option<&Tile>,
    state: Option<&ClipState>,
    playing: bool,
    title_draft: &mut Option<(TileId, String)>,
) -> Vec<InspectorAction> {
    let mut actions = Vec::new();
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.add_space(8.0);
            match tile {
                Some(tile) => {
                    title_field(ui, tile, title_draft, &mut actions);
                    tile_settings(ui, tile, state, playing, &mut actions);
                }
                None => set_summary(ui, set),
            }
        });
    actions
}

/// The tile's title, edited through a draft so that what is typed (such as
/// a trailing space before the next word) survives each frame. The title
/// changes as you type; it is trimmed, or restored if left empty, when the
/// field loses focus.
fn title_field(
    ui: &mut Ui,
    tile: &Tile,
    draft: &mut Option<(TileId, String)>,
    actions: &mut Vec<InspectorAction>,
) {
    let id = ui.make_persistent_id("tile-title");
    let editing = ui.memory(|memory| memory.has_focus(id));
    let stale = draft.as_ref().is_none_or(|(draft_tile, text)| {
        *draft_tile != tile.id || (!editing && *text != tile.title)
    });
    if stale {
        *draft = Some((tile.id, tile.title.clone()));
    }
    let Some((_, text)) = draft.as_mut() else {
        return;
    };
    let field = ui.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .font(egui::TextStyle::Heading)
            .desired_width(f32::INFINITY),
    );
    if field.changed() && !text.trim().is_empty() {
        actions.push(InspectorAction::Rename(tile.id, text.clone()));
    }
    if field.lost_focus() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            *text = tile.title.clone();
        } else if trimmed != text {
            *text = trimmed.to_owned();
            actions.push(InspectorAction::Rename(tile.id, text.clone()));
        }
    }
}

fn set_summary(ui: &mut Ui, set: &Set) {
    ui.label(RichText::new(&set.name).strong().size(16.0));
    ui.add_space(4.0);
    ui.weak(match set.tiles.len() {
        1 => "1 tile".to_owned(),
        count => format!("{count} tiles"),
    });
    ui.add_space(12.0);
    ui.label("Select a tile to change its fades, reverb and how it plays.");
    ui.add_space(6.0);
    ui.weak("Double-click a tile or press its key to play it. Hold Esc to stop everything.");
}

fn tile_settings(
    ui: &mut Ui,
    tile: &Tile,
    state: Option<&ClipState>,
    playing: bool,
    actions: &mut Vec<InspectorAction>,
) {
    let set = |setting| InspectorAction::Command(Command::SetTile(tile.id, setting));

    // Colour.
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        for color in TileColor::ALL {
            if swatch(ui, color, color == tile.color).clicked() {
                actions.push(InspectorAction::Recolor(tile.id, color));
            }
        }
    });

    // What it is.
    ui.add_space(8.0);
    let key = TileKey::for_slot(tile.slot).map_or_else(String::new, |key| key.label());
    let details = match state {
        Some(ClipState::Ready(ready)) => {
            let rate = f64::from(ready.clip.rate());
            let full = ready.clip.frames() as f64 / rate;
            let region = ready.region(tile.trim_silence);
            let trimmed = full - region.len() as f64 / rate;
            let mut line = format!("Key {key} · {}", format::clock(region.len() as f64 / rate));
            if tile.trim_silence && trimmed >= 0.05 {
                line.push_str(&format!(" · {:.1} s of silence trimmed", trimmed));
            }
            line
        }
        Some(ClipState::Failed(message)) => format!("Key {key} · can't play: {message}"),
        Some(ClipState::Loading) | None => format!("Key {key} · loading…"),
    };
    ui.weak(details);
    ui.weak(&tile.sound.original_name)
        .on_hover_text("The file this sound was imported from");

    ui.add_space(8.0);
    let label = if playing { "■ Stop" } else { "▶ Play" };
    if ui
        .add_sized([ui.available_width(), 28.0], egui::Button::new(label))
        .clicked()
    {
        actions.push(InspectorAction::Command(Command::ToggleTile(tile.id)));
    }

    section(ui, "Playback");
    let mut looping = tile.looping;
    if ui
        .checkbox(&mut looping, "Loop")
        .on_hover_text("Play again from the start until stopped")
        .changed()
    {
        actions.push(set(TileSetting::Looping(looping)));
    }
    let mut trim = tile.trim_silence;
    if ui
        .checkbox(&mut trim, "Trim silence")
        .on_hover_text("Skip silence at the start and end of the sound")
        .changed()
    {
        actions.push(set(TileSetting::TrimSilence(trim)));
    }

    section(ui, "Fades");
    egui::Grid::new("olio-fades")
        .num_columns(2)
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            ui.label("Fade in");
            let mut fade_in = tile.fade_in_seconds;
            if ui.add(fade_slider(&mut fade_in)).changed() {
                actions.push(set(TileSetting::FadeIn(fade_in)));
            }
            ui.end_row();
            ui.label("Fade out")
                .on_hover_text("Before the end, and when you stop the tile");
            let mut fade_out = tile.fade_out_seconds;
            if ui.add(fade_slider(&mut fade_out)).changed() {
                actions.push(set(TileSetting::FadeOut(fade_out)));
            }
            ui.end_row();
        });

    section(ui, "Effects");
    egui::Grid::new("olio-effects")
        .num_columns(2)
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            ui.label("Reverb");
            let mut percent = tile.reverb * 100.0;
            let slider = egui::Slider::new(&mut percent, 0.0..=100.0)
                .suffix(" %")
                .max_decimals(0);
            if ui.add(slider).changed() {
                actions.push(set(TileSetting::Reverb(percent / 100.0)));
            }
            ui.end_row();
        });

    ui.add_space(20.0);
    ui.with_layout(Layout::top_down(Align::Min), |ui| {
        if ui.button("Remove Tile").clicked() {
            actions.push(InspectorAction::Remove(tile.id));
        }
    });
}

fn fade_slider(seconds: &mut f32) -> egui::Slider<'_> {
    egui::Slider::new(seconds, 0.0..=MAX_FADE_SECONDS)
        .logarithmic(true)
        .smallest_positive(0.05)
        .suffix(" s")
        .max_decimals(2)
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(14.0);
    ui.label(RichText::new(title).strong());
    ui.add_space(4.0);
}

fn swatch(ui: &mut Ui, color: TileColor, chosen: bool) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), 7.5, palette::strong(color, dark));
    if chosen {
        painter.circle_stroke(
            rect.center(),
            9.5,
            Stroke::new(2.0, ui.visuals().strong_text_color()),
        );
    }
    response.on_hover_text(color.name())
}

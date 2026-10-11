//! The panel on the right: the selected tile's sound, playback, fades and
//! reverb.

use egui::{Button, Color32, RichText, Sense, Stroke, Ui, Vec2};
use olio_control::{Command, TileKey, TileSetting};
use olio_library::{MAX_FADE_SECONDS, Set, Tile, TileColor, TileId};

use crate::clips::ClipState;
use crate::menu::{self, MenuAction};
use crate::prefs::Polish;
use crate::{format, palette};

pub enum InspectorAction {
    Command(Command),
    Rename(TileId, String),
    Recolor(TileId, TileColor),
    /// Asks to remove the tile; the app confirms first.
    Remove(TileId),
    /// Apply the Polish preset, or remove it if the tile has it.
    Polish(TileId),
}

pub struct View<'a> {
    pub set: &'a Set,
    pub tile: Option<&'a Tile>,
    pub state: Option<&'a ClipState>,
    pub playing: bool,
    /// Put the cursor in the title field.
    pub focus_title: bool,
    pub polish: Polish,
}

pub fn show(
    ui: &mut Ui,
    view: &View,
    title_draft: &mut Option<(TileId, String)>,
) -> Vec<InspectorAction> {
    let mut actions = Vec::new();
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.add_space(10.0);
            match view.tile {
                Some(tile) => {
                    title_field(ui, tile, view.focus_title, title_draft, &mut actions);
                    tile_settings(ui, tile, view, &mut actions);
                }
                None => set_summary(ui, view.set),
            }
            ui.add_space(10.0);
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
    focus: bool,
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
            .hint_text("Title")
            .desired_width(f32::INFINITY),
    );
    if focus {
        field.request_focus();
    }
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
    ui.add_space(16.0);
    ui.label("Select a tile to change its title, colour, fades, reverb and how it plays.");
    ui.add_space(10.0);
    ui.weak("Press a tile's key or double-click it to play it. The arrow keys move the selection and Return plays it. Hold Esc to stop everything.");
}

fn tile_settings(ui: &mut Ui, tile: &Tile, view: &View, actions: &mut Vec<InspectorAction>) {
    let (state, playing) = (view.state, view.playing);
    let set = |setting| InspectorAction::Command(Command::SetTile(tile.id, setting));

    // Colour.
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
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
                line.push_str(&format!(" · {trimmed:.1} s of silence trimmed"));
            }
            line
        }
        Some(ClipState::Failed(message)) => format!("Key {key} · can't play: {message}"),
        Some(ClipState::Loading) | None => format!("Key {key} · loading…"),
    };
    ui.weak(details);
    ui.add(egui::Label::new(RichText::new(&tile.sound.original_name).weak()).truncate())
        .on_hover_text("The file this sound was imported from");

    ui.add_space(10.0);
    let label = if playing { "■ Stop" } else { "▶ Play" };
    let button = Button::new(label)
        .shortcut_text(&key)
        .min_size(Vec2::new(ui.available_width(), 32.0));
    if ui.add(button).clicked() {
        actions.push(InspectorAction::Command(Command::ToggleTile(tile.id)));
    }
    ui.add_space(4.0);
    polish_button(ui, tile, view.polish, actions);

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
    fill_with_slider(ui);
    ui.label("Fade in");
    let mut fade_in = tile.fade_in_seconds;
    if ui.add(fade_slider(&mut fade_in)).changed() {
        actions.push(set(TileSetting::FadeIn(fade_in)));
    }
    ui.add_space(4.0);
    ui.label("Fade out")
        .on_hover_text("Before the end, and when you stop the tile");
    let mut fade_out = tile.fade_out_seconds;
    if ui.add(fade_slider(&mut fade_out)).changed() {
        actions.push(set(TileSetting::FadeOut(fade_out)));
    }

    section(ui, "Effects");
    ui.label("Reverb");
    let mut percent = tile.reverb * 100.0;
    let slider = egui::Slider::new(&mut percent, 0.0..=100.0)
        .suffix(" %")
        .max_decimals(0);
    if ui.add(slider).changed() {
        actions.push(set(TileSetting::Reverb(percent / 100.0)));
    }

    ui.add_space(28.0);
    let remove = Button::new(RichText::new("Remove Tile…").color(palette::danger()))
        .stroke(Stroke::new(1.0, palette::danger().gamma_multiply(0.6)))
        .fill(Color32::TRANSPARENT);
    if ui
        .add(remove)
        .on_hover_text("Take the tile off the board; the sound file stays")
        .clicked()
    {
        actions.push(InspectorAction::Remove(tile.id));
    }
}

/// One click for the Polish preset's fades and reverb; a second click
/// takes them off again.
fn polish_button(ui: &mut Ui, tile: &Tile, polish: Polish, actions: &mut Vec<InspectorAction>) {
    let polished = polish.applies_to(tile);
    let label = if polished {
        "✔ Polished"
    } else {
        "✨ Polish"
    };
    let shortcut = menu::shortcut_text(ui.ctx(), MenuAction::PolishSelected);
    let button = Button::new(label)
        .shortcut_text(shortcut)
        .selected(polished)
        .min_size(Vec2::new(ui.available_width(), 32.0));
    let hint = if polished {
        "Click to take the fades and reverb off again.".to_owned()
    } else {
        format!("Fade and reverb in one click: {}.", polish.describe())
    };
    let response = ui.add(button).on_hover_text(format!(
        "{hint}\nChange the preset in Playback ▸ Polish Preset…"
    ));
    if response.clicked() {
        actions.push(InspectorAction::Polish(tile.id));
    }
}

pub fn fade_slider(seconds: &mut f32) -> egui::Slider<'_> {
    egui::Slider::new(seconds, 0.0..=MAX_FADE_SECONDS)
        .logarithmic(true)
        .smallest_positive(0.05)
        .suffix(" s")
        .max_decimals(2)
}

/// Makes the sliders below fill the panel, leaving room for the value box.
fn fill_with_slider(ui: &mut Ui) {
    let value_box = 64.0;
    let width = (ui.available_width() - value_box - ui.spacing().item_spacing.x).max(80.0);
    ui.spacing_mut().slider_width = width;
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(16.0);
    ui.label(RichText::new(title).strong());
    ui.add_space(6.0);
}

fn swatch(ui: &mut Ui, color: TileColor, chosen: bool) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), 8.0, palette::strong(color, dark));
    if chosen {
        painter.circle_stroke(
            rect.center(),
            10.5,
            Stroke::new(2.0, ui.visuals().strong_text_color()),
        );
    } else if response.hovered() {
        painter.circle_stroke(
            rect.center(),
            10.5,
            Stroke::new(1.0, ui.visuals().weak_text_color()),
        );
    }
    response.on_hover_text(color.name())
}

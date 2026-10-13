//! The panel on the right: the selected tile's sound, playback, fades and
//! reverb.

use egui::{Button, Color32, Rect, RichText, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType};
use olio_control::{Command, TileKey, TileSetting};
use olio_library::{MAX_COLUMNS, MAX_FADE_SECONDS, Set, Tile, TileColor, TileId};

use crate::clips::ClipState;
use crate::emoji::EmojiImages;
use crate::menu::{self, MenuAction};
use crate::meter::{self, Meter};
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
    /// Open the emoji picker for the tile.
    ChooseEmoji(TileId),
    /// Make the front set's grid this many columns wide.
    SetColumns(u8),
}

pub struct View<'a> {
    pub set: &'a Set,
    pub tile: Option<&'a Tile>,
    pub state: Option<&'a ClipState>,
    pub playing: bool,
    /// Put the cursor in the title field.
    pub focus_title: bool,
    pub polish: Polish,
    pub images: &'a EmojiImages,
    pub spectrograms: bool,
    /// How many columns fit the window now.
    pub fit_columns: u8,
    /// The tile's level meter, while it has one.
    pub level: Option<Meter>,
    pub now: f64,
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
                    ui.horizontal(|ui| {
                        emoji_well(ui, tile, view.images, &mut actions);
                        title_field(ui, tile, view.focus_title, title_draft, &mut actions);
                    });
                    tile_settings(ui, tile, view, &mut actions);
                }
                None => set_summary(ui, view.set, view.fit_columns, &mut actions),
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

fn set_summary(ui: &mut Ui, set: &Set, fit: u8, actions: &mut Vec<InspectorAction>) {
    ui.label(RichText::new(&set.name).strong().size(16.0));
    ui.add_space(4.0);
    ui.weak(match set.tiles.len() {
        1 => "1 tile".to_owned(),
        count => format!("{count} tiles"),
    });
    ui.add_space(12.0);
    columns_row(ui, set, fit, actions);
    ui.add_space(16.0);
    ui.label("Select a tile to change its title, colour, fades, reverb and how it plays.");
    ui.add_space(10.0);
    ui.weak("Press a tile's key or double-click it to play it. The arrow keys move the selection and Return plays it. Hold Esc to stop everything.");
}

/// The set's width, with buttons for one column fewer or more, and the
/// keys its top row ends at.
fn columns_row(ui: &mut Ui, set: &Set, fit: u8, actions: &mut Vec<InspectorAction>) {
    let (columns, narrowest) = (set.columns, set.min_columns());
    ui.horizontal(|ui| {
        ui.label("Columns");
        let fewer = ui
            .add_enabled(
                columns > narrowest,
                Button::new("−").min_size(Vec2::splat(26.0)),
            )
            .on_hover_text(format!(
                "One column fewer ({})",
                menu::shortcut_text(ui.ctx(), MenuAction::Narrower)
            ))
            .on_disabled_hover_text(format!(
                "Tiles in column {narrowest} keep the grid at least this wide"
            ));
        fewer.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, columns > narrowest, "Fewer columns")
        });
        if fewer.clicked() {
            actions.push(InspectorAction::SetColumns(columns - 1));
        }
        ui.label(RichText::new(columns.to_string()).strong());
        let more = ui
            .add_enabled(
                columns < MAX_COLUMNS,
                Button::new("+").min_size(Vec2::splat(26.0)),
            )
            .on_hover_text(format!(
                "One column more ({})",
                menu::shortcut_text(ui.ctx(), MenuAction::Wider)
            ));
        more.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, columns < MAX_COLUMNS, "More columns")
        });
        if more.clicked() {
            actions.push(InspectorAction::SetColumns(columns + 1));
        }
        let fits = fit.max(narrowest) == columns;
        let button = ui
            .add_enabled(!fits, Button::new("Fit to Window"))
            .on_hover_text(format!(
                "{fit} columns fit the window at this tile size ({})",
                menu::shortcut_text(ui.ctx(), MenuAction::FitColumns)
            ))
            .on_disabled_hover_text("It already fits the window");
        if button.clicked() {
            actions.push(InspectorAction::SetColumns(fit));
        }
    });
    // "1 to 4, Q to R…": what the width means on the keyboard.
    let last = |row: u8| {
        TileKey::for_slot(olio_library::Slot::new(row, columns - 1))
            .map_or_else(String::new, |key| key.label())
    };
    ui.weak(format!(
        "Keys 1–{}, Q–{}, A–{}, Z–{}",
        last(0),
        last(1),
        last(2),
        last(3)
    ));
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
            if tile.normalize {
                let db = 20.0 * ready.gain(true, tile.trim_silence).log10();
                if db.abs() >= 0.05 {
                    line.push_str(&format!(" · normalized {db:+.1} dB"));
                }
            }
            line
        }
        Some(ClipState::Failed(message)) => format!("Key {key} · can't play: {message}"),
        Some(ClipState::Loading) | None => format!("Key {key} · loading…"),
    };
    ui.weak(details);
    ui.add(egui::Label::new(RichText::new(&tile.sound.original_name).weak()).truncate())
        .on_hover_text("The file this sound was imported from");
    if view.spectrograms
        && let Some(ClipState::Ready(ready)) = state
    {
        ui.add_space(6.0);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::hover());
        let dark = ui.visuals().dark_mode;
        crate::grid::spectrogram(ui.painter(), rect, ready, None, tile, dark);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Spectrogram"));
        response.on_hover_text("How the sound's energy spreads over frequency as it plays: low at the bottom, high at the top");
    }

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
    ui.add_space(6.0);
    level_row(ui, view.level.unwrap_or_default(), view.now);

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
    let mut normalize = tile.normalize;
    if ui
        .checkbox(&mut normalize, "Normalize level")
        .on_hover_text(
            "Play at the same loudness as other normalized sounds, without clipping. \
             The file is not changed.",
        )
        .changed()
    {
        actions.push(set(TileSetting::Normalize(normalize)));
    }

    section(ui, "Fades");
    fill_with_slider(ui);
    ui.label("Fade in");
    let mut fade_in = tile.fade_in_seconds;
    if menu::keeps_arrows(ui.add(fade_slider(&mut fade_in))).changed() {
        actions.push(set(TileSetting::FadeIn(fade_in)));
    }
    ui.add_space(4.0);
    ui.label("Fade out")
        .on_hover_text("Before the end, and when you stop the tile");
    let mut fade_out = tile.fade_out_seconds;
    if menu::keeps_arrows(ui.add(fade_slider(&mut fade_out))).changed() {
        actions.push(set(TileSetting::FadeOut(fade_out)));
    }

    section(ui, "Effects");
    ui.label("Reverb");
    let mut percent = tile.reverb * 100.0;
    let slider = egui::Slider::new(&mut percent, 0.0..=100.0)
        .suffix(" %")
        .max_decimals(0);
    if menu::keeps_arrows(ui.add(slider)).changed() {
        actions.push(set(TileSetting::Reverb(percent / 100.0)));
    }

    ui.add_space(28.0);
    let remove = Button::new(RichText::new("Remove Tile").color(palette::danger()))
        .stroke(Stroke::new(1.0, palette::danger().gamma_multiply(0.6)))
        .fill(Color32::TRANSPARENT);
    if ui
        .add(remove)
        .on_hover_text("Take the tile off the board; Undo puts it back")
        .clicked()
    {
        actions.push(InspectorAction::Remove(tile.id));
    }
}

/// The tile's emoji, or a place for one; a click opens the picker.
fn emoji_well(ui: &mut Ui, tile: &Tile, images: &EmojiImages, actions: &mut Vec<InspectorAction>) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
    let visuals = ui.visuals();
    let fill = if response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        visuals.extreme_bg_color
    };
    ui.painter().rect_filled(rect, 8, fill);
    let picture = Rect::from_center_size(rect.center(), Vec2::splat(26.0));
    if tile.emoji.is_empty() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "☺",
            egui::FontId::proportional(20.0),
            visuals.weak_text_color(),
        );
    } else {
        images.paint(ui.painter(), picture, &tile.emoji, visuals.text_color());
    }
    let shortcut = menu::shortcut_text(ui.ctx(), MenuAction::ChooseEmoji);
    let hint = if tile.emoji.is_empty() {
        format!("Choose an emoji for this tile ({shortcut})")
    } else {
        format!("Change the emoji ({shortcut})")
    };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Emoji"));
    if response.on_hover_text(hint).clicked() {
        actions.push(InspectorAction::ChooseEmoji(tile.id));
    }
}

/// How loud the tile is playing, with the loudest level lately in dB.
fn level_row(ui: &mut Ui, level: Meter, now: f64) {
    ui.horizontal(|ui| {
        ui.label("Level");
        let readout_width = 58.0;
        let width = (ui.available_width() - readout_width - ui.spacing().item_spacing.x).max(40.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 8.0), Sense::hover());
        let dark = ui.visuals().dark_mode;
        meter::paint_bar(ui.painter(), rect, &level, meter::Colors::Zones, dark);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Tile level"));
        let text = if level.is_resting(now) {
            "silent".to_owned()
        } else {
            meter::readout(level.hold_db())
        };
        ui.add_sized(
            [readout_width, 16.0],
            egui::Label::new(RichText::new(text).weak()),
        )
        .on_hover_text("The loudest level in the last moment, before the master volume");
    });
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
pub(crate) fn fill_with_slider(ui: &mut Ui) {
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

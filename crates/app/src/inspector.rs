//! The panel on the right: the selected tile, or the front set when none
//! is. The tile's name, emoji and colour are always in view; the rest is
//! grouped in sections that open and close (Sound, How it plays, Polish),
//! remembered in the preferences. Every control that is not obvious says
//! what it does on hover.

use egui::{Rect, RichText, Sense, Stroke, Ui, Vec2, WidgetInfo, WidgetType};
use olio_control::{Command, TileKey, TileSetting};
use olio_library::{MAX_COLUMNS, MAX_FADE_SECONDS, Set, Tile, TileColor, TileId};

use crate::clips::ClipState;
use crate::emoji::EmojiImages;
use crate::menu::{self, MenuAction};
use crate::meter::{self, Meter};
use crate::prefs::{Disclosure, Polish};
use crate::skin::{self, Disclosed};
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
    disclosure: &mut Disclosure,
) -> Vec<InspectorAction> {
    let mut actions = Vec::new();
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            // Clear of the scroll bar, which floats over the content.
            ui.set_max_width(ui.available_width() - 12.0);
            ui.add_space(10.0);
            match view.tile {
                Some(tile) => {
                    ui.horizontal(|ui| {
                        emoji_well(ui, tile, view.images, &mut actions);
                        title_field(ui, tile, view.focus_title, title_draft, &mut actions);
                    });
                    tile_settings(ui, tile, view, disclosure, &mut actions);
                }
                None => set_summary(ui, view.set, view.fit_columns, disclosure, &mut actions),
            }
            ui.add_space(10.0);
        });
    actions
}

/// A section of a panel whose open or closed state is kept in
/// `disclosure` under `key`.
pub(crate) fn section<R>(
    ui: &mut Ui,
    disclosure: &mut Disclosure,
    key: &str,
    title: &str,
    body: impl FnOnce(&mut Ui) -> R,
) -> Disclosed<R> {
    let mut open = disclosure.is_open(key);
    let shown = skin::section(ui, title, &mut open, body);
    disclosure.set_open(key, open);
    shown
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
    let field = skin::field(ui, |ui| {
        ui.add(
            egui::TextEdit::singleline(text)
                .id(id)
                .font(egui::TextStyle::Heading)
                .hint_text("Title")
                .desired_width(f32::INFINITY),
        )
    })
    .on_hover_text("The tile's title, as it reads on the tile and in the toolbar");
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

fn set_summary(
    ui: &mut Ui,
    set: &Set,
    fit: u8,
    disclosure: &mut Disclosure,
    actions: &mut Vec<InspectorAction>,
) {
    ui.label(RichText::new(&set.name).strong().size(16.0));
    ui.add_space(4.0);
    ui.weak(match set.tiles.len() {
        1 => "1 tile".to_owned(),
        count => format!("{count} tiles"),
    });
    section(ui, disclosure, "columns", "Columns", |ui| {
        columns_row(ui, set, fit, actions);
    });
    ui.add_space(20.0);
    ui.label("Select a tile to change its title, colour, fades, reverb and how it plays.");
    ui.add_space(10.0);
    ui.weak("Press a tile's key or double-click it to play it. The arrow keys move the selection and Return plays it. Hold Esc to stop everything.");
}

/// The set's width, with buttons for one column fewer or more, and the
/// keys its top row ends at.
fn columns_row(ui: &mut Ui, set: &Set, fit: u8, actions: &mut Vec<InspectorAction>) {
    let (columns, narrowest) = (set.columns, set.min_columns());
    ui.horizontal(|ui| {
        let fewer = ui
            .add_enabled_ui(columns > narrowest, |ui| skin::button(ui, "−", false))
            .inner
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
        skin::readout_label(ui, 36.0, &columns.to_string())
            .on_hover_text("How many columns wide the set is");
        let more = ui
            .add_enabled_ui(columns < MAX_COLUMNS, |ui| skin::button(ui, "+", false))
            .inner
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
            .add_enabled_ui(!fits, |ui| skin::button(ui, "Fit to Window", false))
            .inner
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
    ))
    .on_hover_text("The keys that play each row of tiles at this width");
}

fn tile_settings(
    ui: &mut Ui,
    tile: &Tile,
    view: &View,
    disclosure: &mut Disclosure,
    actions: &mut Vec<InspectorAction>,
) {
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
    ui.weak(details)
        .on_hover_text("The key that plays it, how long it plays, and what Trim silence and Normalize level do to it");
    ui.add(egui::Label::new(RichText::new(&tile.sound.original_name).weak()).truncate())
        .on_hover_text("The file this sound was imported from");

    section(ui, disclosure, "sound", "Sound", |ui| {
        if view.spectrograms
            && let Some(ClipState::Ready(ready)) = state
        {
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::hover());
            let dark = ui.visuals().dark_mode;
            crate::grid::spectrogram(ui.painter(), rect, ready, None, tile, dark);
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Spectrogram"));
            response.on_hover_text("How the sound's energy spreads over frequency as it plays: low at the bottom, high at the top");
            ui.add_space(8.0);
        }
        let (label, hint) = if playing {
            ("■ Stop", "Stops the tile, with its fade out".to_owned())
        } else {
            ("▶ Play", format!("Plays the tile, as pressing {key} does"))
        };
        if skin::wide_button(ui, label, Some(&key), false)
            .on_hover_text(hint)
            .clicked()
        {
            actions.push(InspectorAction::Command(Command::ToggleTile(tile.id)));
        }
        ui.add_space(6.0);
        level_row(ui, view.level.unwrap_or_default(), view.now);
    });

    section(ui, disclosure, "how_it_plays", "How it plays", |ui| {
        let mut looping = tile.looping;
        if skin::switch(ui, "Loop", &mut looping)
            .on_hover_text("Plays again from the start until you stop it")
            .clicked()
        {
            actions.push(set(TileSetting::Looping(looping)));
        }
        let mut trim = tile.trim_silence;
        if skin::switch(ui, "Trim silence", &mut trim)
            .on_hover_text("Skips the silence at the start and end of the sound")
            .clicked()
        {
            actions.push(set(TileSetting::TrimSilence(trim)));
        }
        let mut normalize = tile.normalize;
        if skin::switch(ui, "Normalize level", &mut normalize)
            .on_hover_text(
                "Plays at the same loudness as other normalized sounds, without clipping. \
                 The file is not changed.",
            )
            .clicked()
        {
            actions.push(set(TileSetting::Normalize(normalize)));
        }
    });

    section(ui, disclosure, "polish", "Polish", |ui| {
        polish_button(ui, tile, view.polish, actions);
        ui.add_space(8.0);
        let mut fade_in = tile.fade_in_seconds;
        if fade_row(ui, "Fade in", &mut fade_in)
            .on_hover_text("How long the sound takes to rise from silence when it starts")
            .changed()
        {
            actions.push(set(TileSetting::FadeIn(fade_in)));
        }
        let mut fade_out = tile.fade_out_seconds;
        if fade_row(ui, "Fade out", &mut fade_out)
            .on_hover_text(
                "How long it takes to sink to silence before the end, and when you stop the tile",
            )
            .changed()
        {
            actions.push(set(TileSetting::FadeOut(fade_out)));
        }
        let mut reverb = tile.reverb;
        let readout = format!("{:.0} %", reverb * 100.0);
        if skin::slider_row(ui, "Reverb", "Reverb", &mut reverb, 0.0..=1.0, &readout)
            .on_hover_text("How much of the tile goes to the room reverb that every tile shares")
            .changed()
        {
            actions.push(set(TileSetting::Reverb(reverb)));
        }
    });

    ui.add_space(24.0);
    if skin::alarm_button(ui, "Remove Tile")
        .on_hover_text(format!(
            "Takes the tile off the board ({}); Undo puts it back",
            menu::shortcut_text(ui.ctx(), MenuAction::RemoveTile)
        ))
        .clicked()
    {
        actions.push(InspectorAction::Remove(tile.id));
    }
}

/// The tile's emoji, or a place for one; a click opens the picker.
fn emoji_well(ui: &mut Ui, tile: &Tile, images: &EmojiImages, actions: &mut Vec<InspectorAction>) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
    let p = skin::palette(ui.ctx());
    // A well the emoji sits in, a little lighter under the pointer.
    let floor = if response.hovered() {
        p.sunken.lerp_to_gamma(p.light, 0.25)
    } else {
        p.sunken
    };
    skin::inset(ui.painter(), rect, skin::BUTTON_RADIUS, floor);
    let picture = Rect::from_center_size(rect.center(), Vec2::splat(26.0));
    if tile.emoji.is_empty() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "☺",
            egui::FontId::proportional(20.0),
            p.text_weak,
        );
    } else {
        images.paint(ui.painter(), picture, &tile.emoji, p.text);
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
        skin::row_name(ui, "Level");
        let readout_width = skin::ROW_READOUT_WIDTH;
        let width = (ui.available_width() - readout_width - ui.spacing().item_spacing.x).max(40.0);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 8.0), Sense::hover());
        meter::paint_bar(ui.painter(), rect, &level, meter::Colors::Zones);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Tile level"));
        response.on_hover_text("How loud the tile is playing now, before the master volume");
        let text = if level.is_resting(now) {
            "silent".to_owned()
        } else {
            meter::readout(level.hold_db())
        };
        skin::readout_label(ui, readout_width, &text)
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
    let hint = if polished {
        "Click to take the fades and reverb off again.".to_owned()
    } else {
        format!("Fade and reverb in one click: {}.", polish.describe())
    };
    let response = skin::wide_button(ui, label, Some(&shortcut), polished).on_hover_text(format!(
        "{hint}\nChange the preset in Playback ▸ Polish Preset…"
    ));
    if response.clicked() {
        actions.push(InspectorAction::Polish(tile.id));
    }
}

/// A fade's length on a slider row: the slider moves along the square root
/// of the seconds, so the short fades most tiles want have most of its
/// travel. Marks the response changed only when the seconds change.
pub fn fade_row(ui: &mut Ui, name: &str, seconds: &mut f32) -> egui::Response {
    let mut along = (seconds.max(0.0) / MAX_FADE_SECONDS).sqrt();
    let readout = fade_words(*seconds);
    let response = skin::slider_row(ui, name, name, &mut along, 0.0..=1.0, &readout);
    if response.changed() {
        // To a hundredth of a second, as the readout shows.
        *seconds = ((MAX_FADE_SECONDS * along * along) * 100.0).round() / 100.0;
    }
    response
}

/// "0.50 s", "12.0 s".
fn fade_words(seconds: f32) -> String {
    if seconds < 10.0 {
        format!("{seconds:.2} s")
    } else {
        format!("{seconds:.1} s")
    }
}

fn swatch(ui: &mut Ui, color: TileColor, chosen: bool) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), 8.0, palette::strong(color, dark));
    let p = skin::palette(ui.ctx());
    if chosen {
        painter.circle_stroke(rect.center(), 10.5, Stroke::new(2.0, p.text));
    } else if response.hovered() {
        painter.circle_stroke(rect.center(), 10.5, Stroke::new(1.0, p.accent));
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, chosen, color.name()));
    response.on_hover_text(format!("Colour the tile {}", color.name().to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_reads_to_a_hundredth_under_ten_seconds() {
        assert_eq!(fade_words(0.5), "0.50 s");
        assert_eq!(fade_words(12.25), "12.2 s");
    }
}

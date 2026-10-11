//! Emoji on tiles: colour pictures from Twemoji, and the picker.
//!
//! Pictures are decoded on first use and kept as textures. An emoji Twemoji
//! has no picture for falls back to egui's emoji font, in one colour.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

use egui::{
    Align, Align2, Color32, ColorImage, FontId, Id, Key, Layout, Painter, Rect, RichText, Sense,
    TextureFilter, TextureHandle, TextureId, TextureOptions, Ui, Vec2, WidgetInfo, WidgetType,
};
use emojis::{Emoji, Group};
use olio_library::TileId;
use twemoji_assets::png::PngTwemojiAsset;

/// The Twemoji PNG for an emoji. Twemoji names some emoji without their
/// variation selector, so both spellings are tried.
fn png(emoji: &str) -> Option<&'static [u8]> {
    let asset = PngTwemojiAsset::from_emoji(emoji)
        .or_else(|| PngTwemojiAsset::from_emoji(&emoji.replace('\u{fe0f}', "")))?;
    Some(**asset)
}

/// Decoded emoji pictures, one texture each, made on first use.
#[derive(Default)]
pub struct EmojiImages {
    textures: RefCell<HashMap<String, Option<TextureHandle>>>,
}

impl EmojiImages {
    /// The emoji's picture, or `None` if Twemoji has none.
    pub fn texture(&self, ctx: &egui::Context, emoji: &str) -> Option<TextureId> {
        if let Some(found) = self.textures.borrow().get(emoji) {
            return found.as_ref().map(TextureHandle::id);
        }
        let handle = png(emoji).and_then(|bytes| {
            let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
            let image = image.to_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            let pixels = ColorImage::from_rgba_unmultiplied(size, image.as_raw());
            let options = TextureOptions {
                mipmap_mode: Some(TextureFilter::Linear),
                ..TextureOptions::LINEAR
            };
            Some(ctx.load_texture(format!("emoji {emoji}"), pixels, options))
        });
        let id = handle.as_ref().map(TextureHandle::id);
        self.textures.borrow_mut().insert(emoji.to_owned(), handle);
        id
    }

    /// Paints the emoji to fill `rect`: its picture, or the emoji font in
    /// `fallback` colour.
    pub fn paint(&self, painter: &Painter, rect: Rect, emoji: &str, fallback: Color32) {
        match self.texture(painter.ctx(), emoji) {
            Some(texture) => {
                let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.image(texture, rect, uv, Color32::WHITE);
            }
            None => {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    emoji,
                    FontId::proportional(rect.height() * 0.85),
                    fallback,
                );
            }
        }
    }
}

/// Every emoji with a picture, in Unicode's order.
fn all() -> &'static [&'static Emoji] {
    static ALL: OnceLock<Vec<&'static Emoji>> = OnceLock::new();
    ALL.get_or_init(|| {
        emojis::iter()
            .filter(|emoji| png(emoji.as_str()).is_some())
            .collect()
    })
}

/// Emoji a soundboard often wants, shown first.
const SUGGESTED: &[&str] = &[
    "👏", "🎉", "🥳", "😂", "🤣", "😱", "😮", "🤔", "😴", "🔔", "🚨", "📣", "📯", "🎺", "🥁", "🎸",
    "🎹", "🎻", "🎵", "🎶", "🎤", "🎧", "📻", "💥", "🔥", "⚡", "💯", "✅", "❌", "⏰", "⏳", "🏆",
    "🥇", "👻", "💀", "🤖", "👽", "🐶", "🐱", "🐔", "🦆", "🐸", "🌧️", "🌊", "🚗", "🚀", "✈️", "🚂",
    "💸", "🍾", "👋", "🙌", "👍", "👎", "❤️", "💔", "⭐", "🎬", "🎮",
];

fn suggested() -> &'static [&'static Emoji] {
    static LIST: OnceLock<Vec<&'static Emoji>> = OnceLock::new();
    LIST.get_or_init(|| {
        SUGGESTED
            .iter()
            .filter_map(|text| emojis::get(text))
            .filter(|emoji| png(emoji.as_str()).is_some())
            .collect()
    })
}

/// Emoji whose name or a shortcode contains every word of `query`.
fn search(query: &str) -> Vec<&'static Emoji> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    all()
        .iter()
        .copied()
        .filter(|emoji| {
            let mut text = emoji.name().to_owned();
            for shortcode in emoji.shortcodes() {
                text.push(' ');
                text.push_str(&shortcode.replace('_', " "));
            }
            words.iter().all(|word| text.contains(word.as_str()))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Suggested,
    Group(Group),
}

fn group_name(group: Group) -> &'static str {
    match group {
        Group::SmileysAndEmotion => "Smileys",
        Group::PeopleAndBody => "People",
        Group::AnimalsAndNature => "Animals and nature",
        Group::FoodAndDrink => "Food and drink",
        Group::TravelAndPlaces => "Travel and places",
        Group::Activities => "Activities",
        Group::Objects => "Objects",
        Group::Symbols => "Symbols",
        Group::Flags => "Flags",
    }
}

/// The emoji shown on a group's tab.
fn group_icon(group: Group) -> &'static str {
    match group {
        Group::SmileysAndEmotion => "😀",
        Group::PeopleAndBody => "👋",
        Group::AnimalsAndNature => "🐻",
        Group::FoodAndDrink => "🍔",
        Group::TravelAndPlaces => "🚗",
        Group::Activities => "⚽",
        Group::Objects => "💡",
        Group::Symbols => "🔣",
        Group::Flags => "🏁",
    }
}

/// The open picker, for one tile.
pub struct Picker {
    pub tile: TileId,
    title: String,
    current: String,
    query: String,
    tab: Tab,
    focused: bool,
}

impl Picker {
    pub fn new(tile: TileId, title: &str, current: &str) -> Self {
        Self {
            tile,
            title: title.to_owned(),
            current: current.to_owned(),
            query: String::new(),
            tab: Tab::Suggested,
            focused: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Waiting,
    Cancelled,
    /// The emoji chosen, or `None` to take it off.
    Chose(Option<&'static str>),
}

const COLUMNS: usize = 9;
const CELL: f32 = 38.0;

/// Shows the picker as a modal. Escape or a click outside cancels;
/// Return picks the first match of a search.
pub fn picker(ctx: &egui::Context, picker: &mut Picker, images: &EmojiImages) -> Reply {
    let modal = egui::Modal::new(Id::new("olio-emoji-picker")).show(ctx, |ui| {
        let width = COLUMNS as f32 * CELL + 16.0;
        ui.set_width(width);
        ui.label(RichText::new(format!("Emoji for “{}”", picker.title)).heading());
        ui.add_space(8.0);
        let field = ui.add(
            egui::TextEdit::singleline(&mut picker.query)
                .hint_text("Search, like “clap” or “party”")
                .desired_width(f32::INFINITY),
        );
        if !picker.focused {
            field.request_focus();
            picker.focused = true;
        }
        let searching = !picker.query.trim().is_empty();
        let list: Vec<&'static Emoji> = if searching {
            search(&picker.query)
        } else {
            match picker.tab {
                Tab::Suggested => suggested().to_vec(),
                Tab::Group(group) => all()
                    .iter()
                    .copied()
                    .filter(|emoji| emoji.group() == group)
                    .collect(),
            }
        };
        let mut reply = Reply::Waiting;
        if searching
            && field.lost_focus()
            && ui.input(|input| input.key_pressed(Key::Enter))
            && let Some(first) = list.first()
        {
            reply = Reply::Chose(Some(first.as_str()));
        }

        ui.add_space(6.0);
        if !searching {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if tab(ui, images, "⭐", "Suggested", picker.tab == Tab::Suggested) {
                    picker.tab = Tab::Suggested;
                }
                for group in Group::iter() {
                    let chosen = picker.tab == Tab::Group(group);
                    if tab(ui, images, group_icon(group), group_name(group), chosen) {
                        picker.tab = Tab::Group(group);
                    }
                }
            });
            ui.add_space(4.0);
        }

        if list.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| ui.weak("No emoji match"));
            ui.add_space(30.0);
        } else {
            let rows = list.len().div_ceil(COLUMNS);
            egui::ScrollArea::vertical()
                .max_height(CELL * 7.0)
                .auto_shrink([false, true])
                .show_rows(ui, CELL, rows, |ui, range| {
                    for row in range {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::ZERO;
                            let start = row * COLUMNS;
                            for emoji in &list[start..(start + COLUMNS).min(list.len())] {
                                let chosen = emoji.as_str() == picker.current;
                                if cell(ui, images, emoji, chosen).clicked() {
                                    reply = Reply::Chose(Some(emoji.as_str()));
                                }
                            }
                        });
                    }
                });
        }

        ui.add_space(10.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Cancel").clicked() {
                reply = Reply::Cancelled;
            }
            let has = !picker.current.is_empty();
            if ui.add_enabled(has, egui::Button::new("No Emoji")).clicked() {
                reply = Reply::Chose(None);
            }
        });
        reply
    });
    if modal.inner == Reply::Waiting && modal.should_close() {
        return Reply::Cancelled;
    }
    modal.inner
}

/// A group tab; true when clicked.
fn tab(ui: &mut Ui, images: &EmojiImages, icon: &str, name: &str, chosen: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    let visuals = ui.visuals();
    if chosen {
        ui.painter()
            .rect_filled(rect, 6, visuals.selection.bg_fill.gamma_multiply(0.5));
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6, visuals.widgets.hovered.weak_bg_fill);
    }
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(20.0));
    images.paint(ui.painter(), icon_rect, icon, visuals.text_color());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, chosen, name));
    response.on_hover_text(name).clicked()
}

fn cell(ui: &mut Ui, images: &EmojiImages, emoji: &Emoji, chosen: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(CELL), Sense::click());
    let visuals = ui.visuals();
    if chosen {
        ui.painter()
            .rect_filled(rect, 8, visuals.selection.bg_fill.gamma_multiply(0.5));
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 8, visuals.widgets.hovered.weak_bg_fill);
    }
    let picture = Rect::from_center_size(rect.center(), Vec2::splat(28.0));
    images.paint(ui.painter(), picture, emoji.as_str(), visuals.text_color());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, emoji.name()));
    response.on_hover_text(emoji.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearly_every_emoji_has_a_picture() {
        let total = emojis::iter().count();
        let missing: Vec<&str> = emojis::iter()
            .filter(|emoji| png(emoji.as_str()).is_none())
            .map(|emoji| emoji.as_str())
            .collect();
        assert!(
            missing.len() * 100 < total,
            "{} of {total} have no picture: {missing:?}",
            missing.len()
        );
        assert!(png("☺️").is_some(), "found with or without the selector");
        assert_eq!(
            suggested().len(),
            SUGGESTED.len(),
            "every suggestion has a picture"
        );
    }

    #[test]
    fn search_matches_every_word_in_names_and_shortcodes() {
        let names = |query| -> Vec<&str> { search(query).iter().map(|e| e.as_str()).collect() };
        assert!(names("party popper").contains(&"🎉"));
        assert!(names("tada").contains(&"🎉"));
        assert!(names("CLAP").contains(&"👏"));
        assert!(names("clapping zebra").is_empty());
    }
}

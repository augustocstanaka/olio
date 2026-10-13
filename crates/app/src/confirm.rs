//! An in-window confirmation before something is removed. Return confirms,
//! Escape or a click outside cancels. In the window rather than a native
//! dialog so it looks the same on both platforms and can be driven by tests.

use egui::{Align, Button, Color32, Id, Key, Layout, Modifiers, RichText};
use olio_library::SetId;

use crate::palette;

pub enum Pending {
    RemoveSet(SetId),
    /// A saved style, by its place in the list.
    RemoveStyle(usize),
}

pub struct Confirm {
    pub title: String,
    pub body: String,
    /// The confirming button's label, a verb: "Remove".
    pub verb: &'static str,
    pub pending: Pending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Waiting,
    Cancelled,
    Confirmed,
}

pub fn show(ctx: &egui::Context, confirm: &Confirm) -> Reply {
    let modal = egui::Modal::new(Id::new("olio-confirm")).show(ctx, |ui| {
        ui.set_max_width(400.0);
        ui.add_space(4.0);
        ui.label(RichText::new(&confirm.title).heading());
        ui.add_space(8.0);
        ui.label(&confirm.body);
        ui.add_space(18.0);
        let mut reply = Reply::Waiting;
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let danger = Button::new(RichText::new(confirm.verb).color(Color32::WHITE).strong())
                .fill(palette::danger());
            if ui.add(danger).clicked() {
                reply = Reply::Confirmed;
            }
            if ui.button("Cancel").clicked() {
                reply = Reply::Cancelled;
            }
        });
        reply
    });
    let mut reply = modal.inner;
    if reply == Reply::Waiting {
        if modal.should_close() {
            reply = Reply::Cancelled;
        } else if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter)) {
            reply = Reply::Confirmed;
        }
    }
    reply
}

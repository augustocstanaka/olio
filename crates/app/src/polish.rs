//! The Polish Preset window: what one click of Polish gives a tile.

use egui::{Align2, RichText};

use crate::inspector::fade_slider;
use crate::prefs::Polish;

/// Shows the window while `open` is true. Edits `polish` in place; it is
/// saved with the other preferences.
pub fn window(ctx: &egui::Context, open: &mut bool, polish: &mut Polish) {
    egui::Window::new("Polish Preset")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label("One click of Polish gives a tile these settings.");
            ui.add_space(10.0);
            ui.spacing_mut().slider_width = 220.0;
            ui.label("Fade in");
            ui.add(fade_slider(&mut polish.fade_in_seconds));
            ui.add_space(4.0);
            ui.label("Fade out");
            ui.add(fade_slider(&mut polish.fade_out_seconds));
            ui.add_space(4.0);
            ui.label("Reverb");
            let mut percent = polish.reverb * 100.0;
            let slider = egui::Slider::new(&mut percent, 0.0..=100.0)
                .suffix(" %")
                .max_decimals(0);
            if ui.add(slider).changed() {
                polish.reverb = percent / 100.0;
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let default = Polish::default();
                let reset = egui::Button::new(format!("Reset to {}", default.describe()));
                if ui.add_enabled(*polish != default, reset).clicked() {
                    *polish = default;
                }
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Tiles already polished keep their values until you polish them again.",
                )
                .weak(),
            );
        });
}

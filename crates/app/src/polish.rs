//! The Polish Preset window: what one click of Polish gives a tile.

use egui::{Align2, RichText};

use crate::inspector::fade_row;
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
            fade_row(ui, "Fade in", &mut polish.fade_in_seconds);
            fade_row(ui, "Fade out", &mut polish.fade_out_seconds);
            let readout = format!("{:.0} %", polish.reverb * 100.0);
            crate::skin::slider_row(
                ui,
                "Reverb",
                "Reverb",
                &mut polish.reverb,
                0.0..=1.0,
                &readout,
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let default = Polish::default();
                let reset = format!("Reset to {}", default.describe());
                if ui
                    .add_enabled_ui(*polish != default, |ui| {
                        crate::skin::button(ui, &reset, false)
                    })
                    .inner
                    .clicked()
                {
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

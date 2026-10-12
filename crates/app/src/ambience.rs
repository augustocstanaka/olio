//! The Ambience window: the generated bed's style, key, weather,
//! instruments, presence, form and level. The settings live in the
//! preferences and go to the engine when they change.

use egui::{Align2, RichText, Ui};
use olio_audio::ambience::{
    AmbienceSettings, Density, Form, Instrument, Presence, ROOT_NAMES, Register, ScaleKind, Style,
    Weather,
};

/// Shows the window while `open` is true. Returns true when a setting
/// changed.
pub fn window(ctx: &egui::Context, open: &mut bool, settings: &mut AmbienceSettings) -> bool {
    let before = *settings;
    egui::Window::new("Ambience")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_width(560.0);
            ui.label(
                "A quiet, endless bed behind your sounds, made up as it goes. Nothing repeats.",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut settings.enabled, "Play the ambience");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("New variation")
                        .on_hover_text("The same settings, different notes")
                        .clicked()
                    {
                        settings.seed = settings.seed.wrapping_add(1);
                    }
                });
            });
            ui.add_space(6.0);

            styles(ui, settings);
            ui.add_space(10.0);

            // One part at a time, so the window stays small.
            let tab_id = ui.make_persistent_id("olio-ambience-tab");
            let mut tab: Tab = ui.data(|data| data.get_temp(tab_id)).unwrap_or_default();
            ui.horizontal(|ui| {
                for choice in Tab::ALL {
                    if ui.selectable_label(tab == choice, choice.name()).clicked() {
                        tab = choice;
                    }
                }
            });
            ui.data_mut(|data| data.insert_temp(tab_id, tab));
            ui.separator();
            ui.add_space(4.0);
            ui.allocate_ui(egui::vec2(ui.available_width(), 230.0), |ui| match tab {
                Tab::Key => {
                    key(ui, settings);
                    section(ui, "Weather");
                    choose(
                        ui,
                        &mut settings.weather,
                        &Weather::ALL,
                        Weather::name,
                        |_| "",
                    );
                }
                Tab::Instruments => instruments(ui, settings),
                Tab::Presence => presence(ui, settings),
                Tab::Form => form(ui, settings),
            });
            ui.add_space(6.0);
            ui.separator();
            level(ui, settings);
            ui.add_space(4.0);
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!(
                    "{} · {}: {}.",
                    settings.describe(),
                    settings.scale.name(),
                    settings.scale.kind.character()
                ))
                .weak(),
            );
        });
    // Anything but picking a style, switching on or a new variation makes
    // the settings Custom.
    let touched = AmbienceSettings {
        enabled: before.enabled,
        seed: before.seed,
        style: before.style,
        ..*settings
    } != before;
    if touched && settings.style == before.style {
        settings.style = None;
    }
    *settings != before
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Key,
    Instruments,
    Presence,
    Form,
}

impl Tab {
    const ALL: [Self; 4] = [Self::Key, Self::Instruments, Self::Presence, Self::Form];

    fn name(self) -> &'static str {
        match self {
            Self::Key => "Key & Weather",
            Self::Instruments => "Instruments",
            Self::Presence => "Presence",
            Self::Form => "Form",
        }
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).strong());
    ui.add_space(4.0);
}

/// A row of choices; true when one was picked.
fn choose<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    all: &[T],
    name: impl Fn(T) -> &'static str,
    hint: impl Fn(T) -> &'static str,
) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for &choice in all {
            let response = ui.selectable_label(*value == choice, name(choice));
            let response = match hint(choice) {
                "" => response,
                text => response.on_hover_text(text),
            };
            if response.clicked() && *value != choice {
                *value = choice;
                changed = true;
            }
        }
    });
    changed
}

fn styles(ui: &mut Ui, settings: &mut AmbienceSettings) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label("Style");
        for style in Style::ALL {
            let chosen = settings.style == Some(style);
            if ui
                .selectable_label(chosen, style.name())
                .on_hover_text(style.describe())
                .clicked()
                && !chosen
            {
                // Everything from the style; whether it plays stays.
                let enabled = settings.enabled;
                *settings = style.settings();
                settings.enabled = enabled;
            }
        }
        if settings.style.is_none() {
            let _ = ui
                .selectable_label(true, "Custom")
                .on_hover_text("Pick a style to start over from one");
        }
    });
}

fn key(ui: &mut Ui, settings: &mut AmbienceSettings) {
    ui.horizontal(|ui| {
        ui.label("Root");
        ui.spacing_mut().item_spacing.x = 3.0;
        for (index, name) in ROOT_NAMES.iter().enumerate() {
            let root = index as u8;
            if ui
                .selectable_label(settings.scale.root == root, *name)
                .clicked()
            {
                settings.scale.root = root;
            }
        }
    });
    ui.add_space(4.0);
    choose(
        ui,
        &mut settings.scale.kind,
        &ScaleKind::all(),
        ScaleKind::name,
        ScaleKind::character,
    );
}

fn instruments(ui: &mut Ui, settings: &mut AmbienceSettings) {
    for instrument in Instrument::ALL {
        let layer = &mut settings.layers[instrument.index()];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let name = ui.add_sized(
                [78.0, 20.0],
                egui::Checkbox::new(&mut layer.enabled, instrument.name()),
            );
            name.on_hover_text(instrument.describe());
            ui.add_enabled_ui(layer.enabled, |ui| {
                let mut percent = layer.level * 100.0;
                ui.spacing_mut().slider_width = 80.0;
                if ui
                    .add(
                        egui::Slider::new(&mut percent, 0.0..=100.0)
                            .show_value(false)
                            .trailing_fill(true),
                    )
                    .on_hover_text(format!("Level {percent:.0} %"))
                    .changed()
                {
                    layer.level = percent / 100.0;
                }
                ui.add_space(6.0);
                if instrument == Instrument::Drone {
                    ui.add_space(176.0);
                } else {
                    row(ui, &mut layer.density, &Density::ALL, Density::name);
                }
                ui.add_space(6.0);
                row(ui, &mut layer.register, &Register::ALL, Register::name);
            });
        });
    }
    if settings.layers.iter().all(|layer| !layer.enabled) {
        ui.weak("No instruments: weather only.");
    }
}

/// Small choices side by side, never wrapping.
fn row<T: Copy + PartialEq>(
    ui: &mut Ui,
    value: &mut T,
    all: &[T],
    name: impl Fn(T) -> &'static str,
) {
    ui.spacing_mut().item_spacing.x = 2.0;
    for &choice in all {
        if ui
            .selectable_label(*value == choice, name(choice))
            .clicked()
        {
            *value = choice;
        }
    }
}

fn presence(ui: &mut Ui, settings: &mut AmbienceSettings) {
    choose(
        ui,
        &mut settings.presence,
        &Presence::ALL,
        Presence::name,
        Presence::describe,
    );
    if settings.presence.pulsed() {
        ui.horizontal(|ui| {
            ui.label("Pulse");
            let mut tempo = f32::from(settings.tempo);
            ui.spacing_mut().slider_width = 160.0;
            if ui
                .add(
                    egui::Slider::new(&mut tempo, 30.0..=120.0)
                        .suffix(" BPM")
                        .max_decimals(0),
                )
                .changed()
            {
                settings.tempo = tempo.round() as u16;
            }
        });
    }
    ui.weak(settings.presence.describe());
}

fn form(ui: &mut Ui, settings: &mut AmbienceSettings) {
    let pieces = matches!(settings.form, Form::Piece { .. });
    ui.horizontal(|ui| {
        if ui.selectable_label(!pieces, "Endless").clicked() && pieces {
            settings.form = Form::Endless;
        }
        if ui
            .selectable_label(pieces, "Pieces")
            .on_hover_text("Each piece opens, builds, closes on the root, then pauses")
            .clicked()
            && !pieces
        {
            settings.form = Form::Piece {
                minutes: 3.0,
                pause_seconds: 20.0,
            };
        }
    });
    if let Form::Piece {
        minutes,
        pause_seconds,
    } = &mut settings.form
    {
        egui::Grid::new("olio-ambience-form")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.spacing_mut().slider_width = 160.0;
                ui.label("Length");
                ui.add(
                    egui::Slider::new(minutes, 0.5..=30.0)
                        .suffix(" min")
                        .max_decimals(1),
                );
                ui.end_row();
                ui.label("Pause");
                ui.add(
                    egui::Slider::new(pause_seconds, 0.0..=120.0)
                        .suffix(" s")
                        .max_decimals(0),
                );
                ui.end_row();
            });
    }
}

fn level(ui: &mut Ui, settings: &mut AmbienceSettings) {
    ui.horizontal(|ui| {
        let mut percent = settings.level * 100.0;
        ui.spacing_mut().slider_width = 200.0;
        if ui
            .add(
                egui::Slider::new(&mut percent, 0.0..=100.0)
                    .suffix(" %")
                    .max_decimals(0),
            )
            .changed()
        {
            settings.level = percent / 100.0;
        }
        ui.checkbox(&mut settings.duck, "Quieter while sounds play");
    });
    ui.weak("Even at full level the ambience stays well under the sounds.");
}

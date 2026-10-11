//! The menu bar along the top of the window, and the keyboard shortcuts it
//! documents. Every shortcut is listed here once, so the menus, the key
//! handling and the Keyboard Shortcuts window never disagree.
//!
//! Menu items that a control surface could also send are commands
//! (`olio_control::Command`); the rest, which only concern this window,
//! are [`MenuAction`]s.

use egui::{Button, Key, KeyboardShortcut, Modifiers, Ui};

use crate::prefs::{Preferences, Theme, TileStyle};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuAction {
    AddSound,
    ImportFolder,
    ImportBoard,
    ShowLibraryFolder,
    NewSet,
    RenameSet,
    RemoveSet,
    RenameTile,
    RemoveTile,
    /// Apply the Polish preset to the selected tile, or remove it.
    PolishSelected,
    /// Apply the Polish preset to every tile of the front set.
    PolishSet,
    /// Open the window that edits the Polish preset.
    EditPolish,
    /// Open the emoji picker for the selected tile.
    ChooseEmoji,
    /// Copy the selected tile into the next free slot.
    DuplicateTile,
    /// Move the selected tile one slot over, swapping with a tile there.
    MoveTile(Direction),
    /// Undo the last change to the board.
    Undo,
    /// Remove it again.
    Redo,
    ToggleSpectrograms,
    ToggleSets,
    ToggleInspector,
    Theme(Theme),
    TileStyle(TileStyle),
    PlaySelected,
    StopAll,
    FadeAll,
    ToggleMute,
    SetVolume(f32),
    ToggleClickPlays,
    Move(Direction),
    ShowShortcuts,
    OpenWebsite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

const CMD: Modifiers = Modifiers::COMMAND;
const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
const SHIFT: Modifiers = Modifiers::SHIFT;
const NONE: Modifiers = Modifiers::NONE;

/// Every shortcut. The first entry for an action is the one the menus
/// show; later ones are aliases (the Delete key beside Backspace).
pub const SHORTCUTS: &[(MenuAction, KeyboardShortcut)] = &[
    (MenuAction::AddSound, KeyboardShortcut::new(CMD, Key::O)),
    (
        MenuAction::ImportFolder,
        KeyboardShortcut::new(CMD_SHIFT, Key::O),
    ),
    (MenuAction::NewSet, KeyboardShortcut::new(CMD, Key::N)),
    (
        MenuAction::RemoveTile,
        KeyboardShortcut::new(NONE, Key::Backspace),
    ),
    (
        MenuAction::RemoveTile,
        KeyboardShortcut::new(NONE, Key::Delete),
    ),
    (
        MenuAction::PolishSelected,
        KeyboardShortcut::new(CMD, Key::P),
    ),
    (MenuAction::ChooseEmoji, KeyboardShortcut::new(CMD, Key::E)),
    (MenuAction::Undo, KeyboardShortcut::new(CMD, Key::Z)),
    (MenuAction::Redo, KeyboardShortcut::new(CMD_SHIFT, Key::Z)),
    (
        MenuAction::ToggleSets,
        KeyboardShortcut::new(CMD_SHIFT, Key::S),
    ),
    (
        MenuAction::ToggleInspector,
        KeyboardShortcut::new(CMD_SHIFT, Key::I),
    ),
    (
        MenuAction::PlaySelected,
        KeyboardShortcut::new(NONE, Key::Enter),
    ),
    (MenuAction::StopAll, KeyboardShortcut::new(CMD, Key::Escape)),
    (MenuAction::FadeAll, KeyboardShortcut::new(NONE, Key::Minus)),
    (
        MenuAction::ToggleMute,
        KeyboardShortcut::new(CMD_SHIFT, Key::M),
    ),
    (
        MenuAction::ShowShortcuts,
        KeyboardShortcut::new(CMD, Key::Slash),
    ),
    (
        MenuAction::Move(Direction::Up),
        KeyboardShortcut::new(NONE, Key::ArrowUp),
    ),
    (
        MenuAction::Move(Direction::Down),
        KeyboardShortcut::new(NONE, Key::ArrowDown),
    ),
    (
        MenuAction::Move(Direction::Left),
        KeyboardShortcut::new(NONE, Key::ArrowLeft),
    ),
    (
        MenuAction::Move(Direction::Right),
        KeyboardShortcut::new(NONE, Key::ArrowRight),
    ),
    (
        MenuAction::DuplicateTile,
        KeyboardShortcut::new(CMD, Key::D),
    ),
    (
        MenuAction::MoveTile(Direction::Up),
        KeyboardShortcut::new(SHIFT, Key::ArrowUp),
    ),
    (
        MenuAction::MoveTile(Direction::Down),
        KeyboardShortcut::new(SHIFT, Key::ArrowDown),
    ),
    (
        MenuAction::MoveTile(Direction::Left),
        KeyboardShortcut::new(SHIFT, Key::ArrowLeft),
    ),
    (
        MenuAction::MoveTile(Direction::Right),
        KeyboardShortcut::new(SHIFT, Key::ArrowRight),
    ),
];

pub fn shortcut_for(action: MenuAction) -> Option<KeyboardShortcut> {
    SHORTCUTS
        .iter()
        .find(|(candidate, _)| *candidate == action)
        .map(|(_, shortcut)| *shortcut)
}

/// The shortcut as the platform writes it: "⌘O" on macOS, "Ctrl+O" on
/// Windows.
pub fn shortcut_text(ctx: &egui::Context, action: MenuAction) -> String {
    shortcut_for(action).map_or_else(String::new, |shortcut| ctx.format_shortcut(&shortcut))
}

/// The actions whose shortcuts were pressed this frame, consuming the key
/// presses. While a text field has focus only shortcuts with Command
/// (Ctrl) count, so typing never plays, moves or removes anything.
pub fn pressed(ctx: &egui::Context) -> Vec<MenuAction> {
    let typing = ctx.text_edit_focused();
    ctx.input_mut(|input| {
        checking_order()
            .into_iter()
            // A text field keeps its own undo and redo.
            .filter(|(action, _)| {
                !(typing && matches!(action, MenuAction::Undo | MenuAction::Redo))
            })
            .filter(|(_, shortcut)| !typing || shortcut.modifiers.command)
            .filter(|(_, shortcut)| input.consume_shortcut(shortcut))
            .map(|(action, _)| action)
            .collect()
    })
}

/// The shortcuts with the most modifiers first. egui lets ⌘Z also match a
/// press of ⇧⌘Z, so ⇧⌘Z must be checked (and its key press consumed)
/// before ⌘Z gets the chance.
fn checking_order() -> Vec<(MenuAction, KeyboardShortcut)> {
    let mut order = SHORTCUTS.to_vec();
    order.sort_by_key(|(_, shortcut)| {
        let modifiers = shortcut.modifiers;
        std::cmp::Reverse(
            u8::from(modifiers.shift) + u8::from(modifiers.alt) + u8::from(modifiers.command),
        )
    });
    order
}

/// What the menus need to know to enable their items.
pub struct MenuState<'a> {
    pub prefs: &'a Preferences,
    pub tile_selected: bool,
    /// The selected tile already has the Polish preset.
    pub tile_polished: bool,
    pub set_has_tiles: bool,
    /// What Undo and Redo would do: "Remove “Applause”".
    pub undo: Option<String>,
    pub redo: Option<String>,
    pub set_removable: bool,
    pub playing: bool,
}

pub fn bar(ui: &mut Ui, state: &MenuState) -> Vec<MenuAction> {
    let mut actions = Vec::new();
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            item(ui, "Add Sound…", MenuAction::AddSound, true, &mut actions);
            item(
                ui,
                "Import Folder as Set…",
                MenuAction::ImportFolder,
                true,
                &mut actions,
            );
            item(
                ui,
                "Import Board…",
                MenuAction::ImportBoard,
                true,
                &mut actions,
            );
            ui.separator();
            item(ui, "New Set", MenuAction::NewSet, true, &mut actions);
            item(ui, "Rename Set", MenuAction::RenameSet, true, &mut actions);
            item(
                ui,
                "Remove Set…",
                MenuAction::RemoveSet,
                state.set_removable,
                &mut actions,
            );
            ui.separator();
            item(
                ui,
                "Show Library Folder",
                MenuAction::ShowLibraryFolder,
                true,
                &mut actions,
            );
        });
        ui.menu_button("Edit", |ui| {
            let undo = state
                .undo
                .as_ref()
                .map_or_else(|| "Undo".to_owned(), |what| format!("Undo {what}"));
            item(
                ui,
                &undo,
                MenuAction::Undo,
                state.undo.is_some(),
                &mut actions,
            );
            let redo = state
                .redo
                .as_ref()
                .map_or_else(|| "Redo".to_owned(), |what| format!("Redo {what}"));
            item(
                ui,
                &redo,
                MenuAction::Redo,
                state.redo.is_some(),
                &mut actions,
            );
        });
        ui.menu_button("Tile", |ui| {
            let selected = state.tile_selected;
            item(ui, "Rename", MenuAction::RenameTile, selected, &mut actions);
            item(
                ui,
                "Choose Emoji…",
                MenuAction::ChooseEmoji,
                selected,
                &mut actions,
            );
            ui.separator();
            item(
                ui,
                "Duplicate",
                MenuAction::DuplicateTile,
                selected,
                &mut actions,
            );
            ui.add_enabled_ui(selected, |ui| {
                ui.menu_button("Move", |ui| {
                    for (direction, label) in [
                        (Direction::Up, "Up"),
                        (Direction::Down, "Down"),
                        (Direction::Left, "Left"),
                        (Direction::Right, "Right"),
                    ] {
                        item(
                            ui,
                            label,
                            MenuAction::MoveTile(direction),
                            true,
                            &mut actions,
                        );
                    }
                });
            });
            ui.separator();
            let polish = if state.tile_polished {
                "Remove Polish"
            } else {
                "Polish"
            };
            item(
                ui,
                polish,
                MenuAction::PolishSelected,
                selected,
                &mut actions,
            );
            item(
                ui,
                "Polish All Tiles in Set",
                MenuAction::PolishSet,
                state.set_has_tiles,
                &mut actions,
            );
            ui.separator();
            item(ui, "Remove", MenuAction::RemoveTile, selected, &mut actions);
        });
        ui.menu_button("Playback", |ui| {
            item(
                ui,
                "Play or Stop Selected Tile",
                MenuAction::PlaySelected,
                state.tile_selected,
                &mut actions,
            );
            ui.separator();
            item(
                ui,
                "Fade All",
                MenuAction::FadeAll,
                state.playing,
                &mut actions,
            );
            item(
                ui,
                "Stop All",
                MenuAction::StopAll,
                state.playing,
                &mut actions,
            );
            ui.separator();
            item(
                ui,
                "Polish Preset…",
                MenuAction::EditPolish,
                true,
                &mut actions,
            );
            ui.separator();
            toggle(
                ui,
                "Mute",
                state.prefs.muted,
                MenuAction::ToggleMute,
                &mut actions,
            );
            toggle(
                ui,
                "Single Click Plays Tiles",
                state.prefs.click_plays,
                MenuAction::ToggleClickPlays,
                &mut actions,
            );
        });
        ui.menu_button("View", |ui| {
            toggle(
                ui,
                "Sets",
                state.prefs.show_sets,
                MenuAction::ToggleSets,
                &mut actions,
            );
            toggle(
                ui,
                "Inspector",
                state.prefs.show_inspector,
                MenuAction::ToggleInspector,
                &mut actions,
            );
            ui.separator();
            for theme in Theme::ALL {
                toggle(
                    ui,
                    theme.name(),
                    state.prefs.theme == theme,
                    MenuAction::Theme(theme),
                    &mut actions,
                );
            }
            ui.separator();
            toggle(
                ui,
                "Spectrograms",
                state.prefs.show_spectrograms,
                MenuAction::ToggleSpectrograms,
                &mut actions,
            );
            for style in TileStyle::ALL {
                toggle(
                    ui,
                    style.name(),
                    state.prefs.tile_style == style,
                    MenuAction::TileStyle(style),
                    &mut actions,
                );
            }
            ui.separator();
            egui::gui_zoom::zoom_menu_buttons(ui);
        });
        ui.menu_button("Help", |ui| {
            item(
                ui,
                "Keyboard Shortcuts",
                MenuAction::ShowShortcuts,
                true,
                &mut actions,
            );
            item(
                ui,
                "Olio on GitHub",
                MenuAction::OpenWebsite,
                true,
                &mut actions,
            );
        });
    });
    actions
}

fn item(
    ui: &mut Ui,
    label: &str,
    action: MenuAction,
    enabled: bool,
    actions: &mut Vec<MenuAction>,
) {
    let button = Button::new(label).shortcut_text(shortcut_text(ui.ctx(), action));
    if ui.add_enabled(enabled, button).clicked() {
        actions.push(action);
        ui.close();
    }
}

/// An item that is on or off; shown highlighted while on.
fn toggle(ui: &mut Ui, label: &str, on: bool, action: MenuAction, actions: &mut Vec<MenuAction>) {
    let label = if on {
        format!("✔ {label}")
    } else {
        format!("    {label}")
    };
    if ui.add(button(ui, &label, action)).clicked() {
        actions.push(action);
        ui.close();
    }
}

/// A menu item with its shortcut on the right, if it has one.
fn button<'a>(ui: &Ui, label: &str, action: MenuAction) -> Button<'a> {
    let button = Button::new(label.to_owned());
    match shortcut_for(action) {
        Some(shortcut) => button.shortcut_text(ui.ctx().format_shortcut(&shortcut)),
        None => button,
    }
}

/// The Keyboard Shortcuts window. Returns false once it is closed.
pub fn shortcuts_window(ctx: &egui::Context, open: &mut bool) {
    let row = |ui: &mut Ui, keys: String, what: &str| {
        ui.label(egui::RichText::new(keys).monospace());
        ui.label(what);
        ui.end_row();
    };
    egui::Window::new("Keyboard Shortcuts")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            egui::Grid::new("olio-shortcuts")
                .num_columns(2)
                .spacing([18.0, 6.0])
                .show(ui, |ui| {
                    let alt = if cfg!(target_os = "macos") {
                        "⌥"
                    } else {
                        "Alt+"
                    };
                    row(
                        ui,
                        "1–0  Q–P  A–;  Z–/".to_owned(),
                        "Play or stop the tile at that key",
                    );
                    row(ui, format!("{alt}key"), "The same keys for rows 5 to 8");
                    row(ui, "Esc (hold)".to_owned(), "Stop all sounds");
                    row(ui, "Esc".to_owned(), "Clear the selection");
                    for (action, what) in [
                        (MenuAction::StopAll, "Stop all sounds"),
                        (MenuAction::FadeAll, "Fade all sounds out"),
                        (MenuAction::ToggleMute, "Mute or unmute"),
                        (MenuAction::PlaySelected, "Play or stop the selected tile"),
                        (MenuAction::Move(Direction::Up), "Select the tile above"),
                        (MenuAction::Move(Direction::Down), "Select the tile below"),
                        (
                            MenuAction::Move(Direction::Left),
                            "Select the tile to the left",
                        ),
                        (
                            MenuAction::Move(Direction::Right),
                            "Select the tile to the right",
                        ),
                        (MenuAction::RemoveTile, "Remove the selected tile"),
                        (MenuAction::DuplicateTile, "Duplicate the selected tile"),
                        (
                            MenuAction::MoveTile(Direction::Left),
                            "Move the selected tile (with any arrow)",
                        ),
                        (
                            MenuAction::PolishSelected,
                            "Polish the selected tile, or unpolish it",
                        ),
                        (
                            MenuAction::ChooseEmoji,
                            "Choose an emoji for the selected tile",
                        ),
                        (MenuAction::Undo, "Undo the last removal"),
                        (MenuAction::Redo, "Redo it"),
                        (MenuAction::AddSound, "Add sounds"),
                        (MenuAction::ImportFolder, "Import a folder as a set"),
                        (MenuAction::NewSet, "New set"),
                        (MenuAction::ToggleSets, "Show or hide the sets"),
                        (MenuAction::ToggleInspector, "Show or hide the inspector"),
                        (MenuAction::ShowShortcuts, "This window"),
                    ] {
                        row(ui, shortcut_text(ctx, action), what);
                    }
                    let set_key = ctx.format_shortcut(&KeyboardShortcut::new(CMD, Key::Num1));
                    row(ui, format!("{set_key} … 9"), "Bring a set to the front");
                    row(ui, "Double-click a set".to_owned(), "Rename it");
                    row(
                        ui,
                        "Drag a tile".to_owned(),
                        "Move it; onto another tile to swap them",
                    );
                    row(
                        ui,
                        "Drag a tile onto a set".to_owned(),
                        "Move it to that set",
                    );
                    let copy = if cfg!(target_os = "macos") {
                        "⌥"
                    } else {
                        "Ctrl+"
                    };
                    row(ui, format!("{copy}drag a tile"), "Drop a copy of it");
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_action_with_a_shortcut_lists_it_first() {
        for (action, shortcut) in SHORTCUTS {
            let first = shortcut_for(*action).unwrap();
            if *shortcut != first {
                // An alias: must share the key's modifiers so it reads the same.
                assert_eq!(shortcut.modifiers, first.modifiers, "{action:?}");
            }
        }
    }

    #[test]
    fn shortcuts_with_more_modifiers_are_checked_first() {
        let order = checking_order();
        let position = |action| order.iter().position(|(a, _)| *a == action).unwrap();
        assert!(position(MenuAction::Redo) < position(MenuAction::Undo));
        assert!(position(MenuAction::ImportFolder) < position(MenuAction::AddSound));
    }

    #[test]
    fn bare_keys_never_collide_with_tile_keys() {
        for (action, shortcut) in SHORTCUTS {
            if shortcut.modifiers == NONE {
                let name = format!("{:?}", shortcut.logical_key);
                assert!(
                    crate::keys::tile_char(shortcut.logical_key).is_none(),
                    "{action:?} uses tile key {name}"
                );
            }
        }
    }
}

//! The list of sets down the left of the window.

use std::collections::HashSet;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align, Align2, FontId, Key, KeyboardShortcut, Layout, Modifiers, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, Ui, Vec2, WidgetInfo, WidgetType,
};
use olio_library::{Library, Set, SetId, TileId};

use crate::{grid, keys};

/// How long a dragged tile is held over a set before the set opens.
const SPRING_DELAY: f64 = 0.6;

/// Which set a dragged tile is held over, and since when.
fn spring_id() -> egui::Id {
    egui::Id::new("olio-spring")
}

pub enum SidebarAction {
    Select(SetId),
    StartRename(SetId),
    Rename(SetId, String),
    CancelRename,
    NewSet,
    RemoveSet(SetId),
    /// Give every tile of the set the Polish preset.
    Polish(SetId),
    /// A tile dragged and held over a set: bring the set to the front so
    /// the tile can be dropped on one of its slots.
    SpringOpen(SetId),
    /// A tile dragged from the grid and dropped on a set: moved there, or
    /// copied with the copy modifier.
    DropTile {
        tile: TileId,
        set: SetId,
        copy: bool,
    },
}

/// A set name being edited in place.
pub struct Rename {
    pub set: SetId,
    pub text: String,
    focused: bool,
}

impl Rename {
    pub fn new(set: &Set) -> Self {
        Self {
            set: set.id,
            text: set.name.clone(),
            focused: false,
        }
    }
}

pub fn show(
    ui: &mut Ui,
    library: &Library,
    current: SetId,
    sounding: &HashSet<SetId>,
    rename: &mut Option<Rename>,
) -> Vec<SidebarAction> {
    let mut actions = Vec::new();
    // A tile being dragged, and the set it is in: every other set can
    // take it.
    let dragged = egui::DragAndDrop::payload::<TileId>(ui.ctx())
        .map(|id| *id)
        .and_then(|id| library.tile(id).map(|(set, _)| (id, set.id)));
    let now = ui.input(|input| input.time);
    let spring: Option<(SetId, f64)> =
        dragged.and_then(|_| ui.ctx().data(|data| data.get_temp(spring_id())));
    let mut held_over = None;
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let new_set = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
            if ui
                .button("+ New Set")
                .on_hover_text(format!(
                    "Add a set ({})",
                    ui.ctx().format_shortcut(&new_set)
                ))
                .clicked()
            {
                actions.push(SidebarAction::NewSet);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let removable = library.sets.len() > 1;
                let remove = ui
                    .add_enabled(
                        removable,
                        egui::Button::new("−").min_size(Vec2::new(32.0, 0.0)),
                    )
                    .on_hover_text("Remove the front set…");
                if remove.clicked() {
                    actions.push(SidebarAction::RemoveSet(current));
                }
            });
        });
        ui.separator();
        ui.with_layout(Layout::top_down(Align::Min), |ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("Sets").strong().size(15.0));
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    for (index, set) in library.sets.iter().enumerate() {
                        let row = Row {
                            set,
                            number: index + 1,
                            selected: set.id == current,
                            sounding: sounding.contains(&set.id),
                            drop: dragged
                                .filter(|&(_, from)| from != set.id)
                                .map(|(tile, _)| tile),
                            // Opening: how far along the hold is, for any
                            // set but the one in front.
                            spring: (dragged.is_some() && set.id != current).then(|| {
                                spring.filter(|&(held, _)| held == set.id).map_or(
                                    0.0,
                                    |(_, since)| {
                                        ((now - since) / SPRING_DELAY).clamp(0.0, 1.0) as f32
                                    },
                                )
                            }),
                        };
                        match rename.as_mut().filter(|rename| rename.set == set.id) {
                            Some(rename) => rename_row(ui, rename, &mut actions),
                            None => {
                                if set_row(ui, &row, &mut actions) && row.spring.is_some() {
                                    held_over = Some(set.id);
                                }
                            }
                        }
                    }
                });
        });
    });
    spring_timer(ui, dragged.is_some(), held_over, spring, now, &mut actions);
    actions
}

/// Opens the set a dragged tile has been held over for long enough, and
/// keeps the window redrawing while the hold lasts.
fn spring_timer(
    ui: &Ui,
    dragging: bool,
    held_over: Option<SetId>,
    spring: Option<(SetId, f64)>,
    now: f64,
    actions: &mut Vec<SidebarAction>,
) {
    let ctx = ui.ctx();
    match held_over.filter(|_| dragging) {
        Some(set) => match spring.filter(|&(held, _)| held == set) {
            Some((_, since)) if now - since >= SPRING_DELAY => {
                actions.push(SidebarAction::SpringOpen(set));
                ctx.data_mut(|data| data.remove::<(SetId, f64)>(spring_id()));
            }
            Some(_) => ctx.request_repaint(),
            None => {
                ctx.data_mut(|data| data.insert_temp(spring_id(), (set, now)));
                ctx.request_repaint();
            }
        },
        None => ctx.data_mut(|data| data.remove::<(SetId, f64)>(spring_id())),
    }
}

struct Row<'a> {
    set: &'a Set,
    number: usize,
    selected: bool,
    sounding: bool,
    /// The tile being dragged, if it can be dropped on this set.
    drop: Option<TileId>,
    /// While a tile is dragged and this set is not in front: how far the
    /// hold that opens it has got, 0 to 1.
    spring: Option<f32>,
}

const ROW_HEIGHT: f32 = 28.0;

/// Draws a set's row; true if a dragged tile is over it.
fn set_row(ui: &mut Ui, row: &Row, actions: &mut Vec<SidebarAction>) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            true,
            row.selected,
            &row.set.name,
        )
    });
    // `hovered` is false while something is dragged; the pointer still
    // counts as inside.
    let drop_here = row.drop.is_some() && response.contains_pointer();
    let held_here = row.spring.is_some() && response.contains_pointer();
    let copying = ui.input(|input| grid::copy_modifier(input.modifiers));
    if let (Some(tile), Some((pos, modifiers))) = (row.drop, grid::release(ui))
        && rect.contains(pos)
    {
        actions.push(SidebarAction::DropTile {
            tile,
            set: row.set.id,
            copy: grid::copy_modifier(modifiers),
        });
    }
    let visuals = ui.visuals();
    let painter = ui.painter();
    if drop_here {
        painter.rect_filled(rect, 6, visuals.selection.bg_fill.gamma_multiply(0.45));
        painter.rect_stroke(
            rect,
            6,
            Stroke::new(2.0, visuals.selection.stroke.color),
            StrokeKind::Inside,
        );
    } else if row.selected {
        painter.rect_filled(rect, 6, visuals.selection.bg_fill);
    } else if response.hovered() || held_here {
        painter.rect_filled(rect, 6, visuals.widgets.hovered.weak_bg_fill);
    }
    if held_here && let Some(progress) = row.spring.filter(|&progress| progress > 0.0) {
        // The row fills from the left while the hold opens the set.
        let fill = Rect::from_min_size(rect.min, Vec2::new(rect.width() * progress, rect.height()));
        painter.rect_filled(fill, 6, visuals.selection.bg_fill.gamma_multiply(0.85));
    }
    let text = if row.selected {
        visuals.selection.stroke.color
    } else {
        visuals.text_color()
    };

    let mut right = rect.right() - 8.0;
    if drop_here {
        // Instead of the shortcut, what dropping will do.
        let drawn = painter.text(
            Pos2::new(right, rect.center().y),
            Align2::RIGHT_CENTER,
            if copying { "copy here" } else { "move here" },
            FontId::proportional(12.5),
            visuals.strong_text_color(),
        );
        right = drawn.left() - 6.0;
    } else if let Some(key) = keys::set_key(row.number) {
        let shortcut = ui
            .ctx()
            .format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, key));
        let drawn = painter.text(
            Pos2::new(right, rect.center().y),
            Align2::RIGHT_CENTER,
            shortcut,
            FontId::proportional(12.5),
            visuals.weak_text_color(),
        );
        right = drawn.left() - 6.0;
    }
    if row.sounding {
        let drawn = painter.text(
            Pos2::new(right, rect.center().y),
            Align2::RIGHT_CENTER,
            "🔊",
            FontId::proportional(12.0),
            text,
        );
        right = drawn.left() - 4.0;
    }
    let left = rect.left() + 10.0;
    let mut name =
        LayoutJob::simple_singleline(row.set.name.clone(), FontId::proportional(14.0), text);
    name.wrap = TextWrapping::truncate_at_width((right - left).max(10.0));
    let name = painter.layout_job(name);
    painter.galley(
        Pos2::new(left, rect.center().y - name.size().y / 2.0),
        name,
        text,
    );

    if response.double_clicked() {
        actions.push(SidebarAction::StartRename(row.set.id));
    } else if response.clicked() || response.secondary_clicked() {
        actions.push(SidebarAction::Select(row.set.id));
    }
    response.context_menu(|ui| {
        if ui.button("Rename").clicked() {
            actions.push(SidebarAction::StartRename(row.set.id));
            ui.close();
        }
        let polishable = !row.set.tiles.is_empty();
        if ui
            .add_enabled(polishable, egui::Button::new("✨ Polish All Tiles"))
            .clicked()
        {
            actions.push(SidebarAction::Polish(row.set.id));
            ui.close();
        }
        ui.separator();
        if ui.button("Remove Set…").clicked() {
            actions.push(SidebarAction::RemoveSet(row.set.id));
            ui.close();
        }
    });
    held_here
}

fn rename_row(ui: &mut Ui, rename: &mut Rename, actions: &mut Vec<SidebarAction>) {
    let edit = egui::TextEdit::singleline(&mut rename.text).desired_width(f32::INFINITY);
    let response = ui.add_sized([ui.available_width(), ROW_HEIGHT], edit);
    if !rename.focused {
        response.request_focus();
        rename.focused = true;
    }
    if ui.input(|input| input.key_pressed(Key::Escape)) {
        actions.push(SidebarAction::CancelRename);
    } else if response.lost_focus() {
        actions.push(SidebarAction::Rename(rename.set, rename.text.clone()));
    }
}

//! The window's look: soft UI. One matte surface, lit from the top left,
//! with every part either raised out of it or pressed into it: a raised
//! part casts a light shadow up-left and a dark one down-right; an inset
//! part has the two inside its edges, the other way round. The colours
//! come from the palette (`theme.rs`), installed into the egui context with
//! [`install`] and read back with [`palette`]. The rules are in
//! `docs/DESIGN.md`; change that and this together.
//!
//! Everything here paints with egui's own shapes: gradients are meshes with
//! coloured vertices, soft shadows are blurred rectangles, so the window
//! scales with the zoom and needs no image assets. Ported from Toca's
//! skin (and through it Sala's); Olio adds the tile, the chip, the slider
//! row, the wide button, and the section with its More.

use std::ops::RangeInclusive;

use egui::accesskit::{Action, ActionData};
use egui::epaint::{RectShape, Shadow};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, EventFilter, FontId, Id, Key, Mesh, Pos2, Rect,
    Response, Sense, Shape, Stroke, StrokeKind, TextureId, Ui, Vec2, WidgetInfo, WidgetType, pos2,
    vec2,
};

use crate::theme::{Mode, Palette};

// --- The palette ----------------------------------------------------------

fn palette_id() -> Id {
    Id::new("olio-palette")
}

/// The palette the window draws with, from the egui context.
pub fn palette(ctx: &egui::Context) -> Palette {
    ctx.data(|d| d.get_temp(palette_id()))
        .unwrap_or_else(|| Palette::of(Mode::of(ctx.theme())))
}

/// Installs the palette for the theme the context resolves to (the
/// preference, or the system's) whenever it changes. Called at the top of
/// every frame.
pub fn sync(ctx: &egui::Context) {
    let mode = Mode::of(ctx.theme());
    let installed: Option<Palette> = ctx.data(|d| d.get_temp(palette_id()));
    if installed.is_none_or(|p| p.mode != mode) {
        install(ctx, Palette::of(mode));
    }
}

/// Puts `palette` in the context: every part drawn from now on uses it,
/// and egui's own widgets (menus, fields, drop-downs) take its colours.
pub fn install(ctx: &egui::Context, palette: Palette) {
    ctx.data_mut(|d| d.insert_temp(palette_id(), palette));
    ctx.set_visuals_of(palette.mode.theme(), visuals(&palette));
    ctx.all_styles_mut(|style| {
        style.spacing.button_padding = vec2(12.0, 6.0);
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.interact_size.y = 28.0;
        style.visuals.slider_trailing_fill = true;
    });
}

/// egui's style from the palette, for the widgets egui draws itself.
fn visuals(p: &Palette) -> egui::Visuals {
    let mut v = match p.mode {
        Mode::Light => egui::Visuals::light(),
        Mode::Dark => egui::Visuals::dark(),
    };
    v.panel_fill = p.surface;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.sunken;
    v.faint_bg_color = p.sunken;
    v.code_bg_color = p.sunken;
    // Fields paint their own inset (`field`); egui's fill stays out of it.
    v.text_edit_bg_color = Some(Color32::TRANSPARENT);
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.text_weak);
    v.selection.bg_fill = p.accent_dim;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.accent;
    v.error_fg_color = p.alarm;
    v.window_corner_radius = CornerRadius::same(PANEL_RADIUS);
    v.menu_corner_radius = CornerRadius::same(BUTTON_RADIUS);
    v.window_stroke = Stroke::NONE;
    v.slider_trailing_fill = true;
    v.window_shadow = shadow(p, Elevation::Panel, Side::Dark);
    v.popup_shadow = shadow(p, Elevation::Low, Side::Dark);
    let edge = Stroke::new(1.0, p.shade.gamma_multiply(0.45));
    for (w, fill, weak) in [
        (&mut v.widgets.noninteractive, p.surface, p.surface),
        (
            &mut v.widgets.inactive,
            p.surface.lerp_to_gamma(p.light, 0.3),
            p.surface,
        ),
        (
            &mut v.widgets.hovered,
            p.surface.lerp_to_gamma(p.light, 0.5),
            p.surface.lerp_to_gamma(p.light, 0.3),
        ),
        (&mut v.widgets.active, p.sunken, p.sunken),
        (&mut v.widgets.open, p.sunken, p.sunken),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = weak;
        w.corner_radius = CornerRadius::same(READOUT_RADIUS);
        w.bg_stroke = edge;
        w.fg_stroke = Stroke::new(1.0, p.text);
        w.expansion = 0.0;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.groove);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.light);
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    v
}

// --- Tokens ---------------------------------------------------------------

/// Corner radii, by part: soft, so a part reads as pushed out of the
/// surface rather than cut from it.
pub const PANEL_RADIUS: u8 = 16;
pub const WELL_RADIUS: u8 = 10;
pub const BUTTON_RADIUS: u8 = 9;
pub const READOUT_RADIUS: u8 = 7;
/// A hovered row: a section's header.
pub const ROW_RADIUS: u8 = 5;

/// How far a raised part stands off the surface: the windows most, the
/// small parts least. The shadows' offset and blur follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elevation {
    Panel,
    Low,
    Thumb,
}

impl Elevation {
    fn offset(self) -> i8 {
        match self {
            Elevation::Panel => 6,
            Elevation::Low => 3,
            Elevation::Thumb => 2,
        }
    }

    fn blur(self) -> u8 {
        match self {
            Elevation::Panel => 16,
            Elevation::Low => 8,
            Elevation::Thumb => 5,
        }
    }
}

/// The light comes from the top left: a raised part's light shadow falls
/// up and left, its dark shadow down and right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Light,
    Dark,
}

/// How solid the two shadows paint.
const LIGHT_SHADOW_ALPHA: f32 = 0.85;
const DARK_SHADOW_ALPHA: f32 = 0.7;
/// How deep an inset's inner shadows reach.
const INSET_DEPTH: f32 = 4.0;

fn shadow(p: &Palette, elevation: Elevation, side: Side) -> Shadow {
    let o = elevation.offset();
    match side {
        Side::Light => Shadow {
            offset: [-o, -o],
            blur: elevation.blur(),
            spread: 0,
            color: p.light.gamma_multiply(LIGHT_SHADOW_ALPHA),
        },
        Side::Dark => Shadow {
            offset: [o, o],
            blur: elevation.blur(),
            spread: 0,
            color: p.shade.gamma_multiply(DARK_SHADOW_ALPHA),
        },
    }
}

/// Text sizes: the body, the uppercase legends, the readouts.
pub fn body_font() -> FontId {
    FontId::proportional(13.0)
}

/// 11 pt: the smallest text the design allows.
pub fn legend_font() -> FontId {
    FontId::proportional(11.0)
}

pub fn readout_font() -> FontId {
    FontId::monospace(12.0)
}

/// An icon button, and the glyph on it.
const ICON_BUTTON: Vec2 = vec2(38.0, 30.0);
const ICON_SIZE: f32 = 17.0;
/// A wide button's height: Play, Polish, Generate Track.
pub const WIDE_BUTTON_HEIGHT: f32 = 32.0;
/// A slider's thumb, its track, and the room around it.
const THUMB: f32 = 16.0;
const TRACK: f32 = 8.0;
pub const SLIDER_BREADTH: f32 = 24.0;

// --- Surfaces -------------------------------------------------------------

/// The shapes of a raised part: its two shadows, then its body, the
/// surface a shade lighter where it faces the light.
fn raised_shapes(
    p: &Palette,
    rect: Rect,
    radius: u8,
    elevation: Elevation,
    fill: Color32,
) -> Vec<Shape> {
    vec![
        Shape::from(shadow(p, elevation, Side::Light).as_shape(rect, radius)),
        Shape::from(shadow(p, elevation, Side::Dark).as_shape(rect, radius)),
        lit_fill(
            rect,
            f32::from(radius),
            fill.lerp_to_gamma(p.light, 0.18),
            fill.lerp_to_gamma(p.shade, 0.12),
        ),
    ]
}

/// A part raised out of the surface: a button, a thumb, a tile.
pub fn raised(painter: &egui::Painter, rect: Rect, radius: u8, elevation: Elevation) {
    let p = palette(painter.ctx());
    raised_with(painter, rect, radius, elevation, p.surface);
}

/// A raised part whose body is `fill` rather than the surface (a hovered
/// button is the surface a touch lighter).
pub fn raised_with(
    painter: &egui::Painter,
    rect: Rect,
    radius: u8,
    elevation: Elevation,
    fill: Color32,
) {
    let p = palette(painter.ctx());
    painter.extend(raised_shapes(&p, rect, radius, elevation, fill));
}

/// The shapes of an inset: its floor, then the dark shadow along its top
/// and left inside edges where the rim blocks the light and the light one
/// along the bottom and right. Painted clipped to `rect`.
///
/// Each shadow is a wide blurred stroke inside a rectangle that reaches
/// well past `rect` on the two sides it must not show on, so that only
/// its two wanted bands, `depth` deep and feathered, fall inside the clip.
fn inset_shapes(p: &Palette, rect: Rect, radius: u8, fill: Color32) -> Vec<Shape> {
    let depth = INSET_DEPTH.min(rect.height() / 3.0);
    let reach = depth * 3.0;
    let mut shapes = vec![Shape::from(RectShape::filled(rect, radius, fill))];
    for (side, colour) in [
        (-1.0, p.shade.gamma_multiply(0.6)),
        (1.0, p.light.gamma_multiply(0.5)),
    ] {
        // Up-left for the dark shadow (its bands sit at the top and left),
        // down-right for the light one.
        let near = Vec2::splat(reach * (1.0 + side) / 2.0 + reach);
        let far = Vec2::splat(reach * (1.0 - side) / 2.0 + reach);
        let outer = Rect::from_min_max(rect.min - near, rect.max + far);
        shapes.push(
            RectShape::stroke(
                outer,
                (f32::from(radius) + reach) as u8,
                Stroke::new(reach + depth, colour),
                StrokeKind::Inside,
            )
            .with_blur_width(depth * 2.0)
            .into(),
        );
    }
    shapes
}

/// A part pressed into the surface, `fill` at its floor.
pub fn inset(painter: &egui::Painter, rect: Rect, radius: u8, fill: Color32) {
    let p = palette(painter.ctx());
    inset_on(painter, rect, radius, fill, p.surface);
}

/// An inset in something other than the surface (a key's well in a
/// tinted tile): `around` is what shows past its rounded corners.
///
/// The shadow bands are clipped to `rect`, a rectangle, so they would
/// square off its corners; the corners outside the rounded floor are put
/// back in `around`.
pub fn inset_on(painter: &egui::Painter, rect: Rect, radius: u8, fill: Color32, around: Color32) {
    let p = palette(painter.ctx());
    let clipped = painter.with_clip_rect(rect);
    clipped.extend(inset_shapes(&p, rect, radius, fill));
    clipped.add(Shape::mesh(corners(rect, f32::from(radius), around)));
}

/// The four corners of `rect` outside its rounded outline, as a mesh
/// filled with `colour`.
fn corners(rect: Rect, radius: f32, colour: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    let outline = rounded_outline(rect, radius);
    if outline.len() != 4 * 7 {
        return mesh;
    }
    // `rounded_outline` goes clockwise from the top right corner's arc.
    let points = [
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    for (corner, arc) in points.into_iter().zip(outline.chunks(7)) {
        let at = mesh.vertices.len() as u32;
        mesh.colored_vertex(corner, colour);
        for point in arc {
            mesh.colored_vertex(*point, colour);
        }
        for i in 0..arc.len() as u32 - 1 {
            mesh.add_triangle(at, at + 1 + i, at + 2 + i);
        }
    }
    mesh
}

/// A well: a part pressed into the surface and left its colour (a pressed
/// button, a switch's track).
pub fn well(painter: &egui::Painter, rect: Rect, radius: u8) {
    let p = palette(painter.ctx());
    inset(painter, rect, radius, p.surface);
}

/// A field: egui's text field on an inset of its own.
pub fn field<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let background = ui.painter().add(Shape::Noop);
    let inner = ui.scope(add);
    let rect = inner.response.rect;
    let p = palette(ui.ctx());
    ui.painter().with_clip_rect(rect).set(
        background,
        Shape::Vec(inset_shapes(&p, rect, READOUT_RADIUS, p.sunken)),
    );
    inner.inner
}

// --- Gradients ------------------------------------------------------------

/// The outline of a rounded rectangle, clockwise from the top left.
fn rounded_outline(rect: Rect, radius: f32) -> Vec<Pos2> {
    let r = radius
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0)
        .max(0.0);
    if r <= 0.0 {
        return vec![
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ];
    }
    let corners = [
        (pos2(rect.right() - r, rect.top() + r), -90.0f32),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), 90.0),
        (pos2(rect.left() + r, rect.top() + r), 180.0),
    ];
    let mut points = Vec::with_capacity(4 * 7);
    for (centre, start) in corners {
        for step in 0..=6 {
            let a = (start + 90.0 * step as f32 / 6.0).to_radians();
            points.push(centre + vec2(a.cos(), a.sin()) * r);
        }
    }
    points
}

/// A mesh fanned from `centre` to `outline`, each vertex coloured by
/// `colour`; a linear function of position interpolates exactly.
fn fan(centre: Pos2, outline: &[Pos2], colour: impl Fn(Pos2) -> Color32) -> Mesh {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(centre, colour(centre));
    for p in outline {
        mesh.colored_vertex(*p, colour(*p));
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    mesh
}

/// A rounded rectangle shaded along the light: `near` at its top-left
/// corner, `far` at its bottom-right.
fn lit_fill(rect: Rect, radius: f32, near: Color32, far: Color32) -> Shape {
    let outline = rounded_outline(rect, radius);
    let mix = move |p: Pos2| {
        let across = (p.x - rect.left()) / rect.width().max(1.0);
        let down = (p.y - rect.top()) / rect.height().max(1.0);
        near.lerp_to_gamma(far, ((across + down) / 2.0).clamp(0.0, 1.0))
    };
    Shape::mesh(fan(rect.center(), &outline, mix))
}

// --- Parts ----------------------------------------------------------------

/// A small uppercase label on the surface.
pub fn legend(painter: &egui::Painter, at: Pos2, align: Align2, text: &str) {
    let p = palette(painter.ctx());
    painter.text(at, align, text.to_uppercase(), legend_font(), p.legend);
}

/// A legend as a widget in the layout.
pub fn legend_label(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui.ctx());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_uppercase(), legend_font(), p.legend);
    let (rect, response) = ui.allocate_exact_size(galley.size() + vec2(0.0, 2.0), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), text));
    legend(ui.painter(), rect.left_center(), Align2::LEFT_CENTER, text);
    response
}

/// A lamp: lit, a dot of `colour` with a glow; off, a dimple in the
/// surface tinted with it, so it still reads as a lamp.
pub fn lamp(painter: &egui::Painter, centre: Pos2, radius: f32, colour: Color32, on: bool) {
    let p = palette(painter.ctx());
    if on {
        let halo = Rect::from_center_size(centre, Vec2::splat(radius * 4.4));
        painter.add(
            RectShape::filled(halo, (radius * 2.2) as u8, colour.gamma_multiply(0.35))
                .with_blur_width(radius * 2.5),
        );
        painter.circle_filled(centre, radius, colour);
    } else {
        let edge = (radius * 0.18).max(0.6);
        painter.circle_filled(
            centre + Vec2::splat(edge),
            radius,
            p.light.gamma_multiply(0.7),
        );
        painter.circle_filled(
            centre - Vec2::splat(edge),
            radius,
            p.shade.gamma_multiply(0.7),
        );
        painter.circle_filled(
            centre,
            radius - edge * 0.5,
            p.sunken.lerp_to_gamma(colour, 0.2),
        );
    }
}

/// A readout: monospace text on a small screen.
pub fn readout(painter: &egui::Painter, rect: Rect, text: &str, colour: Color32) {
    let p = palette(painter.ctx());
    inset(painter, rect, READOUT_RADIUS, p.sunken);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        readout_font(),
        colour,
    );
}

/// A readout in the layout, `width` wide, its text in the accent.
pub fn readout_label(ui: &mut Ui, width: f32, text: &str) -> Response {
    let p = palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    readout(ui.painter(), rect, text, p.accent);
    response
}

/// Paints a button's body for its state: raised; level with the surface
/// (inset) while held; inset and tinted when `selected`; flat when the
/// ui is disabled.
fn paint_button(ui: &Ui, response: &Response, rect: Rect, selected: bool) {
    let p = palette(ui.ctx());
    let painter = ui.painter();
    let pressed = response.is_pointer_button_down_on();
    if selected {
        inset(
            painter,
            rect,
            BUTTON_RADIUS,
            p.surface.lerp_to_gamma(p.accent, 0.08),
        );
    } else if !ui.is_enabled() {
        painter.rect_filled(rect, BUTTON_RADIUS, p.surface.lerp_to_gamma(p.sunken, 0.5));
    } else if pressed {
        well(painter, rect, BUTTON_RADIUS);
    } else if response.hovered() {
        raised_with(
            painter,
            rect,
            BUTTON_RADIUS,
            Elevation::Low,
            p.surface.lerp_to_gamma(p.light, 0.25),
        );
    } else {
        raised(painter, rect, BUTTON_RADIUS, Elevation::Low);
    }
}

/// The colour of a button's text for its state.
fn button_text_colour(ui: &Ui, selected: bool) -> Color32 {
    let p = palette(ui.ctx());
    if !ui.is_enabled() {
        p.text_weak
    } else if selected {
        p.accent
    } else {
        p.text
    }
}

/// A push button: raised, pressed in while held, pressed in and lit when
/// `selected` (a polished tile's Polish).
pub fn button(ui: &mut Ui, text: &str, selected: bool) -> Response {
    let p = palette(ui.ctx());
    let padding = ui.spacing().button_padding;
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), body_font(), p.text);
    let size = galley.size() + padding * 2.0 + vec2(2.0, 2.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), selected, text));
    paint_button(ui, &response, rect, selected);
    let colour = button_text_colour(ui, selected);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, colour);
    response
}

/// An icon button: a glyph on a raised button, `label` for assistive
/// technology and the tooltip.
pub fn icon_button(ui: &mut Ui, glyph: &str, label: &str, enabled: bool) -> Response {
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(ICON_BUTTON, Sense::click());
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
        paint_button(ui, &response, rect, false);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(ICON_SIZE),
            button_text_colour(ui, false),
        );
        response.on_hover_text(label)
    })
    .inner
}

/// A tab in a strip: the chosen one is pressed in, lit, and carries a
/// lamp. `label` is what assistive technology and the tests call it.
pub fn tab(ui: &mut Ui, text: &str, label: &str, selected: bool) -> Response {
    let p = palette(ui.ctx());
    let padding = vec2(12.0, 5.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), body_font(), p.text);
    let size = galley.size() + padding * 2.0 + vec2(if selected { 10.0 } else { 0.0 }, 0.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    paint_button(ui, &response, rect, selected);
    if selected {
        lamp(
            ui.painter(),
            pos2(rect.left() + 9.0, rect.center().y),
            2.5,
            p.accent,
            true,
        );
    }
    let colour = button_text_colour(ui, selected);
    let offset = if selected { vec2(5.0, 0.0) } else { Vec2::ZERO };
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0 + offset, galley, colour);
    response
}

/// A button as wide as the space it is in, `WIDE_BUTTON_HEIGHT` tall: the
/// main thing a group does (Play, Polish, Generate Track). `shortcut`, if
/// any, sits at its right in weak text and is part of its label, as egui
/// does with a menu item's shortcut.
pub fn wide_button(ui: &mut Ui, text: &str, shortcut: Option<&str>, selected: bool) -> Response {
    let p = palette(ui.ctx());
    let size = vec2(ui.available_width(), WIDE_BUTTON_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let label = match shortcut {
        Some(shortcut) => format!("{text} {shortcut}"),
        None => text.to_owned(),
    };
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), selected, &label)
    });
    paint_button(ui, &response, rect, selected);
    let colour = button_text_colour(ui, selected);
    let padding = ui.spacing().button_padding.x;
    ui.painter().text(
        pos2(rect.left() + padding, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        body_font(),
        colour,
    );
    if let Some(shortcut) = shortcut {
        ui.painter().text(
            pos2(rect.right() - padding, rect.center().y),
            Align2::RIGHT_CENTER,
            shortcut,
            body_font(),
            p.text_weak,
        );
    }
    response
}

/// A button that takes something away (Remove Tile): raised like any
/// other, lettered in the alarm colour.
pub fn alarm_button(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui.ctx());
    let padding = ui.spacing().button_padding;
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), body_font(), p.alarm);
    let size = galley.size() + padding * 2.0 + vec2(2.0, 2.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), text));
    paint_button(ui, &response, rect, false);
    let colour = if ui.is_enabled() {
        p.alarm
    } else {
        p.text_weak
    };
    ui.painter().galley_with_override_text_color(
        rect.center() - galley.size() / 2.0,
        galley,
        colour,
    );
    response
}

/// How far a chip's text sits from its edges.
const CHIP_PADDING: Vec2 = vec2(9.0, 4.0);
/// A picture on a chip.
const CHIP_PICTURE: f32 = 16.0;

/// A chip: one of a few choices side by side (a mood, a length, an
/// instrument), a small raised button, pressed in and lit when chosen.
pub fn chip(ui: &mut Ui, text: &str, picture: Option<TextureId>, selected: bool) -> Response {
    small_button(ui, text, text, picture, Some(selected))
}

/// A button with a picture before its text (Surprise Me's dice), as small
/// as a chip.
pub fn picture_button(ui: &mut Ui, text: &str, picture: Option<TextureId>) -> Response {
    small_button(ui, text, text, picture, None)
}

/// A chip when `selected` is given, a plain button otherwise.
fn small_button(
    ui: &mut Ui,
    text: &str,
    label: &str,
    picture: Option<TextureId>,
    selected: Option<bool>,
) -> Response {
    let p = palette(ui.ctx());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), body_font(), p.text);
    let picture_width = if picture.is_some() {
        CHIP_PICTURE + 5.0
    } else {
        0.0
    };
    let size = vec2(
        galley.size().x + picture_width + CHIP_PADDING.x * 2.0,
        (galley.size().y + CHIP_PADDING.y * 2.0).max(SLIDER_BREADTH + 2.0),
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| match selected {
        Some(selected) => WidgetInfo::selected(
            WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            label,
        ),
        None => WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), label),
    });
    let selected = selected.unwrap_or(false);
    paint_button(ui, &response, rect, selected);
    let mut left = rect.left() + CHIP_PADDING.x;
    if let Some(texture) = picture {
        let at = Rect::from_min_size(
            pos2(left, rect.center().y - CHIP_PICTURE / 2.0),
            Vec2::splat(CHIP_PICTURE),
        );
        ui.painter().image(
            texture,
            at,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        left += picture_width;
    }
    let colour = button_text_colour(ui, selected);
    ui.painter().galley(
        pos2(left, rect.center().y - galley.size().y / 2.0),
        galley,
        colour,
    );
    response
}

/// A bubble in the toolbar: a tile playing, or the ambience. A small
/// raised part tinted with `colour` and lettered in it, a stop mark, the
/// picture if any, then `text`; known as "■ {text}". A click stops it.
pub fn bubble(ui: &mut Ui, text: &str, picture: Option<TextureId>, colour: Color32) -> Response {
    let p = palette(ui.ctx());
    let mark = ui
        .painter()
        .layout_no_wrap("■".to_owned(), body_font(), colour);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), body_font(), colour);
    let picture_width = if picture.is_some() {
        CHIP_PICTURE + 5.0
    } else {
        0.0
    };
    let gap = 6.0;
    let size = vec2(
        mark.size().x + gap + picture_width + galley.size().x + CHIP_PADDING.x * 2.0,
        SLIDER_BREADTH + 4.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let label = format!("■ {text}");
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &label));
    let fill = if response.hovered() {
        p.surface.lerp_to_gamma(p.light, 0.25)
    } else {
        p.surface
    }
    .lerp_to_gamma(colour, 0.12);
    if response.is_pointer_button_down_on() {
        inset(ui.painter(), rect, BUTTON_RADIUS, fill);
    } else {
        raised_with(ui.painter(), rect, BUTTON_RADIUS, Elevation::Low, fill);
    }
    let mut left = rect.left() + CHIP_PADDING.x;
    let y = rect.center().y;
    let mark_width = mark.size().x;
    ui.painter()
        .galley(pos2(left, y - mark.size().y / 2.0), mark, colour);
    left += mark_width + gap;
    if let Some(texture) = picture {
        let at = Rect::from_min_size(
            pos2(left, y - CHIP_PICTURE / 2.0),
            Vec2::splat(CHIP_PICTURE),
        );
        ui.painter().image(
            texture,
            at,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        left += picture_width;
    }
    ui.painter()
        .galley(pos2(left, y - galley.size().y / 2.0), galley, colour);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The name before a slider in a [`slider_row`] is at least this wide, so
/// the sliders of a group line up.
pub const ROW_NAME_WIDTH: f32 = 88.0;
/// The readout after it.
pub const ROW_READOUT_WIDTH: f32 = 64.0;

/// The name at the left of a row, at least [`ROW_NAME_WIDTH`] wide so the
/// rows of a group line up. Painted, not a widget: the control after it
/// carries the name for assistive technology.
pub fn row_name(ui: &mut Ui, name: &str) {
    let p = palette(ui.ctx());
    let galley = ui
        .painter()
        .layout_no_wrap(name.to_string(), body_font(), p.text);
    let width = galley.size().x.max(ROW_NAME_WIDTH);
    let (rect, _) = ui.allocate_exact_size(vec2(width, SLIDER_BREADTH), Sense::hover());
    let colour = if ui.is_enabled() { p.text } else { p.text_weak };
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        colour,
    );
}

/// A row of the surface: `name` at the left, a slider filling the middle,
/// and its value in a readout at the right, where the eye lands. The
/// slider is the widget, known to assistive technology and the tests as
/// `label`; the name is painted, so it is not a second node by the same
/// name. Returns the slider's response, for its hover text.
pub fn slider_row(
    ui: &mut Ui,
    name: &str,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    readout: &str,
) -> Response {
    ui.horizontal(|ui| {
        row_name(ui, name);
        let spacing = ui.spacing().item_spacing.x;
        let length = (ui.available_width() - ROW_READOUT_WIDTH - spacing).max(60.0);
        let response = slider(ui, value, range, length, false, label);
        readout_label(ui, ROW_READOUT_WIDTH, readout);
        response
    })
    .inner
}

// --- Sections -------------------------------------------------------------

/// A section header's height, and the gap above it.
const SECTION_HEIGHT: f32 = 28.0;
const SECTION_GAP: f32 = 14.0;

/// A chevron, drawn with lines: a box glyph is what `▾` becomes in egui's
/// default fonts. Points down when `open`, right when not (`up` points it
/// up instead of down, for a More that is open).
fn chevron(painter: &egui::Painter, centre: Pos2, open: bool, up: bool, colour: Color32) {
    let points = match (open, up) {
        (true, false) => vec![
            centre + vec2(-4.0, -2.0),
            centre + vec2(0.0, 2.0),
            centre + vec2(4.0, -2.0),
        ],
        (true, true) => vec![
            centre + vec2(-4.0, 2.0),
            centre + vec2(0.0, -2.0),
            centre + vec2(4.0, 2.0),
        ],
        (false, _) => vec![
            centre + vec2(-2.0, -4.0),
            centre + vec2(2.0, 0.0),
            centre + vec2(-2.0, 4.0),
        ],
    };
    painter.add(Shape::line(points, Stroke::new(1.5, colour)));
}

/// A section's header, and what its body gave back if it was open.
pub struct Disclosed<R> {
    /// The header: its id names the first control in the body that has no
    /// name of its own (`Response::labelled_by`).
    pub header: Response,
    pub inner: Option<R>,
}

/// A section of a panel: an uppercase legend with a chevron, the whole
/// row a toggle, and `body` under it while `open`. A click on the header
/// flips `open`, which the caller keeps (in the preferences). The body is
/// always one scope, open or not, so the ids of everything after it stay
/// put (`docs/EGUI-NOTES.md`).
pub fn section<R>(
    ui: &mut Ui,
    title: &str,
    open: &mut bool,
    body: impl FnOnce(&mut Ui) -> R,
) -> Disclosed<R> {
    section_with(ui, title, open, |ui, _| body(ui))
}

/// A [`section`] whose body is handed its header, to name a control in it
/// that has no name of its own (`Response::labelled_by`).
pub fn section_with<R>(
    ui: &mut Ui,
    title: &str,
    open: &mut bool,
    body: impl FnOnce(&mut Ui, &Response) -> R,
) -> Disclosed<R> {
    let p = palette(ui.ctx());
    ui.add_space(SECTION_GAP);
    let size = vec2(ui.available_width(), SECTION_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *open = !*open;
    }
    let is_open = *open;
    response.widget_info(|| {
        WidgetInfo::selected(
            WidgetType::CollapsingHeader,
            ui.is_enabled(),
            is_open,
            title,
        )
    });
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, ROW_RADIUS, p.surface.lerp_to_gamma(p.light, 0.25));
    }
    legend(
        painter,
        rect.left_center() + vec2(4.0, 0.0),
        Align2::LEFT_CENTER,
        title,
    );
    chevron(
        painter,
        pos2(rect.right() - 12.0, rect.center().y),
        is_open,
        false,
        p.legend,
    );
    let header = response.on_hover_cursor(CursorIcon::PointingHand);
    let inner = ui
        .scope(|ui| {
            if is_open {
                ui.add_space(4.0);
                Some(body(ui, &header))
            } else {
                None
            }
        })
        .inner;
    Disclosed { header, inner }
}

/// A section's More: a small flat toggle at the end of it, "More" while
/// its advanced settings are hidden and "Less" while they show, lit when
/// open. Known as "More in {section}". The caller shows what it reveals,
/// in a scope of its own.
pub fn more(ui: &mut Ui, section: &str, open: &mut bool) -> Response {
    let p = palette(ui.ctx());
    let text = if *open { "Less" } else { "More" };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), body_font(), p.text);
    let size = vec2(galley.size().x + 22.0, SLIDER_BREADTH);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *open = !*open;
    }
    let is_open = *open;
    let label = format!("More in {section}");
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), is_open, &label));
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, ROW_RADIUS, p.surface.lerp_to_gamma(p.light, 0.25));
    }
    let colour = if is_open { p.accent } else { p.text };
    painter.galley(
        pos2(rect.left() + 4.0, rect.center().y - galley.size().y / 2.0),
        galley,
        colour,
    );
    chevron(
        painter,
        pos2(rect.right() - 8.0, rect.center().y),
        true,
        is_open,
        colour,
    );
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(format!("More settings for {}.", section.to_lowercase()))
}

/// A switch: a raised thumb that slides right along an inset track, which
/// lights when on, with `label` beside it. `value` is flipped on a click.
pub fn switch(ui: &mut Ui, label: &str, value: &mut bool) -> Response {
    let p = palette(ui.ctx());
    let track_size = vec2(40.0, 20.0);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), body_font(), p.text);
    let size = vec2(track_size.x + 6.0 + galley.size().x, SLIDER_BREADTH);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *value = !*value;
    }
    let on = *value;
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, ui.is_enabled(), on, label));
    let painter = ui.painter();
    let track = Rect::from_min_size(
        pos2(rect.left(), rect.center().y - track_size.y / 2.0),
        track_size,
    );
    inset(painter, track, 10, if on { p.accent } else { p.groove });
    let thumb_x = if on {
        track.right() - 10.0
    } else {
        track.left() + 10.0
    };
    let thumb = Rect::from_center_size(pos2(thumb_x, track.center().y), Vec2::splat(THUMB));
    let fill = if response.hovered() && ui.is_enabled() {
        p.surface.lerp_to_gamma(p.light, 0.25)
    } else {
        p.surface
    };
    raised_with(painter, thumb, 8, Elevation::Thumb, fill);
    painter.galley(
        pos2(track.right() + 6.0, rect.center().y - galley.size().y / 2.0),
        galley,
        button_text_colour(ui, false),
    );
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A slider: a raised thumb on an inset track, the travel behind it lit.
/// Horizontal, `length` wide, or vertical, `length` tall and reading
/// upwards. A click or a drag sets `value` and marks the response changed;
/// with keyboard focus (Tab gives it) the arrows step it by a hundredth.
pub fn slider(
    ui: &mut Ui,
    value: &mut f32,
    range: RangeInclusive<f32>,
    length: f32,
    vertical: bool,
    label: &str,
) -> Response {
    let p = palette(ui.ctx());
    let size = if vertical {
        vec2(SLIDER_BREADTH, length)
    } else {
        vec2(length, SLIDER_BREADTH)
    };
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let (start, end) = (*range.start(), *range.end());
    let span = (end - start).max(f32::EPSILON);
    // The thumb's centre runs from half a thumb in at one end to the other.
    let (lo, hi) = if vertical {
        (rect.bottom() - THUMB / 2.0, rect.top() + THUMB / 2.0)
    } else {
        (rect.left() + THUMB / 2.0, rect.right() - THUMB / 2.0)
    };
    let at = |v: f32| lo + (hi - lo) * ((v - start) / span).clamp(0.0, 1.0);

    if ui.is_enabled() {
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let along = if vertical { pointer.y } else { pointer.x };
            let fraction = ((along - lo) / (hi - lo)).clamp(0.0, 1.0);
            let wanted = start + fraction * span;
            if wanted != *value {
                *value = wanted;
                response.mark_changed();
            }
        }
        if response.has_focus() {
            // The arrows are this slider's while it has focus, not egui's
            // to move focus with, nor the tile grid's (`menu::keeps_arrows`).
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    response.id,
                    EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                );
            });
            let steps = ui.input(|i| {
                let up = i.num_presses(Key::ArrowRight) + i.num_presses(Key::ArrowUp);
                let down = i.num_presses(Key::ArrowLeft) + i.num_presses(Key::ArrowDown);
                up as i32 - down as i32
            });
            if steps != 0 {
                *value = (*value + steps as f32 * span / 100.0).clamp(start, end);
                response.mark_changed();
            }
        }
        // Assistive technology, and the window tests, set the value or
        // step it as they would egui's own slider.
        let (asked, steps) = ui.input(|i| {
            let asked = i
                .accesskit_action_requests(response.id, Action::SetValue)
                .filter_map(|request| match request.data {
                    Some(ActionData::NumericValue(value)) => Some(value as f32),
                    _ => None,
                })
                .last();
            let steps = i.num_accesskit_action_requests(response.id, Action::Increment) as i32
                - i.num_accesskit_action_requests(response.id, Action::Decrement) as i32;
            (asked, steps)
        });
        if let Some(asked) = asked {
            *value = asked.clamp(start, end);
            response.mark_changed();
        }
        if steps != 0 {
            *value = (*value + steps as f32 * span / 100.0).clamp(start, end);
            response.mark_changed();
        }
    }
    let response = crate::menu::keeps_arrows(response);
    response.widget_info(|| WidgetInfo::slider(ui.is_enabled(), f64::from(*value), label));
    let now = f64::from(*value);
    ui.ctx().accesskit_node_builder(response.id, |builder| {
        builder.set_min_numeric_value(f64::from(start));
        builder.set_max_numeric_value(f64::from(end));
        builder.add_action(Action::SetValue);
        if now < f64::from(end) {
            builder.add_action(Action::Increment);
        }
        if now > f64::from(start) {
            builder.add_action(Action::Decrement);
        }
    });

    let painter = ui.painter();
    let track = if vertical {
        Rect::from_center_size(rect.center(), vec2(TRACK, (lo - hi).abs() + THUMB))
    } else {
        Rect::from_center_size(rect.center(), vec2(hi - lo + THUMB, TRACK))
    };
    inset(painter, track, (TRACK / 2.0) as u8, p.groove);
    let here = at(*value);
    let lit = if ui.is_enabled() { p.accent } else { p.groove };
    let travel = if vertical {
        Rect::from_min_max(
            pos2(track.left(), here),
            pos2(track.right(), track.bottom()),
        )
    } else {
        Rect::from_min_max(track.left_top(), pos2(here, track.bottom()))
    };
    painter
        .with_clip_rect(travel)
        .rect_filled(track, (TRACK / 2.0) as u8, lit);
    let centre = if vertical {
        pos2(rect.center().x, here)
    } else {
        pos2(here, rect.center().y)
    };
    let thumb = Rect::from_center_size(centre, Vec2::splat(THUMB));
    let active = ui.is_enabled() && (response.hovered() || response.dragged());
    let fill = if active {
        p.surface.lerp_to_gamma(p.light, 0.25)
    } else {
        p.surface
    };
    raised_with(painter, thumb, (THUMB / 2.0) as u8, Elevation::Thumb, fill);
    if response.has_focus() {
        painter.circle_stroke(centre, THUMB / 2.0, Stroke::new(1.0, p.accent));
    }
    if ui.is_enabled() {
        response.on_hover_cursor(if vertical {
            CursorIcon::ResizeVertical
        } else {
            CursorIcon::ResizeHorizontal
        })
    } else {
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::luminance;

    #[test]
    fn a_rounded_outline_stays_inside_its_rectangle() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 50.0));
        for p in rounded_outline(rect, 8.0) {
            assert!(rect.expand(1e-3).contains(p), "{p:?}");
        }
    }

    #[test]
    fn an_insets_corners_cover_only_what_lies_outside_its_outline() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(40.0, 20.0));
        let mesh = corners(rect, 10.0, Color32::RED);
        assert_eq!(mesh.vertices.len(), 4 * 8);
        // The middle of the top edge is inside the outline: no corner
        // triangle reaches it.
        let middle = pos2(20.0, 1.0);
        for triangle in mesh.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|n| mesh.vertices[triangle[n] as usize].pos);
            let side = |p: Pos2, q: Pos2| (q - p).x * (middle - p).y - (q - p).y * (middle - p).x;
            let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
            let inside =
                (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0);
            assert!(!inside);
        }
    }

    #[test]
    fn a_raised_part_casts_light_up_left_and_shade_down_right() {
        for mode in [Mode::Light, Mode::Dark] {
            let p = Palette::of(mode);
            for elevation in [Elevation::Panel, Elevation::Low, Elevation::Thumb] {
                let light = shadow(&p, elevation, Side::Light);
                let dark = shadow(&p, elevation, Side::Dark);
                assert!(light.offset[0] < 0 && light.offset[1] < 0);
                assert!(dark.offset[0] > 0 && dark.offset[1] > 0);
                assert_eq!(light.offset, [-dark.offset[0], -dark.offset[1]]);
                assert!(luminance(light.color) > luminance(dark.color));
            }
        }
    }

    #[test]
    fn the_installed_palette_is_the_one_read_back_and_follows_the_theme() {
        let ctx = egui::Context::default();
        ctx.set_theme(egui::Theme::Light);
        sync(&ctx);
        assert_eq!(palette(&ctx), Palette::of(Mode::Light));
        assert_eq!(
            ctx.style_of(egui::Theme::Light).visuals.panel_fill,
            Palette::of(Mode::Light).surface
        );
        ctx.set_theme(egui::Theme::Dark);
        sync(&ctx);
        assert_eq!(palette(&ctx).mode, Mode::Dark);
    }

    /// A slider in a window, as the tests drive it.
    fn slider_harness() -> egui_kittest::Harness<'static, f32> {
        let mut harness = egui_kittest::Harness::new_ui_state(
            |ui, value: &mut f32| {
                sync(ui.ctx());
                slider(ui, value, 0.0..=10.0, 200.0, false, "Level");
            },
            2.0,
        );
        harness.run();
        harness
    }

    #[test]
    fn a_click_on_the_track_sets_the_value_where_it_landed() {
        use egui_kittest::kittest::Queryable;
        let mut harness = slider_harness();
        let node = harness.get_by_label("Level");
        let rect = node.rect();
        let at = pos2(
            rect.left() + THUMB / 2.0 + (rect.width() - THUMB) * 0.75,
            rect.center().y,
        );
        harness.hover_at(at);
        harness.run();
        harness.drag_at(at);
        harness.step();
        harness.drop_at(at);
        harness.run();
        let value = *harness.state();
        assert!((value - 7.5).abs() < 0.2, "{value}");
    }

    #[test]
    fn a_slider_takes_an_accesskit_set_value_and_steps() {
        use egui_kittest::kittest::{NodeT, Queryable};
        let mut harness = slider_harness();
        let node = harness.get_by_label("Level");
        let (target_node, target_tree) = node.accesskit_node().locate();
        harness.event(egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                target_node,
                target_tree,
                action: Action::SetValue,
                data: Some(ActionData::NumericValue(4.0)),
            },
        ));
        harness.run();
        assert_eq!(*harness.state(), 4.0);
        harness.event(egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                target_node,
                target_tree,
                action: Action::Increment,
                data: None,
            },
        ));
        harness.run();
        assert!((*harness.state() - 4.1).abs() < 1e-4);
    }

    #[test]
    fn a_section_header_toggles_and_scopes_its_body() {
        use egui_kittest::kittest::{NodeT, Queryable};
        let mut harness = egui_kittest::Harness::new_ui_state(
            |ui, open: &mut bool| {
                sync(ui.ctx());
                section(ui, "Polish", open, |ui| ui.label("Inside"));
                ui.label("After");
            },
            true,
        );
        harness.run();
        let header = harness.get_by_label("Polish");
        assert_eq!(
            header.accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        harness.get_by_label("Inside");
        let after = harness.get_by_label("After").accesskit_node().id();
        header.click();
        harness.run();
        assert!(!*harness.state());
        assert_eq!(harness.query_all_by_label("Inside").count(), 0);
        assert_eq!(
            harness.get_by_label("After").accesskit_node().id(),
            after,
            "what follows keeps its id"
        );
    }

    #[test]
    fn more_reads_as_more_in_its_section_and_flips() {
        use egui_kittest::kittest::{NodeT, Queryable};
        let mut harness = egui_kittest::Harness::new_ui_state(
            |ui, open: &mut bool| {
                sync(ui.ctx());
                more(ui, "Key and mood", open);
            },
            false,
        );
        harness.run();
        harness.get_by_label("More in Key and mood").click();
        harness.run();
        assert!(*harness.state());
        assert_eq!(
            harness
                .get_by_label("More in Key and mood")
                .accesskit_node()
                .toggled(),
            Some(egui::accesskit::Toggled::True)
        );
    }

    #[test]
    fn a_slider_row_names_its_slider_once() {
        use egui_kittest::kittest::Queryable;
        let mut harness = egui_kittest::Harness::new_ui_state(
            |ui, value: &mut f32| {
                sync(ui.ctx());
                slider_row(ui, "Reverb", "Reverb", value, 0.0..=1.0, "30 %");
            },
            0.3,
        );
        harness.run();
        assert_eq!(harness.query_all_by_label("Reverb").count(), 1);
        harness.get_by_role_and_label(egui::accesskit::Role::Slider, "Reverb");
    }

    #[test]
    fn a_switch_flips_on_a_click_and_reads_as_a_checkbox() {
        use egui_kittest::kittest::{NodeT, Queryable};
        let mut harness = egui_kittest::Harness::new_ui_state(
            |ui, on: &mut bool| {
                sync(ui.ctx());
                switch(ui, "On", on);
            },
            false,
        );
        harness.run();
        let node = harness.get_by_label("On");
        assert_eq!(
            node.accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::False)
        );
        node.click();
        harness.run();
        assert!(*harness.state());
        assert_eq!(
            harness.get_by_label("On").accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::True)
        );
    }
}

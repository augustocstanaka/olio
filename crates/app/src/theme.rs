//! The window's palette: one soft-UI material and every colour the window
//! draws with, computed in OKLCH rather than typed. Light and dark are one
//! lightness table each over the same hue, so every contrast the tests
//! check holds in both. The rules are in `docs/DESIGN.md`.
//!
//! Nothing here touches egui's style: `skin::install` does that with the
//! palette this module computes.

use egui::Color32;

/// Light or dark: which way the surface and its two shadows go. Follows
/// the theme egui resolves from the preference (`prefs::Theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Light,
    Dark,
}

impl Mode {
    pub fn of(theme: egui::Theme) -> Self {
        match theme {
            egui::Theme::Light => Mode::Light,
            egui::Theme::Dark => Mode::Dark,
        }
    }

    pub fn theme(self) -> egui::Theme {
        match self {
            Mode::Light => egui::Theme::Light,
            Mode::Dark => egui::Theme::Dark,
        }
    }
}

/// The surface's hue, in OKLCH degrees: a cool grey. The accent is its
/// complement, amber.
pub const HUE: f32 = 250.0;

/// Every colour the window draws with: one hue at different lightness and
/// chroma, so the window reads as one material with one light on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub mode: Mode,
    /// The body and every raised part: the one material everything is
    /// made of.
    pub surface: Color32,
    /// The floor of an inset that shows something (a well, a readout,
    /// a field), a little deeper than the surface.
    pub sunken: Color32,
    /// A groove or track: a slider's channel, the stripe of every other
    /// row on a screen.
    pub groove: Color32,
    /// The light shadow: the surface lit, cast up and left of a raised
    /// part and along the lower right inside an inset one.
    pub light: Color32,
    /// The dark shadow: the surface shaded, cast down and right of a
    /// raised part and along the upper left inside an inset one.
    pub shade: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    /// The uppercase labels.
    pub legend: Color32,
    /// Lit things: the chosen chip or tab, a slider's travel, a switch that
    /// is on, a section's More when open.
    pub accent: Color32,
    /// The accent faded into the surface: a selection.
    pub accent_dim: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    /// Errors, and what takes something away (Remove Tile).
    pub alarm: Color32,
}

/// Lightness (OKLCH L, 0 to 1) of each token in one mode. Chroma and hue
/// are the same in both modes; only these move.
struct Levels {
    surface: f32,
    sunken: f32,
    groove: f32,
    light: f32,
    shade: f32,
    text: f32,
    text_weak: f32,
    legend: f32,
    accent: f32,
    on_accent: f32,
    alarm: f32,
}

const LIGHT: Levels = Levels {
    surface: 0.91,
    sunken: 0.885,
    groove: 0.84,
    light: 1.0,
    shade: 0.76,
    text: 0.24,
    text_weak: 0.44,
    legend: 0.40,
    accent: 0.42,
    on_accent: 0.98,
    alarm: 0.55,
};

const DARK: Levels = Levels {
    surface: 0.30,
    sunken: 0.275,
    groove: 0.24,
    light: 0.39,
    shade: 0.16,
    text: 0.95,
    text_weak: 0.78,
    legend: 0.82,
    accent: 0.80,
    on_accent: 0.18,
    alarm: 0.72,
};

/// How coloured the surface is: a hint of hue, not a colour.
const SURFACE_CHROMA: f32 = 0.012;
const TEXT_CHROMA: f32 = 0.01;
const ACCENT_CHROMA: f32 = 0.15;
const ALARM_CHROMA: f32 = 0.18;
/// Red.
const ALARM_HUE: f32 = 25.0;

impl Palette {
    pub fn of(mode: Mode) -> Self {
        let levels = match mode {
            Mode::Light => &LIGHT,
            Mode::Dark => &DARK,
        };
        let surface_tone = |l: f32| oklch(l, SURFACE_CHROMA, HUE);
        let text_tone = |l: f32| oklch(l, TEXT_CHROMA, HUE);
        let accent_hue = (HUE + 180.0).rem_euclid(360.0);
        let surface = surface_tone(levels.surface);
        let accent = oklch(levels.accent, ACCENT_CHROMA, accent_hue);
        Self {
            mode,
            surface,
            sunken: surface_tone(levels.sunken),
            groove: surface_tone(levels.groove),
            light: surface_tone(levels.light),
            shade: surface_tone(levels.shade),
            text: text_tone(levels.text),
            text_weak: text_tone(levels.text_weak),
            legend: text_tone(levels.legend),
            accent,
            accent_dim: surface.lerp_to_gamma(accent, 0.3),
            on_accent: oklch(levels.on_accent, TEXT_CHROMA, accent_hue),
            alarm: oklch(levels.alarm, ALARM_CHROMA, ALARM_HUE),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Palette::of(Mode::Dark)
    }
}

// --- Colour maths ---------------------------------------------------------

/// An sRGB colour from OKLCH lightness (0 to 1), chroma and hue (degrees).
/// A colour outside the sRGB gamut loses chroma until it fits, so its hue
/// and lightness stay where they were asked for.
pub fn oklch(l: f32, c: f32, h_deg: f32) -> Color32 {
    let [l, a, b] = lab(l, c, h_deg);
    oklab(l, a, b)
}

/// OKLCH as OKLab.
fn lab(l: f32, c: f32, h_deg: f32) -> [f32; 3] {
    let h = h_deg.to_radians();
    [l, c * h.cos(), c * h.sin()]
}

/// An sRGB colour from OKLab, losing chroma as [`oklch`] does.
fn oklab(l: f32, a: f32, b: f32) -> Color32 {
    let l = l.clamp(0.0, 1.0);
    let mut rgb = oklab_to_linear(l, a, b);
    if !in_gamut(rgb) {
        // Grey (no chroma) is always in gamut; find the most chroma that is.
        let (mut fits, mut spills) = (0.0f32, 1.0f32);
        for _ in 0..16 {
            let mid = (fits + spills) / 2.0;
            if in_gamut(oklab_to_linear(l, mid * a, mid * b)) {
                fits = mid;
            } else {
                spills = mid;
            }
        }
        rgb = oklab_to_linear(l, fits * a, fits * b);
    }
    let channel = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        let gamma = if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (gamma * 255.0).round() as u8
    };
    Color32::from_rgb(channel(rgb[0]), channel(rgb[1]), channel(rgb[2]))
}

fn in_gamut(rgb: [f32; 3]) -> bool {
    rgb.iter().all(|v| (-1e-4..=1.0 + 1e-4).contains(v))
}

/// OKLab to linear sRGB (Björn Ottosson's matrices).
fn oklab_to_linear(l: f32, a: f32, b: f32) -> [f32; 3] {
    let l_ = l + 0.3963378 * a + 0.2158038 * b;
    let m_ = l - 0.1055613 * a - 0.06385417 * b;
    let s_ = l - 0.08948418 * a - 1.291486 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076742 * l3 - 3.307712 * m3 + 0.2309699 * s3,
        -1.268438 * l3 + 2.609757 * m3 - 0.3413194 * s3,
        -0.004196086 * l3 - 0.7034186 * m3 + 1.707615 * s3,
    ]
}

/// sRGB channel to linear light.
#[cfg(test)]
fn linear(v: u8) -> f32 {
    let v = f32::from(v) / 255.0;
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance, ignoring alpha.
#[cfg(test)]
pub fn luminance(c: Color32) -> f32 {
    0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
}

/// WCAG contrast ratio between two opaque colours, 1 to 21.
#[cfg(test)]
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (l1, l2) = (luminance(a), luminance(b));
    (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn both() -> impl Iterator<Item = (Mode, Palette)> {
        [Mode::Light, Mode::Dark]
            .into_iter()
            .map(|mode| (mode, Palette::of(mode)))
    }

    #[test]
    fn text_reads_on_every_surface() {
        for (mode, p) in both() {
            let what = format!("{mode:?}");
            // WCAG 1.4.3: 7:1 for the main text, 4.5:1 for the rest.
            assert!(contrast(p.text, p.surface) >= 7.0, "{what}: text");
            assert!(
                contrast(p.text, p.sunken) >= 7.0,
                "{what}: text on a screen"
            );
            assert!(contrast(p.text_weak, p.surface) >= 4.5, "{what}: weak text");
            assert!(
                contrast(p.text_weak, p.sunken) >= 4.5,
                "{what}: weak text on a screen"
            );
            assert!(contrast(p.legend, p.surface) >= 4.5, "{what}: legend");
            assert!(contrast(p.accent, p.surface) >= 4.5, "{what}: accent");
            assert!(
                contrast(p.accent, p.sunken) >= 4.5,
                "{what}: accent readout"
            );
            assert!(contrast(p.on_accent, p.accent) >= 4.5, "{what}: on accent");
            // WCAG 1.4.11: 3:1 for a state shown without text.
            assert!(contrast(p.alarm, p.surface) >= 3.0, "{what}: alarm");
            assert!(
                contrast(p.accent_dim, p.sunken) >= 1.3,
                "{what}: a selection shows"
            );
            assert!(
                contrast(p.text, p.accent_dim) >= 4.5,
                "{what}: selected text"
            );
        }
    }

    #[test]
    fn the_light_shadow_is_lighter_and_the_dark_shadow_darker_than_the_surface() {
        for (mode, p) in both() {
            let what = format!("{mode:?}");
            assert!(luminance(p.light) > luminance(p.surface), "{what}: light");
            assert!(luminance(p.shade) < luminance(p.surface), "{what}: shade");
            assert!(
                luminance(p.shade) < luminance(p.sunken)
                    && luminance(p.sunken) < luminance(p.surface),
                "{what}: an inset floor sits between the surface and its shadow"
            );
            assert!(luminance(p.groove) < luminance(p.sunken), "{what}: groove");
        }
    }

    #[test]
    fn light_and_dark_are_each_other_upside_down() {
        let light = Palette::of(Mode::Light);
        let dark = Palette::of(Mode::Dark);
        assert!(luminance(light.surface) > 0.5 && luminance(dark.surface) < 0.1);
        assert!(luminance(light.text) < luminance(light.surface));
        assert!(luminance(dark.text) > luminance(dark.surface));
    }

    #[test]
    fn the_mode_follows_the_theme_egui_resolves() {
        assert_eq!(Mode::of(egui::Theme::Light), Mode::Light);
        assert_eq!(Mode::of(egui::Theme::Dark), Mode::Dark);
        assert_eq!(Mode::Light.theme(), egui::Theme::Light);
        assert_eq!(Mode::Dark.theme(), egui::Theme::Dark);
    }

    #[test]
    fn oklch_grey_is_grey_and_white_is_white() {
        let grey = oklch(0.5, 0.0, 0.0);
        assert_eq!(grey.r(), grey.g());
        assert_eq!(grey.g(), grey.b());
        assert_eq!(oklch(1.0, 0.0, 0.0), Color32::from_rgb(255, 255, 255));
        assert_eq!(oklch(0.0, 0.0, 0.0), Color32::from_rgb(0, 0, 0));
    }
}

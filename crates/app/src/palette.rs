//! Colours for tiles, in light and dark mode.

use egui::Color32;
use olio_library::TileColor;

/// The tile colour at full strength, for titles and the played part of the
/// waveform. Dark mode lifts it a little so it reads on a dark tile.
pub fn strong(color: TileColor, dark: bool) -> Color32 {
    let (r, g, b) = match color {
        TileColor::Purple => (139, 61, 255),
        TileColor::Pink => (233, 30, 140),
        TileColor::Red => (229, 72, 77),
        TileColor::Orange => (247, 107, 21),
        TileColor::Yellow => (205, 152, 0),
        TileColor::Green => (43, 168, 74),
        TileColor::Teal => (18, 165, 148),
        TileColor::Blue => (62, 99, 221),
        TileColor::Gray => (128, 131, 143),
    };
    let base = Color32::from_rgb(r, g, b);
    if dark {
        base.lerp_to_gamma(Color32::WHITE, 0.18)
    } else {
        base
    }
}

/// The colour at `alpha` (0-255) opacity, for tints.
pub fn tint(color: TileColor, dark: bool, alpha: u8) -> Color32 {
    let strong = strong(color, dark);
    Color32::from_rgba_unmultiplied(strong.r(), strong.g(), strong.b(), alpha)
}

pub fn tile_background(dark: bool) -> Color32 {
    if dark {
        Color32::from_gray(36)
    } else {
        Color32::WHITE
    }
}

/// For buttons that remove things.
pub fn danger() -> Color32 {
    Color32::from_rgb(205, 52, 60)
}

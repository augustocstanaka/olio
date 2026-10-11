//! Colours for tiles, in light and dark mode.

use egui::{Color32, Shadow};
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

/// The colours and light of the Soft tile style. The canvas is a little
/// darker than a raised tile, so the light from the top left shows on
/// tiles and the shadow falls to the bottom right.
pub struct Soft {
    /// Behind the grid.
    pub canvas: Color32,
    /// A tile at rest.
    pub raised: Color32,
    /// The rim of a pressed tile, where its inner shadow falls.
    pub pressed_rim: Color32,
    /// The middle of a pressed tile.
    pub pressed_fill: Color32,
    /// An empty slot, a shallow well.
    pub well: Color32,
    pub dark_shadow: Shadow,
    pub light_shadow: Shadow,
}

pub fn soft(dark: bool) -> Soft {
    if dark {
        Soft {
            canvas: Color32::from_gray(30),
            raised: Color32::from_gray(40),
            pressed_rim: Color32::from_gray(20),
            pressed_fill: Color32::from_gray(31),
            well: Color32::from_gray(27),
            dark_shadow: Shadow {
                offset: [4, 4],
                blur: 12,
                spread: 0,
                color: Color32::from_black_alpha(150),
            },
            light_shadow: Shadow {
                offset: [-3, -3],
                blur: 10,
                spread: 0,
                color: Color32::from_white_alpha(14),
            },
        }
    } else {
        Soft {
            canvas: Color32::from_rgb(228, 232, 238),
            raised: Color32::from_rgb(239, 242, 247),
            pressed_rim: Color32::from_rgb(207, 213, 222),
            pressed_fill: Color32::from_rgb(228, 232, 238),
            well: Color32::from_rgb(222, 226, 233),
            dark_shadow: Shadow {
                offset: [4, 4],
                blur: 12,
                spread: 0,
                color: Color32::from_rgba_unmultiplied(140, 152, 175, 150),
            },
            light_shadow: Shadow {
                offset: [-3, -3],
                blur: 10,
                spread: 0,
                color: Color32::from_white_alpha(230),
            },
        }
    }
}

//! Colours for tiles, in light and dark mode.

use egui::Color32;
use olio_library::TileColor;

/// How far each mode moves a tile colour so it reads on the surface (3:1,
/// `tests::tile_colours_read_on_the_surface`): lighter on a dark surface,
/// darker on a light one.
const DARK_LIFT: f32 = 0.18;
const LIGHT_DEEPEN: f32 = 0.25;

/// The tile colour at full strength, for titles and the played part of the
/// waveform, moved towards the far end of the mode so it reads on the
/// surface.
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
        base.lerp_to_gamma(Color32::WHITE, DARK_LIFT)
    } else {
        base.lerp_to_gamma(Color32::BLACK, LIGHT_DEEPEN)
    }
}

/// The colour at `alpha` (0-255) opacity, for tints.
pub fn tint(color: TileColor, dark: bool, alpha: u8) -> Color32 {
    let strong = strong(color, dark);
    Color32::from_rgba_unmultiplied(strong.r(), strong.g(), strong.b(), alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Mode, Palette, contrast};

    /// Every colour, as a title and a played waveform on a tile, against the
    /// surface the tile is raised from: WCAG 1.4.11's 3:1 for a mark.
    #[test]
    fn tile_colours_read_on_the_surface() {
        for mode in [Mode::Light, Mode::Dark] {
            let surface = Palette::of(mode).surface;
            for color in TileColor::ALL {
                let seen = contrast(strong(color, mode == Mode::Dark), surface);
                assert!(seen >= 3.0, "{mode:?} {color:?}: {seen:.2}");
            }
        }
    }
}

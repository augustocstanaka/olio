//! Fonts beyond egui's own: on macOS the system's symbol font, so shortcuts
//! read ⌘O, ⇧⌘S and ⌥Q the way the platform writes them. egui falls back
//! to "Cmd+O" wherever the glyphs are missing, so nothing breaks without
//! it.

use egui::FontDefinitions;

/// Where macOS keeps the font with the modifier symbols.
#[cfg(target_os = "macos")]
const SYMBOL_FONT: &str = "/System/Library/Fonts/Apple Symbols.ttf";

/// Installs the extra fonts; a no-op where there are none.
pub fn install(ctx: &egui::Context) {
    let mut definitions = FontDefinitions::default();
    if !add_symbols(&mut definitions) {
        return;
    }
    ctx.set_fonts(definitions);
}

#[cfg(target_os = "macos")]
fn add_symbols(definitions: &mut FontDefinitions) -> bool {
    use egui::{FontData, FontFamily};

    let Ok(bytes) = std::fs::read(SYMBOL_FONT) else {
        return false;
    };
    let name = "Apple Symbols".to_owned();
    definitions.font_data.insert(
        name.clone(),
        std::sync::Arc::new(FontData::from_owned(bytes)),
    );
    // Last in line: only glyphs the bundled fonts lack come from it.
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        definitions
            .families
            .entry(family)
            .or_default()
            .push(name.clone());
    }
    true
}

#[cfg(not(target_os = "macos"))]
fn add_symbols(_definitions: &mut FontDefinitions) -> bool {
    false
}

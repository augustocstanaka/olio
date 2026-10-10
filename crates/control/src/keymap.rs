//! Which key plays which slot of the front set's grid.
//!
//! The grid follows the keyboard: row 0 is the number row (1 to 0), then
//! Q to P, A to ;, and Z to /. Rows 4 to 7 are the same keys with Option
//! (Alt) held. Column `n` is the `n`th key of its row, so a set three
//! columns wide is played with 1 2 3 / Q W E / A S D / Z X C.
//!
//! Keys are named by their position on a US QWERTY keyboard; the app reads
//! the physical key, so the grid keeps its shape on other layouts.

use olio_library::{MAX_COLUMNS, MAX_ROWS, Slot};

/// The four keyboard rows, ten keys each, as lowercase characters.
pub const KEY_ROWS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl;", "zxcvbnm,./"];

const KEY_ROW_COUNT: u8 = KEY_ROWS.len() as u8;

/// The key (and whether Option/Alt is held) that plays a grid slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    /// The key's character on US QWERTY, lowercase: one of [`KEY_ROWS`].
    pub key: char,
    pub alt: bool,
}

impl TileKey {
    /// The key for a slot; `None` outside the 8 x 10 grid.
    pub fn for_slot(slot: Slot) -> Option<Self> {
        if slot.row >= MAX_ROWS || slot.column >= MAX_COLUMNS {
            return None;
        }
        let row = KEY_ROWS[usize::from(slot.row % KEY_ROW_COUNT)];
        Some(Self {
            key: row.chars().nth(usize::from(slot.column))?,
            alt: slot.row >= KEY_ROW_COUNT,
        })
    }

    /// The slot this key plays; `None` for keys outside the grid.
    pub fn slot(self) -> Option<Slot> {
        KEY_ROWS.iter().enumerate().find_map(|(row, keys)| {
            let column = keys.chars().position(|key| key == self.key)?;
            let row = row as u8 + if self.alt { KEY_ROW_COUNT } else { 0 };
            Some(Slot::new(row, column as u8))
        })
    }

    /// Reads a key as people write it: "Q", "q", "1", ";", and for the lower
    /// four rows "Alt+Q", "Option+Q" or "⌥Q". `None` for anything that is
    /// not one of the grid's keys.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let lower = text.to_lowercase();
        let (alt, key) = ["alt+", "option+", "opt+", "⌥"]
            .iter()
            .find_map(|prefix| lower.strip_prefix(prefix).map(|rest| (true, rest)))
            .unwrap_or((false, lower.as_str()));
        let mut chars = key.trim().chars();
        let key = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        let tile_key = Self { key, alt };
        tile_key.slot().map(|_| tile_key)
    }

    /// How the key is shown on a tile: "Q", or "⌥Q" (Alt+Q off macOS).
    pub fn label(self) -> String {
        let key = self.key.to_ascii_uppercase();
        if !self.alt {
            key.to_string()
        } else if cfg!(target_os = "macos") {
            format!("⌥{key}")
        } else {
            format!("Alt+{key}")
        }
    }
}

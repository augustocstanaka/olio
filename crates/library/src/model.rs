//! The library's data: sets of tiles, each tile playing one sound.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The version of `library.json` this build writes. A change older builds
/// cannot read bumps it; an additive field with a default does not.
pub const FORMAT_VERSION: u32 = 1;

/// Rows in a set's grid: four rows of keys, then the same four with
/// Option (Alt) held.
pub const MAX_ROWS: u8 = 8;

/// Columns in a set's grid: the ten keys of each keyboard row.
pub const MAX_COLUMNS: u8 = 10;

/// Columns a new set starts with.
pub const DEFAULT_COLUMNS: u8 = 4;

/// Rows a grid shows even when they are empty, so there is room to drop
/// sounds.
pub const MIN_VISIBLE_ROWS: u8 = 4;

/// Columns for a new set that should show `tiles` tiles in about four
/// rows: at least [`DEFAULT_COLUMNS`], at most [`MAX_COLUMNS`].
pub fn columns_to_fit(tiles: usize) -> u8 {
    let columns = tiles.div_ceil(usize::from(MIN_VISIBLE_ROWS));
    columns.clamp(usize::from(DEFAULT_COLUMNS), usize::from(MAX_COLUMNS)) as u8
}

/// The most tiles a set can hold.
pub const MAX_TILES: usize = MAX_ROWS as usize * MAX_COLUMNS as usize;

/// Identifies a set for its whole life, across renames and moves. Also the
/// stable address control surfaces use (ADR 0004).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SetId(pub Uuid);

impl SetId {
    pub fn random() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Identifies a tile for its whole life, across renames and moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TileId(pub Uuid);

impl TileId {
    pub fn random() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Everything a user has made: an ordered list of sets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Library {
    pub format_version: u32,
    pub sets: Vec<Set>,
}

impl Default for Library {
    /// A library with one empty set, ready for sounds.
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            sets: vec![Set::new("My Sounds")],
        }
    }
}

impl Library {
    pub fn set(&self, id: SetId) -> Option<&Set> {
        self.sets.iter().find(|set| set.id == id)
    }

    pub fn set_mut(&mut self, id: SetId) -> Option<&mut Set> {
        self.sets.iter_mut().find(|set| set.id == id)
    }

    /// The set holding a tile, and the tile.
    pub fn tile(&self, id: TileId) -> Option<(&Set, &Tile)> {
        self.sets
            .iter()
            .find_map(|set| set.tile(id).map(|tile| (set, tile)))
    }

    pub fn tile_mut(&mut self, id: TileId) -> Option<&mut Tile> {
        self.sets.iter_mut().find_map(|set| set.tile_mut(id))
    }

    /// Moves a tile to another set, into `preferred` there if it is free
    /// (by default its own slot, so it keeps its key), else the earliest
    /// free slot. Returns the set and slot it left and the slot it took;
    /// `None` if the tile or set is unknown, the tile is already in that
    /// set, or the set is full.
    pub fn move_tile_to_set(
        &mut self,
        id: TileId,
        to: SetId,
        preferred: Option<Slot>,
    ) -> Option<(SetId, Slot, Slot)> {
        let (from, tile) = self.tile(id)?;
        let (from, from_slot) = (from.id, tile.slot);
        if from == to {
            return None;
        }
        let slot = self
            .set(to)?
            .free_slot_from(preferred.unwrap_or(from_slot))?;
        let mut tile = self.set_mut(from)?.remove_tile(id)?;
        tile.slot = slot;
        self.set_mut(to)?.tiles.push(tile);
        Some((from, from_slot, slot))
    }

    /// Copies a tile, title and all, into another set: into `preferred`
    /// if it is free (by default the tile's own slot), else the earliest
    /// free slot. Returns the copy's id; `None` if the tile or set is
    /// unknown, the tile is in that set already (duplicate it instead), or
    /// the set is full.
    pub fn copy_tile_to_set(
        &mut self,
        id: TileId,
        to: SetId,
        preferred: Option<Slot>,
    ) -> Option<TileId> {
        let (from, tile) = self.tile(id)?;
        if from.id == to {
            return None;
        }
        let mut copy = tile.clone();
        let target = self.set(to)?;
        copy.slot = target.free_slot_from(preferred.unwrap_or(copy.slot))?;
        copy.id = TileId::random();
        let copy_id = copy.id;
        self.set_mut(to)?.tiles.push(copy);
        Some(copy_id)
    }

    /// Appends a new empty set with a name no other set has.
    pub fn add_set(&mut self) -> SetId {
        let set = Set::new(self.unused_set_name());
        let id = set.id;
        self.sets.push(set);
        id
    }

    pub fn remove_set(&mut self, id: SetId) -> Option<Set> {
        let index = self.sets.iter().position(|set| set.id == id)?;
        Some(self.sets.remove(index))
    }

    /// "Set 2", "Set 3", ...: the first such name not already taken.
    pub fn unused_set_name(&self) -> String {
        (self.sets.len() + 1..)
            .map(|n| format!("Set {n}"))
            .find(|name| !self.has_set_named(name))
            .expect("an unbounded range always has a free name")
    }

    /// `base` if no set has that name, else "base 2", "base 3", ...
    pub fn unique_set_name(&self, base: &str) -> String {
        if !self.has_set_named(base) {
            return base.to_owned();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|name| !self.has_set_named(name))
            .expect("an unbounded range always has a free name")
    }

    pub fn has_set_named(&self, name: &str) -> bool {
        self.sets.iter().any(|set| set.name == name)
    }
}

/// A named page of tiles laid out on the keyboard grid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Set {
    pub id: SetId,
    pub name: String,
    /// Columns of the grid, 1 to [`MAX_COLUMNS`]; there are always
    /// [`MAX_ROWS`] rows.
    pub columns: u8,
    pub tiles: Vec<Tile>,
}

impl Set {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: SetId::random(),
            name: name.into(),
            columns: DEFAULT_COLUMNS,
            tiles: Vec::new(),
        }
    }

    pub fn tile(&self, id: TileId) -> Option<&Tile> {
        self.tiles.iter().find(|tile| tile.id == id)
    }

    pub fn tile_mut(&mut self, id: TileId) -> Option<&mut Tile> {
        self.tiles.iter_mut().find(|tile| tile.id == id)
    }

    pub fn tile_at(&self, slot: Slot) -> Option<&Tile> {
        self.tiles.iter().find(|tile| tile.slot == slot)
    }

    /// True if `slot` is inside this set's grid.
    pub fn contains(&self, slot: Slot) -> bool {
        slot.row < MAX_ROWS && slot.column < self.columns
    }

    /// True if `slot` is inside the grid and holds no tile.
    pub fn is_free(&self, slot: Slot) -> bool {
        self.contains(slot) && self.tile_at(slot).is_none()
    }

    /// The earliest free slot, reading the grid row by row.
    pub fn first_free_slot(&self) -> Option<Slot> {
        (0..MAX_ROWS)
            .flat_map(|row| (0..self.columns).map(move |column| Slot { row, column }))
            .find(|&slot| self.is_free(slot))
    }

    /// Adds a tile for `sound` at `preferred` if that slot is free, else at
    /// the earliest free slot. `None` when the set is full.
    pub fn place(&mut self, sound: Sound, preferred: Option<Slot>) -> Option<TileId> {
        let slot = preferred
            .filter(|&slot| self.is_free(slot))
            .or_else(|| self.first_free_slot())?;
        let tile = Tile::new(sound, slot);
        let id = tile.id;
        self.tiles.push(tile);
        Some(id)
    }

    /// Moves a tile to `to`. A tile already there takes the moved tile's
    /// old slot, so the two swap. Returns the slot the tile came from;
    /// `None` if the tile is not here, `to` is outside the grid, or it is
    /// already there.
    pub fn move_tile(&mut self, id: TileId, to: Slot) -> Option<Slot> {
        if !self.contains(to) {
            return None;
        }
        let from = self.tile(id)?.slot;
        if from == to {
            return None;
        }
        if let Some(other) = self.tiles.iter_mut().find(|tile| tile.slot == to) {
            other.slot = from;
        }
        self.tile_mut(id)?.slot = to;
        Some(from)
    }

    /// The first free slot after `slot`, reading row by row and wrapping
    /// round to the top; `None` when the set is full.
    pub fn free_slot_after(&self, slot: Slot) -> Option<Slot> {
        let columns = usize::from(self.columns.max(1));
        let count = usize::from(MAX_ROWS) * columns;
        let start = usize::from(slot.row) * columns + usize::from(slot.column);
        (1..=count)
            .map(|step| (start + step) % count)
            .map(|index| Slot::new((index / columns) as u8, (index % columns) as u8))
            .find(|&slot| self.is_free(slot))
    }

    /// Where something dropped on `slot` lands without displacing a tile:
    /// `slot` if it is free, else the first free slot after it. `None`
    /// when the set is full.
    pub fn landing_slot(&self, slot: Slot) -> Option<Slot> {
        if self.is_free(slot) {
            Some(slot)
        } else {
            self.free_slot_after(slot)
        }
    }

    /// Copies a tile, with all its settings and the same sound, into `at`
    /// if that slot is free, else the first free slot after it (after the
    /// original, without `at`). Returns the copy's id; `None` if the tile
    /// is not here or the set is full.
    pub fn duplicate(&mut self, id: TileId, at: Option<Slot>) -> Option<TileId> {
        let original = self.tile(id)?;
        let slot = match at {
            Some(at) => self.landing_slot(at)?,
            None => self.free_slot_after(original.slot)?,
        };
        let mut copy = original.clone();
        copy.id = TileId::random();
        copy.slot = slot;
        copy.title = format!("{} copy", original.title);
        let id = copy.id;
        self.tiles.push(copy);
        Some(id)
    }

    /// `slot` if it is free, else the earliest free slot; `None` when the
    /// set is full.
    pub fn free_slot_from(&self, slot: Slot) -> Option<Slot> {
        Some(slot)
            .filter(|&slot| self.is_free(slot))
            .or_else(|| self.first_free_slot())
    }

    /// Puts a whole tile back, as when a removal is undone: in its own slot
    /// if that is free, else the earliest free slot. `None` (and the tile
    /// is not added) when the set is full.
    pub fn put_back(&mut self, tile: &Tile) -> Option<Slot> {
        let slot = self.free_slot_from(tile.slot)?;
        let mut tile = tile.clone();
        tile.slot = slot;
        self.tiles.push(tile);
        Some(slot)
    }

    pub fn remove_tile(&mut self, id: TileId) -> Option<Tile> {
        let index = self.tiles.iter().position(|tile| tile.id == id)?;
        Some(self.tiles.remove(index))
    }

    /// Rows the grid shows: every row up to the last tile, and at least
    /// [`MIN_VISIBLE_ROWS`].
    pub fn visible_rows(&self) -> u8 {
        let used = self
            .tiles
            .iter()
            .map(|tile| tile.slot.row + 1)
            .max()
            .unwrap_or(0);
        used.clamp(MIN_VISIBLE_ROWS, MAX_ROWS)
    }
}

/// A position in a set's grid. Row 0 is the top row; rows 0-3 follow the
/// keyboard rows from the number row down, and rows 4-7 repeat them with
/// Option (Alt) held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Slot {
    pub row: u8,
    pub column: u8,
}

impl Slot {
    pub fn new(row: u8, column: u8) -> Self {
        Self { row, column }
    }
}

/// One sound on the board, with how it plays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    pub id: TileId,
    pub slot: Slot,
    pub title: String,
    pub sound: Sound,
    pub color: TileColor,
    /// Play again from the start until stopped.
    #[serde(default)]
    pub looping: bool,
    /// Skip silence at the start and end of the sound.
    #[serde(default = "yes")]
    pub trim_silence: bool,
    /// Play the sound at a common loudness, so quiet and loud recordings
    /// sound alike. Applied as the voice's gain; the file is unchanged.
    #[serde(default = "yes")]
    pub normalize: bool,
    /// Rise from silence over this many seconds when starting.
    #[serde(default)]
    pub fade_in_seconds: f32,
    /// Sink to silence over this many seconds before the end, and when
    /// stopped.
    #[serde(default)]
    pub fade_out_seconds: f32,
    /// Share of the sound sent to the reverb, 0 (dry) to 1.
    #[serde(default)]
    pub reverb: f32,
    /// One emoji shown on the tile, fully qualified ("☺️", not "☺"); empty
    /// for none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub emoji: String,
}

/// The emoji `text` names: the emoji itself ("🎉", with or without its
/// variation selector) or its GitHub shortcode ("tada" or ":tada:"). Always
/// returns the fully qualified emoji, so the same emoji is stored the same
/// way however it was written.
pub fn parse_emoji(text: &str) -> Option<&'static str> {
    let text = text.trim();
    emojis::get(text)
        .or_else(|| emojis::get_by_shortcode(text.trim_matches(':')))
        .map(emojis::Emoji::as_str)
}

fn yes() -> bool {
    true
}

/// The longest fade a tile can have, in seconds.
pub const MAX_FADE_SECONDS: f32 = 30.0;

impl Tile {
    /// A tile titled after the sound's file name, coloured by its column.
    pub fn new(sound: Sound, slot: Slot) -> Self {
        Self {
            id: TileId::random(),
            slot,
            title: title_from_file_name(&sound.original_name),
            sound,
            color: TileColor::for_column(slot.column),
            looping: false,
            trim_silence: true,
            normalize: true,
            fade_in_seconds: 0.0,
            fade_out_seconds: 0.0,
            reverb: 0.0,
            emoji: String::new(),
        }
    }
}

/// "big_game-loop.mp3" becomes "big game-loop".
fn title_from_file_name(name: &str) -> String {
    let stem = match name.rsplit_once('.') {
        Some((stem, _extension)) if !stem.is_empty() => stem,
        _ => name,
    };
    let title = stem.replace('_', " ");
    let title = title.trim();
    if title.is_empty() {
        "Untitled".to_owned()
    } else {
        title.to_owned()
    }
}

/// An imported audio file. The library keeps its own copy, named by the
/// hash of its contents, so the original can move or disappear and two
/// tiles with the same file share one copy.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Sound {
    /// BLAKE3 hash of the file's bytes, in hex.
    pub hash: String,
    /// The original file's extension in lowercase, without the dot; empty
    /// if it had none. Kept on the copy so decoders get a format hint.
    pub extension: String,
    /// The name of the file the sound was imported from.
    pub original_name: String,
}

impl Sound {
    /// The copy's file name inside the library's `sounds` directory.
    pub fn file_name(&self) -> String {
        if self.extension.is_empty() {
            self.hash.clone()
        } else {
            format!("{}.{}", self.hash, self.extension)
        }
    }
}

/// The nine colours a tile can be, for telling groups of sounds apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TileColor {
    Purple,
    Pink,
    Red,
    Orange,
    Yellow,
    Green,
    Teal,
    Blue,
    Gray,
}

impl TileColor {
    pub const ALL: [TileColor; 9] = [
        TileColor::Purple,
        TileColor::Pink,
        TileColor::Red,
        TileColor::Orange,
        TileColor::Yellow,
        TileColor::Green,
        TileColor::Teal,
        TileColor::Blue,
        TileColor::Gray,
    ];

    /// New tiles take their column's colour, so a fresh grid reads in
    /// stripes.
    pub fn for_column(column: u8) -> Self {
        Self::ALL[usize::from(column) % Self::ALL.len()]
    }

    pub fn name(self) -> &'static str {
        match self {
            TileColor::Purple => "Purple",
            TileColor::Pink => "Pink",
            TileColor::Red => "Red",
            TileColor::Orange => "Orange",
            TileColor::Yellow => "Yellow",
            TileColor::Green => "Green",
            TileColor::Teal => "Teal",
            TileColor::Blue => "Blue",
            TileColor::Gray => "Gray",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::title_from_file_name;

    #[test]
    fn titles_come_from_file_names() {
        assert_eq!(title_from_file_name("big_game-loop.mp3"), "big game-loop");
        assert_eq!(title_from_file_name("Anthem.final.wav"), "Anthem.final");
        assert_eq!(title_from_file_name("noext"), "noext");
        assert_eq!(title_from_file_name(".hidden"), ".hidden");
        assert_eq!(title_from_file_name("__.wav"), "Untitled");
    }
}

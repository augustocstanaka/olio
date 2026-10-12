//! Undo and redo for changes to the board: removing a tile or a set,
//! duplicating a tile, and moving one to another slot.
//!
//! A removal keeps the whole tile or set, so undoing it brings back its
//! slot, title, emoji and settings exactly. Settings changed in the
//! inspector are not recorded: they are small and made in plain sight.

use olio_library::{Set, SetId, Slot, Tile, TileId};

/// The most changes remembered.
const LIMIT: usize = 50;

/// A change that can be undone, holding what is needed to undo or redo it.
#[derive(Clone, Debug)]
pub enum Change {
    // Tiles are boxed: one is much larger than a set's Vec.
    RemovedTile {
        set: SetId,
        tile: Box<Tile>,
    },
    RemovedSet {
        index: usize,
        set: Set,
    },
    /// A copy of the tile titled `source`.
    Duplicated {
        set: SetId,
        tile: Box<Tile>,
        source: String,
    },
    /// A tile moved, or copied, to another set. `tile` is as it is there
    /// now: the moved tile, or the copy.
    ToSet {
        tile: Box<Tile>,
        from: SetId,
        from_slot: Slot,
        to: SetId,
        to_name: String,
        copy: bool,
    },
    /// The set's grid was made wider or narrower.
    Columns {
        set: SetId,
        name: String,
        from: u8,
        to: u8,
    },
    /// The tile went from one slot to another, swapping with any tile there.
    Moved {
        set: SetId,
        tile: TileId,
        title: String,
        from: Slot,
        to: Slot,
    },
}

impl Change {
    /// What it was, for the Edit menu: "Remove “Applause”".
    pub fn describe(&self) -> String {
        match self {
            Self::RemovedTile { tile, .. } => format!("Remove “{}”", tile.title),
            Self::RemovedSet { set, .. } => format!("Remove Set “{}”", set.name),
            Self::Duplicated { source, .. } => format!("Duplicate “{source}”"),
            Self::Moved { title, .. } => format!("Move “{title}”"),
            Self::Columns { name, to, .. } => format!("{to} Columns for “{name}”"),
            Self::ToSet {
                tile,
                to_name,
                copy,
                ..
            } => {
                let verb = if *copy { "Copy" } else { "Move" };
                format!("{verb} “{}” to “{to_name}”", tile.title)
            }
        }
    }

    /// The status after making (or redoing) it, if it deserves one.
    pub fn done_message(&self) -> Option<String> {
        match self {
            Self::RemovedTile { tile, .. } => Some(format!("Removed “{}”", tile.title)),
            Self::RemovedSet { set, .. } => Some(format!("Removed the set “{}”", set.name)),
            Self::Duplicated { source, .. } => Some(format!("Duplicated “{source}”")),
            // The tile is seen moving, the grid changing; no need to say so.
            Self::Moved { .. } | Self::Columns { .. } => None,
            // It leaves the grid, or lands out of sight: say where it went.
            Self::ToSet {
                tile,
                to_name,
                copy,
                ..
            } => {
                let verb = if *copy { "Copied" } else { "Moved" };
                Some(format!("{verb} “{}” to “{to_name}”", tile.title))
            }
        }
    }

    /// The status after undoing it.
    pub fn undone_message(&self) -> String {
        match self {
            Self::RemovedTile { tile, .. } => format!("Put back “{}”", tile.title),
            Self::RemovedSet { set, .. } => format!("Put back the set “{}”", set.name),
            Self::Duplicated { tile, .. } => format!("Removed the copy “{}”", tile.title),
            Self::Moved { title, .. } => format!("Moved “{title}” back"),
            Self::Columns { name, from, .. } => format!("“{name}” is {from} columns wide again"),
            Self::ToSet {
                tile,
                to_name,
                copy: true,
                ..
            } => format!("Removed the copy of “{}” from “{to_name}”", tile.title),
            Self::ToSet { tile, .. } => format!("Moved “{}” back", tile.title),
        }
    }
}

#[derive(Default)]
pub struct History {
    undo: Vec<Change>,
    redo: Vec<Change>,
}

impl History {
    /// A new change: it can be undone, and what was undone before can no
    /// longer be redone.
    pub fn record(&mut self, change: Change) {
        self.redo.clear();
        self.push_undo(change);
    }

    /// Back on the undo list after a redo, keeping the rest of the redo list.
    pub fn push_undo(&mut self, change: Change) {
        self.undo.push(change);
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    pub fn push_redo(&mut self, change: Change) {
        self.redo.push(change);
    }

    pub fn take_undo(&mut self) -> Option<Change> {
        self.undo.pop()
    }

    pub fn take_redo(&mut self) -> Option<Change> {
        self.redo.pop()
    }

    pub fn next_undo(&self) -> Option<&Change> {
        self.undo.last()
    }

    pub fn next_redo(&self) -> Option<&Change> {
        self.redo.last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(name: &str) -> Change {
        Change::RemovedSet {
            index: 0,
            set: Set::new(name),
        }
    }

    fn name(change: Option<&Change>) -> Option<String> {
        change.map(Change::describe)
    }

    #[test]
    fn undo_and_redo_walk_back_and_forth_and_a_new_change_ends_redo() {
        let mut history = History::default();
        history.record(set("A"));
        history.record(set("B"));
        assert_eq!(name(history.next_undo()).as_deref(), Some("Remove Set “B”"));

        let b = history.take_undo().unwrap();
        history.push_redo(b);
        assert_eq!(name(history.next_redo()).as_deref(), Some("Remove Set “B”"));
        let b = history.take_redo().unwrap();
        history.push_undo(b);
        assert!(history.next_redo().is_none());

        let b = history.take_undo().unwrap();
        history.push_redo(b);
        history.record(set("C"));
        assert!(history.next_redo().is_none(), "a new change ends redo");
    }

    #[test]
    fn only_the_last_fifty_changes_are_kept() {
        let mut history = History::default();
        for n in 0..60 {
            history.record(set(&n.to_string()));
        }
        let mut count = 0;
        while history.take_undo().is_some() {
            count += 1;
        }
        assert_eq!(count, LIMIT);
    }

    #[test]
    fn moves_are_described_but_not_announced() {
        let moved = Change::Moved {
            set: Set::new("S").id,
            tile: TileId::random(),
            title: "Applause".to_owned(),
            from: Slot::new(0, 0),
            to: Slot::new(1, 1),
        };
        assert_eq!(moved.describe(), "Move “Applause”");
        assert_eq!(moved.done_message(), None);
        assert_eq!(moved.undone_message(), "Moved “Applause” back");
    }
}

//! Undo and redo for removing tiles and sets.
//!
//! A removal keeps the whole tile or set, so undoing it brings back its
//! slot, title, emoji and settings exactly. Only removals are recorded:
//! other edits are small and made in plain sight in the inspector.

use olio_library::{Set, SetId, Tile};

/// The most removals remembered.
const LIMIT: usize = 50;

/// Something removed, kept so it can be put back.
#[derive(Clone, Debug)]
pub enum Removed {
    // Boxed: a tile is much larger than the set variant's Vec.
    Tile { set: SetId, tile: Box<Tile> },
    Set { index: usize, set: Set },
}

impl Removed {
    /// What the removal was, for the Edit menu: "Remove “Applause”".
    pub fn describe(&self) -> String {
        match self {
            Self::Tile { tile, .. } => format!("Remove “{}”", tile.title),
            Self::Set { set, .. } => format!("Remove Set “{}”", set.name),
        }
    }

    /// The status after removing it: "Removed “Applause”".
    pub fn removed_message(&self) -> String {
        match self {
            Self::Tile { tile, .. } => format!("Removed “{}”", tile.title),
            Self::Set { set, .. } => format!("Removed the set “{}”", set.name),
        }
    }

    /// The status after undoing it: "Put back “Applause”".
    pub fn put_back_message(&self) -> String {
        match self {
            Self::Tile { tile, .. } => format!("Put back “{}”", tile.title),
            Self::Set { set, .. } => format!("Put back the set “{}”", set.name),
        }
    }
}

#[derive(Default)]
pub struct History {
    undo: Vec<Removed>,
    redo: Vec<Removed>,
}

impl History {
    /// A new removal: it can be undone, and what was undone before can no
    /// longer be redone.
    pub fn record(&mut self, removed: Removed) {
        self.redo.clear();
        self.push_undo(removed);
    }

    /// Back on the undo list after a redo, keeping the rest of the redo list.
    pub fn push_undo(&mut self, removed: Removed) {
        self.undo.push(removed);
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    pub fn push_redo(&mut self, removed: Removed) {
        self.redo.push(removed);
    }

    pub fn take_undo(&mut self) -> Option<Removed> {
        self.undo.pop()
    }

    pub fn take_redo(&mut self) -> Option<Removed> {
        self.redo.pop()
    }

    pub fn next_undo(&self) -> Option<&Removed> {
        self.undo.last()
    }

    pub fn next_redo(&self) -> Option<&Removed> {
        self.redo.last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(name: &str) -> Removed {
        Removed::Set {
            index: 0,
            set: Set::new(name),
        }
    }

    fn name(removed: Option<&Removed>) -> Option<String> {
        removed.map(Removed::describe)
    }

    #[test]
    fn undo_and_redo_walk_back_and_forth_and_a_new_removal_ends_redo() {
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
        assert!(history.next_redo().is_none(), "a new removal ends redo");
    }

    #[test]
    fn only_the_last_fifty_removals_are_kept() {
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
}

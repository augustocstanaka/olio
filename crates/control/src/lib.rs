//! Olio's control vocabulary (ADR 0004).
//!
//! Every way of driving Olio (the keyboard and mouse today; MIDI, OSC, the
//! Stream Deck plugin and automations later) turns its input into a
//! [`Command`], and the app carries out commands in one place. A new control
//! surface is therefore a translator into commands, never a second path into
//! the app.
//!
//! - [`command`]: the [`Command`] enum.
//! - [`keymap`]: which key plays which grid slot.

pub mod command;
pub mod keymap;

pub use command::{Command, TileSetting};
pub use keymap::TileKey;

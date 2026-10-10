//! Olio's library: the sets, tiles and sounds a user has built up, and how
//! they are stored on disk (ADR 0003).
//!
//! - [`model`]: [`Library`], [`Set`], [`Tile`], [`Sound`] and the grid
//!   [`Slot`]s tiles sit in. Plain data with serde, plus the rules for
//!   placing tiles.
//! - [`store`]: the library directory: `library.json` and the copies of
//!   every imported sound.
//! - [`manifest`]: boards written by hand or by scripts as JSON, to import.
//! - [`folder`]: listing a folder's files in natural alphabetical order.
//!
//! Depends on no UI or audio crate.

pub mod folder;
pub mod manifest;
pub mod model;
pub mod store;

pub use manifest::{Manifest, ManifestError, ManifestSet, ManifestTile, TileSpec};
pub use model::{
    FORMAT_VERSION, Library, MAX_COLUMNS, MAX_FADE_SECONDS, MAX_ROWS, MAX_TILES, Set, SetId, Slot,
    Sound, Tile, TileColor, TileId, columns_to_fit,
};
pub use store::{Lock, Store, StoreError};

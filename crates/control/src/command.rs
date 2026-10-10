//! What can be asked of Olio.

use olio_library::{SetId, TileId};

/// One request from a control surface. Tiles and sets are named by id, so a
/// command means the same thing whichever set is in front.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Start the tile if it is silent, stop it if it is playing: a key
    /// press, a double-click, a Stream Deck button.
    ToggleTile(TileId),
    /// Start the tile from the beginning (again, if it is playing).
    PlayTile(TileId),
    StopTile(TileId),
    /// Stop everything at once.
    StopAll,
    /// Fade everything out over a couple of seconds, then stop it.
    FadeAll,
    /// Set the master volume, 0 (silent) to 1 (full).
    SetMasterVolume(f32),
    /// Bring a set to the front, so its tiles answer the keyboard.
    SelectSet(SetId),
    /// Change one of a tile's settings. Looping and reverb also change a
    /// voice that is already playing; the rest apply from the next play.
    SetTile(TileId, TileSetting),
}

/// A tile setting and its new value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TileSetting {
    Looping(bool),
    TrimSilence(bool),
    /// Seconds.
    FadeIn(f32),
    /// Seconds.
    FadeOut(f32),
    /// 0 (dry) to 1.
    Reverb(f32),
}

# Olio

Olio is a soundboard: sets of tiles laid out on the keyboard, each playing
a sound, for live and recorded shows.

## Language

### The library

**Library**:
Everything a user has made: their sets, tiles and imported sounds, kept in
one folder.
_Avoid_: project, session, database

**Set**:
A named page of tiles laid out on the keyboard grid. One set is in front at
a time; its tiles answer the keyboard.
_Avoid_: board (except a board file), bank, page, scene

**Front set**:
The set shown in the window, whose tiles the keys play.
_Avoid_: active set, current page

**Tile**:
One sound on a set, with its title, colour, key and playback settings.
_Avoid_: button, pad, cue, cell

**Slot**:
A position in a set's grid (row, column). Rows 0 to 3 follow the keyboard's
rows; rows 4 to 7 repeat them with Option (Alt). Dragging a tile to
another slot moves it (and so changes its key); onto another tile, the
two swap. Dropped on another set, a tile takes the same slot there if it
is free.
_Avoid_: cell, position (alone), index

**Key**:
The keyboard key (with or without Option/Alt) that plays the tile in a slot
of the front set.
_Avoid_: hotkey, shortcut (for tile keys)

**Hotkey**:
A system-wide shortcut that works when Olio is in the background (planned).
_Avoid_: key (for these)

**Sound**:
An audio file the library holds a copy of, named by a hash of its contents.
_Avoid_: file (in the UI), sample, asset

**Board**:
A JSON file describing sets and tiles, written by hand or a script and
imported in one go (`docs/BOARD-JSON.md`).
_Avoid_: manifest (in the UI), preset, template

**Import**:
Bringing sounds into the library: files into a set, a folder as a new set,
or a board.
_Avoid_: load, add (for folders and boards)

### Playback

**Clip**:
A sound decoded into memory, ready to play.
_Avoid_: buffer, sample

**Voice**:
One playing instance of a clip. Pressing a tile's key starts a voice;
stopping the tile fades its voice out.
_Avoid_: channel, track, instance

**Region**:
The frames of a clip a voice plays and loops over.
_Avoid_: selection (reserved for the editor), range

**Normalize**:
A tile setting, on by default, that plays the sound at a common loudness
(-18 dBFS RMS of what plays) with its peak kept under -1 dBFS. A gain on
the voice; the file is unchanged.
_Avoid_: compress, level (alone), auto-gain

**Spectrogram**:
The strip under a tile's waveform showing how the sound's energy spreads
over frequency as it plays, low at the bottom.
_Avoid_: spectrum (one moment, not over time), EQ

**Undo**:
Reversing the last change to the board (⌘Z): a removed tile or set comes
back exactly as it was, a duplicate goes away, a moved tile goes back.
Redo makes the change again. Settings changed in the inspector are not
recorded.
_Avoid_: restore, trash

**Spring-loading**:
Holding a dragged tile over a set in the sidebar until the set opens, to
drop the tile on one of its slots. A drag that ends without a drop goes
back to the set it started in.
_Avoid_: hover-open, auto-open

**Duplicate**:
A copy of a tile, with the same sound and settings and " copy" after its
title, in the next free slot (or where it is dropped).
_Avoid_: clone

**Trim silence**:
A tile setting that makes its region the audible part of the clip, skipping
silence (below -60 dBFS) at the start and end.
_Avoid_: auto-crop, strip

**Fade in / Fade out**:
Tile settings in seconds: the voice rises from silence at the start, and
sinks to silence before the region ends and whenever the tile is stopped.
_Avoid_: ramp (for these), attack, release

**Reverb**:
A tile setting from 0 to 1: how much of the tile is sent to the mixer's
shared room reverb.
_Avoid_: echo, wet (alone)

**Mixer**:
The code on the audio thread that adds every voice and the reverb into the
output.
_Avoid_: engine (for this part)

**Engine**:
The app's handle on the mixer: starts and stops voices and hears when they
end.
_Avoid_: player, backend

**Stop All**:
Stop every voice at once (button, Cmd-Esc, or hold Esc).
_Avoid_: panic, kill

**Fade All**:
Fade every voice out over two seconds, then stop it (button, or `-`).
_Avoid_: fade out all, dim

**Master volume**:
The level of the whole mix, with a mute; a preference, not part of the
library.
_Avoid_: main volume, output level

**Generator**:
The panel, in place of the inspector, where gentle music is made up: a
key, a character, a background, instruments, presence and form. It plays the
ambience live and generates tracks.
_Avoid_: composer (that is the code that picks the notes), synth

**Ambience**:
The Generator's music playing live behind the sounds, made up as it goes
on the audio thread. Not a sound or a tile; a track is how to keep it.
_Avoid_: background music (as a name), soundtrack, loop

**Track**:
A sound the Generator rendered into the library and put on a tile, named
after its style and length ("Rain Study 2 min"). From then on it is a
sound like any other.
_Avoid_: recording, export, song

**Background**:
The sound under the generated music: rain, thunder, wind, winter, waves,
a forest, a desert, a fire, or none.
_Avoid_: weather (only some are), noise, ambience (that is the whole)

**Style**:
A named preset of Generator settings (Rain Study, Seaside Piano,
Campfire…). Picking one fills everything in; changing anything makes the
settings Custom. Custom settings can be saved as a style of one's own
(Save Style…), listed after the built-in ones, renamed or removed, and
kept in the preferences; tracks made from it are titled after it.
_Avoid_: preset (that is Polish), theme

**Presence**:
How noticeable the ambience is: Background, Ambient or Declarative.
_Avoid_: intensity, volume (that is level)

**Character**:
How the Generator's lines move, as three amounts from 0 to 100 %:
**Variety** (how often a motif comes back, against new melodies and
chords), **Concentration** (how closely the notes gather around the chord
and the home note) and **Chaos** (dissonance: notes outside the key,
clashes and clusters). A **Mood** names a place for all three.
_Avoid_: complexity, randomness (for chaos)

**Mood**:
A named character: Still, Gentle (where every style starts), Wandering,
Restless or Unsettled. Picking one sets Variety, Concentration and Chaos;
moving any of them leaves the mood Custom.
_Avoid_: feel, vibe, character preset

**Details**:
The Generator's switch between the essentials (style, mood, background,
which instruments play) and every setting.
_Avoid_: advanced, expert mode

**Bubble**:
A playing tile's button in the toolbar, which stops it.
_Avoid_: chip, badge

**Emoji**:
The one emoji a tile can show beside its title, chosen in the emoji
picker or set by a board.
_Avoid_: icon, sticker

**Level meter**:
A bar showing how loud something is playing, in dBFS (0 is full scale):
the output's in the toolbar, with a clip lamp, and each tile's own,
before the master volume.
_Avoid_: VU meter (it is a peak meter), volume (that is a setting)

### Control

**Command**:
A request from any control surface (keyboard, mouse, MIDI, OSC, Stream
Deck, automation), carried out in one place.
_Avoid_: action, message (reserved for engine-to-mixer messages)

**Control surface**:
Any way of sending commands: the keyboard, a MIDI controller, an OSC app,
a Stream Deck.
_Avoid_: controller (alone), device (reserved for audio devices)

**Inspector**:
The right-hand panel showing the selected tile's settings.
_Avoid_: properties, sidebar (that is the sets list)

**Menu action**:
A request that concerns only the window (show a panel, open a dialog,
move the selection), as opposed to a command. Listed in the menu bar.
_Avoid_: command (reserved for what control surfaces send)

**Polish**:
The preset of fade in, fade out and reverb a tile gets in one click
(1 s, 1 s, 30 % unless changed); clicking again takes it off.
_Avoid_: preset (alone), effect chain, enhance

**Tile size**:
How big tiles are drawn: Compact (title, key and time only, so more fit
on screen), Regular, or Large. A preference, not part of a set.
_Avoid_: zoom (that scales the whole window), density

**Columns**:
How wide a set's grid is, 1 to 10: the keys of each keyboard row it
uses (4 columns: 1–4, Q–R, A–F, Z–V). Never narrower than its
right-most tile. Fit to Window sets it once to as many as fit the window
at the tile size; it does not follow later resizes, so keys stay put.
_Avoid_: width (alone), grid size

**Tile style**:
How tiles are drawn: Flat (outlined cards) or Soft (raised buttons that
press in while they play).
_Avoid_: theme (that is light or dark), skin

**Preferences**:
Window settings kept beside the library in `preferences.ron`: theme,
master volume, which panels show, single-click playback, tile style, the
Polish preset, and the Generator's settings and saved styles.
_Avoid_: settings (reserved for the planned Settings window), config

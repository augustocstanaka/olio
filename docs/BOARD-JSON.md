# Board JSON

A board is a JSON file that describes sets of tiles. Write one by hand or
generate it from a script, then import it:

- in the window, with **Import → Board (JSON)…** or by dropping the file
  on the window;
- from a terminal or a batch file, without opening the window:

```bash
olio import show.json
```

`olio import` exits with 0 when everything was imported, 1 when something
could not be (it still imports the rest and lists each problem), and 2 when
it could not run at all (bad arguments, or the library is open in Olio).
Quit Olio first: while the window has the library open, `olio import`
refuses to touch it, because the window would save over the import. Pass
`--library DIR` to import into another library.

## A first board

```json
{
  "sets": [
    {
      "name": "Show Intro",
      "tiles": [
        "sounds/applause.wav",
        "sounds/drum roll.mp3",
        { "file": "music/theme.mp3", "key": "Q", "loop": true, "fade_out": 2 }
      ]
    },
    { "name": "Sound Effects", "folder": "sfx" }
  ]
}
```

Paths are relative to the JSON file (absolute paths work too). This board
makes two sets: *Show Intro* with three tiles, and *Sound Effects* with
every audio file in the `sfx` folder, in alphabetical order.

## Sets

| Field | Meaning | Default |
|---|---|---|
| `name` | The set's name. Required. | |
| `tiles` | The set's tiles, in order (below). | none |
| `folder` | A folder whose sounds are added after `tiles`, in alphabetical order. | none |
| `columns` | Grid width, 1 to 10. | fits the tiles in about four rows |
| `replace` | `true` replaces the tiles of an existing set with this name, so importing the same board again gives the same result. Otherwise a set with a taken name is added as "Name 2". | `false` |

Use `"replace": true` for boards a script keeps up to date.

## Tiles

A tile is either a path, or an object with a `file` and any of these:

| Field | Meaning | Default |
|---|---|---|
| `file` | The sound: WAV, AIFF, CAF, FLAC, MP3, M4A/AAC, Ogg Vorbis, or a video (MP4, MOV, M4V, MKV) whose audio is used. Required. | |
| `title` | The tile's name. | the file name |
| `key` | The key that plays it: `1`–`0`, `Q`–`P`, `A`–`;`, `Z`–`/`; add `Alt+` (or `Option+`, `⌥`) for the lower four rows. | the next free slot |
| `color` | `purple`, `pink`, `red`, `orange`, `yellow`, `green`, `teal`, `blue` or `gray`. | by column |
| `loop` | Play again from the start until stopped. | `false` |
| `trim_silence` | Skip silence at the start and end. | `true` |
| `fade_in` | Seconds to rise from silence, 0 to 30. | `0` |
| `fade_out` | Seconds to sink to silence before the end and when stopped, 0 to 30. | `0` |
| `reverb` | How much reverb, 0 (none) to 1. | `0` |
| `emoji` | One emoji shown on the tile, written as the emoji (`"🎉"`) or its GitHub shortcode (`"tada"`, `":clap:"`). | none |

Tiles with a `key` take their slot first; the rest fill the free slots in
order, row by row.

```json
{
  "sets": [
    {
      "name": "Quiz Night",
      "columns": 4,
      "replace": true,
      "tiles": [
        { "file": "correct.wav", "key": "1", "color": "green" },
        { "file": "wrong.wav", "key": "2", "color": "red" },
        { "file": "clock.mp4", "key": "3", "title": "Thinking Music", "loop": true, "fade_in": 1, "fade_out": 1.5 },
        { "file": "winner.flac", "key": "Q", "reverb": 0.4, "emoji": "trophy" },
        { "file": "applause.wav", "key": "Alt+1", "fade_out": 3 }
      ]
    }
  ]
}
```

## Mistakes

Olio checks the whole file before importing anything from it. A typo in a
field name, a colour it does not know, or a value out of range stops the
import with a message naming the line, the set or the file:

```text
problem: show.json: unknown field `loops`, expected one of `file`, `title`, `key`, `color`, `loop`, `trim_silence`, `fade_in`, `fade_out`, `reverb`, `emoji` at line 6 column 41
```

A missing sound file or a key that is not on the grid is reported, and the
rest of the board is still imported.

## Generating boards from a script

Any language that writes JSON works. A Python example that turns a folder
of numbered cues into a set with keys in order:

```python
import json, pathlib

keys = list("1234567890QWERTYUIOP")
cues = sorted(pathlib.Path("cues").glob("*.wav"))
board = {"sets": [{
    "name": "Cues",
    "replace": True,
    "tiles": [{"file": str(cue), "key": key} for cue, key in zip(cues, keys)],
}]}
pathlib.Path("cues.json").write_text(json.dumps(board, indent=2))
```

```bash
python3 make_cues.py && olio import cues.json
```

## Importing folders

Folders do not need a board. Drop a folder on the window, choose
**Import → Folder as Set…**, or run `olio import FOLDER`. Each folder that
holds sounds becomes a set named after it, sounds in natural alphabetical
order ("Track 2" before "Track 10"), and each subfolder with sounds becomes
another set. A folder with more than 80 sounds fills several sets.

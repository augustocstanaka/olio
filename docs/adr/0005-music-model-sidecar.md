---
status: superseded by 0007
date: 2026-10-31
---

# The music model: ACE-Step 1.5 as a helper process, installed by choice

The Generator makes up gentle music from settings. Users also want music
from a few words ("warm piano over soft rain, slow"), which takes a
trained model. Olio keeps its promise that nothing leaves the computer,
so the model runs locally.

## Decision

- **ACE-Step 1.5** is the music model: MIT code and weights, music to be
  used freely, 10 s to 10 min of stereo 48 kHz from a prompt, an
  instrumental mode, Apple Silicon through MLX, Windows through CUDA or
  the CPU, and a local REST API. Stable Audio Open Small (11 s, a
  sound-effects bias, a gated licence), Magenta RealTime (streaming, no
  file render, CC-BY), MusicGen (non-commercial) and YuE (CC BY-NC,
  24 GB) were turned down.
- It runs as a **helper process**, not in the app: ACE-Step's own API
  server, in a Python environment that uv makes from ACE-Step's lock
  file (`uv sync --frozen`), with a Python uv installs itself. There is
  no Rust runtime for this model with an MLX path, and a process can be
  stopped, which frees its memory, and cannot take the window down with
  it.
- **Pinned**: one ACE-Step tag, one uv release, one revision of the
  weights on Hugging Face, each file's size and SHA-256 in
  `music_model/mod.rs`. What is installed is what was tested.
- **By choice**: nothing is downloaded until the user presses Install
  Music Model (or runs `olio music-model install`). The install is
  resumable and lives in `MusicModel` in the per-user data folder,
  shared by every library; Remove Music Model takes it all away.
- **Only the diffusion model**, no language model (`thinking: false`,
  `ACESTEP_INIT_LLM=false`): about 6 GB of memory, which a 16 GB laptop
  can spare beside the app. Prompted tracks are instrumental (`lyrics:
  "[Instrumental]"`).
- **Local and keyed**: the server listens on 127.0.0.1 on a free port,
  with an API key made for each start; the app spawns the environment's
  Python directly (not `uv run`), so stopping it stops the model.
- **Started when needed, stopped when idle**: the first prompted track
  starts it (and waits for the model to load, a few minutes the first
  time); it stops after ten minutes unused, on Cancel while it loads,
  on Remove, and when the app quits.
- **In the Generator, with its words**: a From a prompt section, the
  same Length picker, progress bar, Cancel and tile placement as other
  tracks; "music model" and "prompt" in the UI, never AI, LLM or the
  model's name; the title of a track is its prompt and length.

## Consequences

- About 10 GB on disk, 6 GB of downloads, and a few minutes to install.
  The button says so.
- CI never runs the model: tests speak to a fake of its API
  (`music_model::client::fake`), install fixtures stand in for the
  archives, and the server tests use `sleep`. The real model is tried by
  hand, on macOS; Windows compiles and is untested here.
- A crash of the app can leave the helper running; its command line
  names Olio's environment, so it can be found and stopped. A `pid` file
  and a check at start are a follow-up.
- The small archives (uv for each platform, ACE-Step's source) are
  mirrored as assets of the Olio release `music-model-1`, like the
  instrument pack: one origin, one hash table, bytes that never change.
  The weights (6 GB, above GitHub's 2 GB cap on an asset) come from
  Hugging Face at a pinned revision.
- Progress from the model is not reported, so a prompted track shows an
  animated bar and the time passed, not a percentage.

---
status: accepted
date: 2026-11-02
---

# The Generator stays procedural: no music model

Supersedes ADR 0005.

ADR 0005 added a music model (ACE-Step 1.5, run locally as a Python
helper process) so a track could be made from a prompt. In use it asked
for prompts in the model's own tag language, its output was hard to
predict or steer, it took about 10 GB on disk and 4 to 8 GB of memory, and
a dependency the size of a Python machine-learning stack sat beside a
small Rust app. The user trusts music made by rules they can see and
shape more than a model's.

## Decision

- Remove the music model entirely: the `music_model` module, the From a
  prompt section, the bed under a prompted track, `olio generate
  --prompt` and `olio music-model`, and their preferences. Preferences
  saved with a prompt still load; the fields are ignored.
- Everything the Generator plays is made by rules from its settings:
  scales, chords, motifs, density, register, journey, backgrounds, played
  by synthesised instruments and recorded ones from the instrument pack.
- Make the Generator richer the same way: better free instruments in the
  pack (CC0 or CC-BY recordings, no non-commercial or no-derivatives
  licences), engines and effects written in or linked from MIT/Apache/BSD
  code, and generative techniques (co-prime loops, voice leading,
  layering, slow modulation, convolution with free impulse responses).
  No GPL or LGPL code is linked into the app.

## Consequences

- Olio downloads at most the instrument pack (about 148 MB), never a
  Python environment or model weights.
- The `music-model-1` GitHub release and any installed `MusicModel`
  folder are no longer used by the app; removing them is a separate,
  manual step.
- Whether a track sounds good stays testable and repeatable: the same
  settings and seed render the same track, and offline render tests can
  check its loudness, spread and density.

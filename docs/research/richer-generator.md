# Making Olio's Generator richer without a music model

Research from 2026-11-02, after ADR 0007 removed the music model: seven agents read Olio's Generator and swept free instruments, engines and techniques, with every external source's licence checked against its own terms. Licences change; recheck one before pinning it.

## 1. Why it sounds thin today

The thinness comes mostly from how the music is written and mixed. The instruments matter less.

- **The harmony moves in blocks.** With a pulse, `composer.rs` walks the fixed `CADENCE` (I IV V I VI IV V I) forever. In free time, `pick_chord()` is a memoryless weighted draw. `pad()` plays every chord as a root-position triad, so every voice jumps at each change. There are no inversions, no common tones held, no open voicings and no borrowed chords. `strings()` plays one note, plus a fifth when energy is high enough.
- **Repeated notes sound identical.** Piano, harp, bells, strings and pad have one take per note. Velocity layers switch hard at 0.5 (`crates/packer/src/build.rs`), so at Ambient energy the piano almost never reaches its loud layer. The motifs in `melody()` repeat notes on purpose, so you hear the same sample again and again.
- **The pad sample is stretched.** The bowed vibraphone under the pad has only 6 roots, so notes are shifted several semitones. VCSL Soft Mallets vibraphone is already pinned in `sources.json` but no recipe uses it.
- **The loops can be heard.** `bake_loop()` always loops 40 % to 80 % of the recording with a 0.5 s blend, without matching the pitch period or the vibrato. Strings and pad notes are held 4 to 22 s, so the loop point comes round every few seconds.
- **Everything shares one space.** `Ambience::render` (`mod.rs`) sends the whole bus through one `Chorus`, bass and drums included, which muddies the low end. It then goes into one `Hall` at a fixed `HALL_SEND` of 0.6. No layer is near or far, there are no early reflections, and the drone is mono and dead centre.
- **Nothing breathes.** Apart from the Build swell, energy, density, levels and brightness never move (`journey.rs`). Evolve makes one discrete step every 2 to 4 minutes. A key change releases held notes and restarts on I (`Ambience::follow`, `Composer::modulate`).
- **The lines don't relate to each other.** Harp, piano and violin are independent random walks. There is no call and response, no imitation and no phrase structure. Free time is random gaps, and pulsed music has no rhythmic patterns.
- **The synthesised voices are basic** (`synth.rs`). Bells have 4 sine partials and no strike. The piano has 5 partials. Pad and strings use a single 6 dB/oct one-pole filter. Without the pack it sounds like a sine demo.
- **Voice stealing clicks.** `Sampler::play` overwrites the oldest of its 40 voices with no fade, and the synth pools do the same.
- **The bus has no glue.** There is no EQ, saturation or compression on the bus, and offline tracks only get RMS normalisation and a limiter (`track.rs`).

## 2. Instruments to add to the pack

Every source here passed the licence audit. They are ranked by what they add to ambient music.

**1. Unused parts of VCSL and VSCO-2-CE (CC0-1.0).** This is the biggest gain for the least work.
- **Sources:** https://github.com/sgossner/VCSL and https://github.com/sgossner/VSCO-2-CE. Both are already pinned at the same commits, so there is no new licence review and no new fetch code. You only add file entries with their sha and new recipes in `recipes.rs`.
- **Order to add them:**
  - Vibraphone Soft Mallets: already pinned, 42 MB of source WAV, 11 notes × 2 layers.
  - Wine Glasses and Bowed Psaltery: bowed glass and bowed metal sustains.
  - Kalimbas and mbiras: gentle ostinatos. Mbira tunings are not equal-tempered, so retune them or tag them as fixed-scale.
  - Glockenspiel, Tubular Bells and Nepalese Hand Bells.
  - Dan Tranh: a koto-like pentatonic pluck.
  - Folk Harp: a softer harp.
  - Pipe and Renaissance organ: drones.
  - Flute and clarinet from VSCO-2-CE: breathy slow lines.
  - Soft F Horn: warm swells.
  - Gongs and Mark Trees: swells.
  - Ocean Drum: a recorded wash.
- **Size:** a trimmed subset adds tens of MB.
- **Limits:** notes are sampled sparsely, about every minor third, with 1 to 3 dynamics and few round robins. Rooms differ between instruments.

**2. Karoryfer free libraries (CC0).**
- **Source:** https://shop.karoryfer.com/pages/free-samples. The publisher says all its free libraries are CC0 except Marie Ork, which you must leave out.
- **What they add:** Bigcat Cello for solo cello lines over pads, D. Smolken double bass for low bowed drones, and String Cyborgs for textural strings. This fills the gap of strings with only one articulation.
- **How:** pin the sfzinstruments GitHub mirrors by commit. Check the licence file inside each archive, because older downloads may carry an older licence.
- **Uncertain:** the sizes were not listed (Sneakybass alone is 324 MB).

**3. A soft or felt-style piano (choose one or two).**
- **Upright Piano KW, FreePats (CC0).** https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html. 32 MiB SFZ+FLAC at best quality or 2.9 MiB small, 2 layers, all 88 keys. It is the cheapest warm upright. Its soft layer, with a low-pass, gives a lo-fi felt piano.
- **Osiris Piano (CC0).** https://github.com/sfzinstruments/Osiris_Piano. The "noisy" variants keep the mechanism noise and the Sus variants have the pedal down, which suits intimate ambient. Its size and layers are unknown until you download it.
- **Salamander Grand V3 (CC-BY 3.0, credit Alexander Holm).** https://github.com/sfzinstruments/SalamanderGrandPiano. 16 velocity layers; 707 MiB as SFZ+FLAC. Taking only its 2 to 4 softest layers gives a beautiful soft grand for roughly 40 to 80 MB. Accurate-Salamander (CC-BY 3.0, credit C. Yamauchi as well) is a more even version, but it is 1.6 GB.
- **Headroom Piano (CC-BY 4.0, credit Bengt Nilsson).** https://github.com/sfzinstruments/BengtNilsson.HeadroomPiano. 156 MB, a mellower Yamaha C3.
- **VCSL Keys (CC0).** https://versilian-studios.com/vcsl-keys/. 680 MB covering a Steinway B, uprights and harpsichords. Pin from the VCSL repo first, where the folders may already be.

**4. University of Iowa MIS.**
- **Source:** https://theremin.music.uiowa.edu/MIS.html. The terms say the samples "may be downloaded and used for any projects, without restrictions". This is not a named licence, so keep a copy of the statement in CREDITS and consider asking them for written confirmation.
- **What it adds:** chromatic, dry recordings at pp, mf and ff. There are alto and bass flute (breathy low winds, standing in for shakuhachi), bowed vibraphone, bowed crotales and pp piano.
- **Work:** pin each file by URL and sha256. Some files hold whole chromatic runs, so the packer needs an onset splitter.

**5. MuseScore_General SoundFont (MIT).**
- **Source:** https://ftp.osuosl.org/pub/musescore/soundfont/MuseScore_General/. 38 MB as sf3, 206 MB as sf2.
- **What it adds:** the only clean source found for choir (Choir Aahs, Voice Oohs), and also Music Box, Koto, Shakuhachi, Celesta and Bowed Glass. The quality is General MIDI level, which is fine as a pad buried in the hall.
- **Conditions:** skip its Grand Piano preset, which uses the AKAI samples with unverified provenance. Check `Sample_Sources.csv` for each preset you extract. Keep the MIT notices in CREDITS.
- **Work:** an SF2 extractor in the packer, offline only. rustysynth's parser is MIT.

**6. Freesound, sound by sound (CC0, or CC-BY with credit).**
- **Source:** https://freesound.org
- **What it adds:** singing bowls, music box, shakuhachi, and real rain, stream and fire beds to layer with `background.rs`.
- **Work:** a "freesound" source kind in `sources.json` (sound ID, uploader, licence, sha256). The packer re-checks each licence at build time and drops anything BY-NC or Sampling+. Originals need an API token at build time only, never in the app.

**7. AKWF single-cycle waveforms (CC0 / public domain).** https://github.com/KristofferKarlAxelEkstrand/AKWF-FREE. Tiny. They are source material for a wavetable pad voice.

**Changes the packer and pack format need for all of this:**
- Format v2 in `instruments.rs`, with v1 packs still loading. It adds round-robin groups, velocity crossfades, an articulation index, per-zone attack and release, a licence field per instrument, and impulse-response and texture entries.
- A CREDITS file generated from those licence fields, also shown in the About box.
- Archive fetching (URL plus sha256, then unzip) in `fetch.rs`, for FreePats, Karoryfer and MuseScore.
- Better loops in `bake_loop()`: choose the loop by autocorrelation and use a longer crossfade.
- Mono kept as mono, and per-instrument peak normalisation replaced so register and dynamic differences survive.
- A size budget per instrument, because the pack is about 71 MB today.

**Rejected, all for licence reasons:**

| Source | Why it is out |
|---|---|
| Splendid Grand, and the MuseScore piano preset | The public-domain claim has no primary statement from AKAI |
| GeneralUser GS | The author says sample provenance is uncertain |
| Sonatina Symphonic Orchestra | CC Sampling Plus 1.0, plus provenance doubts |
| Philharmonia samples | They may not be made available "as is" as samples or an instrument |
| Pianobook | Redistribution is forbidden |
| Spitfire LABS | Proprietary EULA |
| Virtual Playing Orchestra | Mixed in Sampling+, BY-SA and Philharmonia material |
| jRhodes3c and jRhodes3d | NC |
| LivingRoom Upright | CC-BY-SA, and not needed |
| Dave Choir | No licence stated |
| Sonniss GDC bundle | Forbids redistribution in a sample pack |
| EchoThief IRs | No redistribution grant |
| Voxengo IRs | Must be shipped complete and unaltered, plus a "no indirect profit" clause |
| OpenAIR as a whole library | Licence is set per IR and may be NC or ND. Single IRs could come back after each page is checked |

**Still open:** there is no cleared source of real impulse responses. Until one is found, convolution would use CC0 IRs from Freesound, checked one by one, or IRs rendered by Olio itself.

## 3. Engines and effects to build in

All of these are under MIT, Apache, BSD or public domain, or are algorithms you write yourself. The rule for all of them is the same: allocate and plan in `new()` or at pack load, and never on the audio thread.

| Engine | Licence | Where it goes | Audio-thread notes |
|---|---|---|---|
| Your own modal and extended Karplus-Strong voices (bells with 6–12 inharmonic partials, kalimba, singing bowl with beating partial pairs, pick-position pluck), after J.O. Smith's PASP | own code | `synth.rs`, plus a resonant SVF in `dsp.rs` | Fixed partial arrays. Flush denormals |
| Your own granular cloud over the pack's sustained regions | own code | new `grains.rs` reading sampler buffers. Pool of 32–64 grains, windows from a precomputed table | Drop grains when the pool is full, never grow it |
| Shimmer in the hall (2-head delay-line octave shifter fed back into the FDN) | own code | `hall.rs` | Preallocated delay line. Keep loop gain below 1 with a soft limiter |
| Early reflections, per-layer send and distance filtering | own code | `hall.rs`, and the mixdown in `synth.rs` and `sampler.rs` | Fixed taps |
| Dattorro plate as a second space | algorithm from the paper; the Faust `dattorro_rev` is STK-4.3 | `hall.rs`, chosen per style | Delay lines sized for the highest sample rate |
| fundsp | MIT OR Apache-2.0, v0.23.0 | As a toolkit or a reference for SVF, Moog filter, FDN and PolyBLEP | Run `allocate()` before handing it to the mixer. Use static nodes, never edit a `Net` on the audio thread. 0.x API changes |
| mi-plaits-dsp (Plaits 1.2 port) | MIT; upstream STM32 code is MIT | `synth.rs`: modal, string, FM, chord and particle voices | Git-only, so pin by commit. Allocation behaviour is not documented, so check it. Fixed internal rate, so you may need to resample |
| Rings, Elements and Clouds, ported from pichenettes/eurorack STM32 folders | MIT | Rings and Elements as `synth.rs` voices; Clouds as the granular layer | Fixed buffers. Never take code from the AVR folders (GPL-3) |
| Airwindows ports (tape, console, saturation) | MIT | A tape and glue stage on the ambience bus in `mod.rs`, used live and in `track.rs` | Small fixed state. Scale for the sample rate |
| Partitioned convolution with realfft (MIT) and rustfft (already in Cargo.lock) | MIT | `hall.rs` "Space" option | Plan FFTs at IR load. Swap IRs lock-free. Waits on an IR source (see section 2) |
| rustysynth | MIT, v1.3.8 | Optional SF2 sampler backend | Check that `render()` and a `note_on` with a full pool don't allocate. Extracting SF2 into the pack is safer |
| Faust (`-lang rust` in an xtask) | output takes the licence of the `.lib` files used | Generated voices and reverbs checked in under `ambience/faust/` | Check every function: jpverb and greyhole are MIT; `vital_rev` and `kb_rom_rev1` are GPL. The sweeps disagree on whether zita sits under GPL or STK-4.3, so read the header before using it |

Kept by the audit but not recommended: dx7 (MIT, but version 0.0.4 and it ships a patch file of unclear origin), sfizz (BSD-2, but archived since June 2026 and needs a C++ build), glicol, knyst, twang and dasp (permissive but dormant; read them for ideas). Excluded: oxisynth (LGPL), nih-plug's VST3 path (GPLv3), and Surge XT, Vital, Dexed and Odin 2 (GPL-3). You may study their ideas but not copy their code.

**Hosting user-installed VST3 or CLAP plugins: legally possible now, but a later, opt-in feature.** clack-host and clap-sys are MIT/Apache. Steinberg moved the VST 3 SDK to MIT with VST 3.8, and the coupler `vst3` crate is MIT/Apache. Plugins the user installed themselves are never redistributed, so GPL plugins like Surge or Vital are fine to host.

It should be offline only. It would run as an `olio plugin-host` child process fed by the composer's note events, with the result mixed into `track.rs`'s WAV. It should never run on the cpal callback, because plugins may allocate, hang or crash.

The costs:
- Weeks of host work, mostly VST3 COM boilerplate.
- Plugin windows next to egui.
- Copy-protected plugins may not render headless.
- Tracks stop being reproducible from a seed on other machines.

It also cuts against the procedural direction, so build it after the in-house engines.

## 4. Techniques, ranked by impact per effort

1. **Voice leading and wider voicings (M, `composer.rs`).**
   - Keep the pad's current voicing.
   - For each new chord, try its inversions and choose the one with the least total movement, holding common tones.
   - Spread voicings wide in the low register.
   - Add 6, 9 and 11 colours below Vibrant energy.
   - Let `Sustained` voices glide by step. This gives the largest "composed, not generated" change.
2. **Slow modulation on every layer (S, new `modulation.rs`, `journey.rs`, `synth.rs`, `sampler.rs`).**
   - Level, filter cutoff, pan and send drift on unrelated prime periods (37, 53, 71, 97 s), plus 1/f random walks.
   - Read them once per block.
   - Keep bass and drone centred.
3. **Per-layer space: chorus removed from bass and drums, sends, distance, early reflections, stereo drone (S–M, `mod.rs`, `hall.rs`, `synth.rs`).**
4. **Eno loops of co-prime lengths and Bloom-style echoes (S–M, `composer.rs`).**
   - Each melodic instrument loops a fixed phrase on its own period (about 17 to 32 s), with chord-relative degrees.
   - Notes echo 1 to 4 times, getting softer. The existing queue already drops events when full.
   - The variety slider sets the balance between looping and wandering.
5. **Markov chord and step tables, plus modal interchange (S–M, `composer.rs`, `mod.rs`).**
   - Per-mood `[[f32;7];7]` const tables replace `pick_chord()`'s flat weights and the fixed `CADENCE`.
   - Melodies get a step table with gap-fill after leaps.
   - Borrowed chords (iv, bVI, bVII) are held over the drone.
6. **Voice-steal fades and sampler humanising (S, `sampler.rs`, `synth.rs`).**
   - Fade stolen voices out quickly instead of cutting them.
   - Add small detune and start-offset variation.
   - Add a per-voice one-pole that follows velocity.
7. **Shimmer and a tape stage (S–M, `hall.rs`, `mod.rs`).**
   - Octave shimmer by default.
   - Gentle wow and flutter, soft saturation, a high roll-off, and gain compensation.
8. **Richer synthesised voices (M, `synth.rs`).** Modal bells, a bowl, a better pluck, a multi-voice pad with an SVF and filter envelope, and a just-tuned drone with slow beating. Just tuning is limited to fifths and octaves when the pack is loaded.
9. **Tintinnabuli T-voice and upper pedals (S, `composer.rs`).**
10. **Sections and pivot-chord key changes (M, `journey.rs`, `Ambience::follow`, `Composer::modulate`).**
    - Sections set targets for density, brightness and register and blend over 20 to 60 s.
    - A resting instrument fades out instead of being switched off.
11. **Euclidean patterns and phasing for pulsed styles (S, `composer.rs` `on_pulse`/`grid`).**
12. **Granular clouds and spectral freeze (M–L, new `grains.rs`).** Write the freeze from the Paulstretch algorithm's description; its code is GPL. Pre-render frozen beds when the chord changes, or offline.
13. **L-systems and cellular automata (S–M).** They add flavour only, after everything above.

## 5. Roadmap

**Milestone 1: Writing and mixing (no new samples).** Items 1 to 6 from section 4: voice leading, modulation, per-layer space, loops and echoes, Markov tables and borrowed chords, and steal fades.
- **Done on 2026-11-02 (items 1 to 3):**
  - Voice leading in `composer.rs`: the pad plays a low root, three upper voices led from the last chord (common tones held, each voice moving a fourth at most and eight semitones in all), and a ninth or eleventh over major and minor chords below fuller energy. A note still held is carried on (`Note::from`): the same note is held longer, a step away glides there, a leap lets go.
  - Slow modulation in the new `modulation.rs`: level, brightness, pan and hall send drift for every layer on 36 prime periods (31 to 199 s) plus a smoothed walk, worked out every 64 frames of the music's clock. The bass, drone and drums keep to the middle.
  - Per-layer space in `hall.rs`: the synth and sampler hand back one stem per layer; each has a place (dry, send, chorus or not, distance); the bass, drone and drums skip the chorus; the hall gained early reflections; the drone is stereo. `MUSIC_TRIM` went from 0.72 to 0.8 to keep the level (within about 1.5 dB per style).
  - Each voicing has a number (`Note::voicing`); a carried note is matched to a voice that voicing has not struck or carried yet, takes the new note's level over about a second, and is only held on in a recording that loops. A voice the new voicing leaves out is let go (a carried note with no duration). The pad no longer voices the old chord just before a change, and a change drops the old chord's notes still queued; the composer counts a pad note as sounding only once it is handed out, so a dropped note is never led from. Switching the pad off forgets its voicing, and the end of a piece lets held notes go rather than hold them through the pause. The colour is left out when the voicing has climbed too high to fit it above.
  - Tests: voice movement bounds, carried notes (each voice once per voicing, the new level, looped or not, and eight minutes of composer and synth together holding nothing from an older chord), the colour above the chord with no doubled note, the places, a seed rendering the same twice, and a counting allocator around `render` with synthesised and recorded instruments (`crates/audio/tests/no_alloc.rs`).
- **Done on 2026-11-03 (items 4 to 6):**
  - Loops and echoes in `composer.rs`: a repeated motif is a loop, its rhythm stretched so a pass lasts a whole multiple or whole fraction of the line's loop length (harp 17 s, piano 23 s, violin 29 s), so the lines drift against each other; the stretch is the nearest whole multiple or fraction in ratio, about 40 % at most, and the harp starts no rising phrase while it loops. Half the notes of a loop echo one to three times, half as loud each time, from side to side (a beat apart on the pulse). Variety sets the balance as before: the lower it is, the more often a motif comes back as a loop.
  - Chord tables: seven-note scales choose the next chord from a table per chord, functional on the pulse (the dominant goes home, ii and IV lead to V) and drifting in free time (plagal as often as authentic); the character leans on them as it leaned on the old weights. The next chord is never the same one again (suspended or not, with chaos or not), nor one whose fifth is diminished, but for the tonic. Other scales keep the old walk. From variety 0.7 (Wandering, Restless, Unsettled) a chord is now and then borrowed from the parallel key (iv, bVI, bVII in a major mode; IV, V in Aeolian or Dorian), everything playing in that key until the harmony returns to I or V. Not when suspended.
  - Melodies fill the gap after a leap with a step back, more often as concentration rises.
  - Stealing: a recorded voice taken for a new note fades out on a spare voice over 50 ms; the synthesised pad and strings take the quietest voice already let go, else the held one nearest its end (never one just bowed). Recorded notes are detuned a few cents, held ones start a moment in, and softer ones sound darker.
- **Milestone 1 is complete.**

**Done on 2026-11-05 (Milestone 2, as role and voice):** the plan allowed either a role plus pack-id scheme or new `Instrument` variants; roles and voices won, because a voice needs no new composer part, no new layer, and saved settings keep working (`Layer::voice`, 0 when absent).
- **Pack format 2** (`instruments.rs`): each entry has a `voice`, a `name`, a `license` and a `credit`, a velocity `blend` and a playable `range`; version 1 packs still load as one voice of each. The packer refuses a pack with a voice lacking a name, licence or credit, or without a voice 0, or over 150 MB.
- **Sampler:** a note between two velocity layers plays both at equal power (the lines blend by up to half a layer); notes outside a voice's range move by octaves into it; carried held notes carry all their voices. Round robins were already there: zones that overlap take turns, but these libraries record one take of most notes.
- **Loops:** each held note is looped inside its steady part (from where it reaches 70 % of its loudest to where it last holds 50 %, never in the ring-out after the bow lifts, which the first build did and the first review caught); at least 1.5 s of it, and of a longer one the flattest stretch of the recipe's length (a swelling glass loops where it swells least); the join is chosen where the sound before the end most resembles the sound before the start (correlation, coarse then fine), and blended at constant power by the correlation read every 40 ms along it, since two tones of nearly one pitch drift in and out of step (a long equal-power blend dipped 3 dB on such a pair). The packer refuses a loop under a quarter second, and the ignored whole-pack test fails a loop whose parts differ by more than 10 dB. A bug the new whole-pack test found: a loop that ends where its data ends was dropped just before it wrapped, so held recordings fell silent at their first pass; the installed version 1 pack had it too. The synthesised pad beneath hid it.
- **Voices** (all CC0, from the two libraries Olio already pins; `sources.json` is now written by `olio-packer pin` from the recipes and checked by a test): piano, 3 layers; harp: concert, folk, dan tranh, kalimba; bells: hand chimes, glockenspiel, vibraphone; strings: sections, flute, clarinet, horn; pad: bowed vibraphone, bowed psaltery, wine glasses, organ. Pack: 148 MB, 47 minutes (limit 150 MB; the version 1 pack is 71 MB). The quiet layers stored at the file's full range compress less: the piano alone is 31 MB.
- **Levels:** loudness is a held note's level inside its loop and a pluck's loudest 50 ms (not its tail, which the second review caught as the measure for plucks). Notes of a layer are brought 70 % of the way to the layer's median loudness (the library records some notes 30 to 40 dB apart, from one loud outlier fixing the scale), and the layers of a voice 75 % of the way to each other (and blended where they are wide enough: 0.10 for most voices), each stored at the file's full range with the rest in the zone gain; a soft recording is much quieter than a loud one, and a velocity on top of that made the level step 14 dB at the line between layers and made the blend useless. The first and last recording of a layer stretch at most a fifth beyond their root; beyond that a note is played from the nearest recording of the layer the velocity calls for, unless another layer's is more than five semitones nearer (the horn's loud layer and the bass's had recordings only up to C3 and E1); where one layer alone has a recording near, it plays at full weight at any velocity, with nothing to blend with (the second review caught a dip of 24 dB there). Each voice's gain was then set from the ignored measuring test (four seeds, four minutes), so every voice is within 1 % of its synthesised voice's level, the pad's by the recording's own level against a fixed aim.
- **Tools:** `olio generate --voice harp=2` renders a style with a voice; `every_voice_of_the_pack_plays_every_note` (ignored, `OLIO_PACK`) plays every voice at 25 notes and 3 velocities and checks that each sounds, is finite, does not peak over 6, is not stretched over a fifth among the voice's recordings and holds ten seconds, and that no loop swells or dies away, and that the played level follows the velocity law to within 6 dB at every step of 0.02 (the old pack steps 10 to 21 dB at its layer lines); the measuring test now fails a voice more than 1.5 dB from its level.
- **Review (a workflow of four reviewers and two skeptics each):** it found the glasses' and psaltery's ranges an octave low, the ring-out loops, the stretched horn and bass loud layers, the uneven levels, a gain calibration from one seed, and a failed voice load in a track keeping the previous voice. A second review of those fixes found the glasses' loop 0.15 s long, loops that dipped at the join, the plucks' levels measured in their tails, the glass levels measured outside the loop, the blend dipping where one layer alone covers a note, the track fallback untested, and the whole-pack test too lenient; all fixed, each with a test or a check. Refuted in the first: round-robin share, allocation in the new paths, pack replacement keeping old voices, flute and horn needing ranges, the dan tranh tune across the velocity line.
- **Published** on 2026-11-05 as the release `instruments-2` (148,235,075 bytes, SHA-256 9e3687…89c8); the app downloads it. An Olio that already has the version 1 pack installed keeps it until the pack is installed again (there is no update check).
- **Not done:** Surprise Me does not pick voices; the voices are untested by ear; several pitch readings of low strings and kalimba tines disagree with their file names (the detector, most likely) and play as named.
- **By ear:** run `cargo xtask previews` before and after.
  - Chord changes should sound legato.
  - The mix should keep moving even when notes repeat.
  - The bass should be clean, and no clicks should be heard at Vibrant energy.
- **By test:**
  - Offline `Engine::offline` renders stay identical for a given seed.
  - Each pad voice moves at most N semitones per chord change.
  - Notes stay in the scale, except borrowed or PLR chords, which are behind a gate.
  - Loop periods recur at the expected frames.
  - Bus level stays within bounds after re-checking `MUSIC_TRIM`.
  - An allocation counter (assert_no_alloc style) runs around `render`.

**Milestone 2: Pack v2 from the CC0 sources Olio already pins.**
- **Packer and format work:**
  - Format v2 with round robins, velocity crossfades and licence fields; v1 packs keep loading.
  - CREDITS generation.
  - Better loops, and Soft Mallets vibes in use.
- **New instruments:** Wine Glasses, Bowed Psaltery, kalimba, glockenspiel and hand bells, Dan Tranh, Folk Harp, organ, flute, clarinet and horn.
- **Saving:** a role plus pack-id scheme, or appended `Instrument` variants with serde defaults, so saved layers keep working.
- **By ear:** repeated motif notes vary, held strings and pads have no loop pulse, and the piano reaches its louder layer.
- **By test:**
  - v1 packs still load.
  - Velocity crossfades give the same summed level.
  - Round-robin picks rotate.
  - Every pack instrument has a licence field and a CREDITS line.
  - Pack size stays within an agreed budget, for example 150 MB.

**Milestone 3: New voices and spaces.**
- Modal bells and bowl, the improved pluck and SVF pads in `synth.rs`.
- Shimmer, Dattorro plate and a tape stage.
- A first sample batch from outside: Karoryfer cello and bass, plus Upright Piano KW or Osiris, or Salamander's soft layers if you accept CC-BY attribution.
- Optional: mi-plaits-dsp voices, after checking that they don't allocate.
- **By ear:** with the pack off, the Generator should no longer sound like a sine demo. Far bells sound dark and wet, and the melody sounds close.
- **By test:**
  - Shimmer feedback decays to silence after the input stops.
  - The plate and hall stay bounded.
  - Tape saturation is level-compensated within ±1 dB.
  - CPU per buffer is measured in an offline benchmark.

**Milestone 4: Texture and form.**
- The granular cloud over the pack, and frozen beds.
- Choir and music box from MuseScore_General, plus CC0 or CC-BY Freesound bowls and field recordings.
- Sections and pivot-chord key changes in `journey.rs`.
- Convolution, only once a cleared IR set exists.
- Plugin hosting can be assessed after this milestone, if you still want it.
- **By ear:** a 20-minute render has an audible shape, and key changes glide instead of resetting.
- **By test:**
  - The grain pool never grows, and drops grains when full.
  - Section changes blend over the set time.
  - Key changes keep at least one common tone.
  - Section tests are added beside the existing journey tests.

**Uncertainties:**
- Real-time behaviour of fundsp, mi-plaits-dsp and rustysynth is not verified.
- Karoryfer, Osiris and Iowa sizes and layer counts are unknown until downloaded.
- The Iowa terms are a statement, not a formal licence.
- The Faust zita licence is disputed between the sweeps.
- No cleared impulse-response library has been found yet.
//! Building the pack: each recipe's recordings decoded, checked for
//! pitch, trimmed, held ones looped seamlessly, brought to one level per
//! voice, written as 16-bit FLAC (mono where the recording is), with the
//! manifest and credits (written from the manifest, so every voice has a
//! licence and a line), and zipped with its SHA-256.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use olio_audio::decode_file;
use olio_audio::instruments::{InstrumentManifest, MANIFEST, Manifest, PACK_VERSION, ZoneManifest};
use sha2::{Digest, Sha256};

use crate::fetch::{self, Sources};
use crate::pitch;
use crate::recipes::{RECIPES, Recipe, Source};

/// The loudest note of each voice peaks here.
const PEAK: f32 = 0.89;
/// The shortest loop, in frames (about a quarter second at 44.1 kHz).
const MIN_LOOP_FRAMES: u64 = 10_000;
/// The pack's zip may be no bigger than this.
const SIZE_BUDGET: u64 = 150_000_000;
/// The pack's name, and its folder in the zip.
pub const NAME: &str = "olio-instruments";

/// A recording, decoded and placed.
pub struct Take {
    stem: String,
    root: u8,
    layer: usize,
    /// The notes it plays, when set by the recipe.
    range: Option<(u8, u8)>,
    pitched: bool,
    rate: u32,
    /// Interleaved stereo.
    samples: Vec<f32>,
    looped: Option<(u64, u64)>,
    /// Measured against the root, in cents.
    tune: f32,
}

/// The layer of a file: the index of the first of `layers` that is one of
/// its `_`-separated parts.
fn layer(stem: &str, layers: &[&str]) -> Option<usize> {
    if layers.is_empty() {
        return Some(0);
    }
    let parts: Vec<&str> = stem.split('_').collect();
    layers.iter().position(|layer| parts.contains(layer))
}

fn takes_file(source: &Source, name: &str) -> bool {
    (source.any.is_empty() || source.any.iter().any(|part| name.contains(part)))
        && !source.exclude.iter().any(|part| name.contains(part))
}

/// The recordings a recipe takes from the list `sources`, with their notes
/// and layers: all of those that match.
pub fn gather(
    recipe: &Recipe,
    sources: &Sources,
    dir: &Path,
) -> Result<Vec<(PathBuf, Take)>, String> {
    gather_thinned(recipe, sources, dir, false)
}

/// As [`gather`], from a list of every recording a library has, `thin`ning
/// each source to the roots it asks for (see [`Source::every`]): what
/// `pin` writes to the list `build` then takes whole.
pub fn gather_thinned(
    recipe: &Recipe,
    sources: &Sources,
    dir: &Path,
    thin_sources: bool,
) -> Result<Vec<(PathBuf, Take)>, String> {
    let mut takes = Vec::new();
    for source in recipe.sources {
        let first = takes.len();
        for file in &sources.files {
            let Some((folder, name)) = file.path.rsplit_once('/') else {
                continue;
            };
            if file.repo != source.repo || folder != source.folder || !takes_file(source, name) {
                continue;
            }
            let stem = name.trim_end_matches(".wav");
            let Some(layer) = layer(stem, source.layers) else {
                continue;
            };
            let (root, range) = match source.root {
                Some((root, low, high)) => (root, Some((low, high))),
                None => match pitch::note_in(stem, source.octave) {
                    Some(root) => (root, None),
                    None => continue,
                },
            };
            if root < source.roots.0 || root > source.roots.1 {
                continue;
            }
            takes.push((
                fetch::local(dir, file),
                Take {
                    stem: stem.to_owned(),
                    root,
                    layer,
                    range,
                    pitched: source.pitched,
                    rate: 0,
                    samples: Vec::new(),
                    looped: None,
                    tune: 0.0,
                },
            ));
        }
        if thin_sources {
            thin(&mut takes, first, source.every);
        }
    }
    if takes.is_empty() {
        return Err(format!("no recordings for {:?}", recipe.instrument));
    }
    Ok(takes)
}

/// Keeps every `every`th root of each layer among `takes[first..]` (the
/// first and last always), with all the takes of a root kept or dropped
/// together.
fn thin(takes: &mut Vec<(PathBuf, Take)>, first: usize, every: usize) {
    if every <= 1 {
        return;
    }
    let mut keep = Vec::new();
    for layer in 0..=takes[first..]
        .iter()
        .map(|(_, take)| take.layer)
        .max()
        .unwrap_or(0)
    {
        let mut roots: Vec<u8> = takes[first..]
            .iter()
            .filter(|(_, take)| take.layer == layer && take.range.is_none())
            .map(|(_, take)| take.root)
            .collect();
        roots.sort_unstable();
        roots.dedup();
        let last = roots.len().saturating_sub(1);
        for (index, root) in roots.iter().enumerate() {
            if index % every == 0 || index == last {
                keep.push((layer, *root));
            }
        }
    }
    let mut index = 0;
    takes.retain(|(_, take)| {
        index += 1;
        index <= first || take.range.is_some() || keep.contains(&(take.layer, take.root))
    });
}

/// Decodes a recording into interleaved stereo.
fn decode(path: &Path) -> Result<(u32, Vec<f32>), String> {
    let (rate, channels, samples) = match decode_file(path) {
        Ok(clip) => (
            clip.rate(),
            usize::from(clip.channels().max(1)),
            clip.samples().to_vec(),
        ),
        // Some recordings have a format chunk Symphonia refuses.
        Err(err) => read_pcm_wav(path).ok_or_else(|| format!("{}: {err}", path.display()))?,
    };
    let samples = samples
        .chunks_exact(channels)
        .flat_map(|frame| [frame[0], frame[if channels > 1 { 1 } else { 0 }]])
        .collect();
    Ok((rate, samples))
}

/// A plain PCM WAV (16- or 24-bit), whatever the size of its format
/// chunk: rate, channels and interleaved samples.
fn read_pcm_wav(path: &Path) -> Option<(u32, usize, Vec<f32>)> {
    let data = fs::read(path).ok()?;
    if data.get(0..4)? != b"RIFF" || data.get(8..12)? != b"WAVE" {
        return None;
    }
    let u16_at = |at: usize| Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?));
    let mut at = 12;
    let mut format = None;
    while at + 8 <= data.len() {
        let id = &data[at..at + 4];
        let size = u32_at(at + 4)? as usize;
        let body = at + 8;
        if id == b"fmt " {
            let (tag, channels, rate, bits) = (
                u16_at(body)?,
                u16_at(body + 2)?,
                u32_at(body + 4)?,
                u16_at(body + 14)?,
            );
            (tag == 1 && (bits == 16 || bits == 24)).then_some(())?;
            format = Some((usize::from(channels.max(1)), rate, usize::from(bits / 8)));
        } else if id == b"data" {
            let (channels, rate, width) = format?;
            let end = (body + size).min(data.len());
            let samples = data[body..end]
                .chunks_exact(width)
                .map(|bytes| match width {
                    2 => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32768.0,
                    _ => {
                        (i32::from_le_bytes([0, bytes[0], bytes[1], bytes[2]]) >> 8) as f32
                            / 8_388_608.0
                    }
                })
                .collect();
            return Some((rate, channels, samples));
        }
        at = body + size + (size & 1);
    }
    None
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()))
}

/// The pitch measured a little after the attack, in cents from the root;
/// `None` if there is no clear pitch.
fn measure(take: &Take) -> Option<f32> {
    let frames = take.samples.len() / 2;
    let from = ((0.15 * take.rate as f32) as usize).min(frames / 3);
    let length = 16_384.min(frames.saturating_sub(from));
    let (window, _) = take.samples[from * 2..(from + length) * 2].as_chunks::<2>();
    let mono: Vec<f32> = window
        .iter()
        .map(|[left, right]| 0.5 * (left + right))
        .collect();
    let measured = pitch::detect(&mono, take.rate)?;
    Some(pitch::cents(measured, pitch::hz(take.root)))
}

/// Starts the recording at its first sound, a couple of milliseconds
/// before.
fn trim_start(take: &mut Take) {
    let threshold = peak(&take.samples) * 0.01;
    let first = take
        .samples
        .iter()
        .position(|s| s.abs() > threshold)
        .map_or(0, |at| at / 2);
    let start = first.saturating_sub((0.002 * take.rate as f32) as usize);
    take.samples.drain(..start * 2);
}

/// Keeps at most `seconds`, the last `fade` of them fading out.
fn trim_end(take: &mut Take, seconds: f32, fade: f32) {
    let frames = (take.samples.len() / 2).min((seconds * take.rate as f32) as usize);
    take.samples.truncate(frames * 2);
    let fade = ((fade * take.rate as f32) as usize).min(frames);
    for i in 0..fade {
        let t = i as f32 / fade as f32;
        let gain = 0.5 * (1.0 + (std::f32::consts::PI * t).cos());
        let at = frames - fade + i;
        take.samples[at * 2] *= gain;
        take.samples[at * 2 + 1] *= gain;
    }
}

/// Where in a held note's steady part the loop lies: from this share of it
/// to near its end.
const LOOP_FROM: f32 = 0.3;
/// The loop's end is looked for between these shares of the steady part.
const LOOP_END: (f32, f32) = (0.75, 1.0);

/// The steady part of a held note, in frames: from where its loudness
/// (RMS over 50 ms, smoothed) first reaches 70 % of its greatest to where
/// it last holds 50 % of it. After that the bow lifts, the breath stops or
/// the string is let go and the sound rings out, which a loop must not
/// take in, or each pass would die away and swell again.
fn plateau(mono: &[f32], rate: u32, longest: f32) -> (usize, usize) {
    let block = (rate as usize / 20).max(1);
    let levels: Vec<f32> = mono
        .chunks(block)
        .map(|chunk| (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt())
        .collect();
    let smooth: Vec<f32> = (0..levels.len())
        .map(|i| {
            let (from, to) = (i.saturating_sub(1), (i + 2).min(levels.len()));
            levels[from..to].iter().sum::<f32>() / (to - from) as f32
        })
        .collect();
    let loudest = smooth.iter().copied().fold(0.0f32, f32::max);
    let first = smooth.iter().position(|level| *level >= 0.7 * loudest);
    let last = smooth.iter().rposition(|level| *level >= 0.5 * loudest);
    let (from, to) = match (first, last) {
        (Some(first), Some(last)) if last > first => {
            (first * block, ((last + 1) * block).min(mono.len()))
        }
        _ => (mono.len() / 4, mono.len() * 9 / 10),
    };
    // A note that swells all the way, or has little steady sound, is given
    // at least this much to loop in, reaching back from where it ends
    // (but not into its attack), and no more than `longest` seconds
    // (0: as long as it is steady).
    let wanted = (MIN_STEADY * rate as f32) as usize;
    let from = if to - from < wanted {
        to.saturating_sub(wanted)
            .max(from.min((0.3 * rate as f32) as usize))
    } else {
        from
    };
    // Of a longer steady part, the flattest stretch of the length wanted:
    // a note that swells and fades while it is held loops best where it
    // swells least.
    let wanted = ((if longest > 0.0 { longest } else { STEADY }) * rate as f32) as usize;
    if to - from <= wanted {
        return (from, to);
    }
    let blocks = (wanted / block).max(1);
    let (first, last) = (from / block, (to / block).saturating_sub(blocks));
    let swing = |at: usize| {
        let part = &smooth[at..(at + blocks).min(smooth.len())];
        let high = part.iter().copied().fold(0.0f32, f32::max);
        let low = part.iter().copied().fold(f32::MAX, f32::min);
        high / low.max(1e-9)
    };
    let best = (first..=last.max(first))
        .min_by(|a, b| swing(*a).total_cmp(&swing(*b)))
        .unwrap_or(first);
    (best * block, best * block + wanted)
}

/// The longest stretch of a held note looped when a recipe gives none.
const STEADY: f32 = 6.0;

/// The least a loop has to loop in, in seconds.
const MIN_STEADY: f32 = 1.5;

/// The normalised correlation of `window` frames of `mono` ending at
/// `a` and ending at `b`: 1 where they match exactly, 0 where unrelated.
fn similarity(mono: &[f32], a: usize, b: usize, window: usize) -> f32 {
    let (x, y) = (&mono[a - window..a], &mono[b - window..b]);
    let (mut xy, mut xx, mut yy) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in x.iter().zip(y) {
        let (x, y) = (f64::from(*x), f64::from(*y));
        xy += x * y;
        xx += x * x;
        yy += y * y;
    }
    (xy / (xx * yy).sqrt().max(1e-12)) as f32
}

/// How alike the sound before an end and the sound before a start are,
/// along a blend: every `STEP` seconds, the normalised correlation
/// there, smoothed over neighbours, between -0.5 and 1.
struct Similarity {
    values: Vec<f32>,
    step: usize,
}

impl Similarity {
    /// At frame `i` of the blend, between the readings either side.
    fn at(&self, i: usize) -> f32 {
        if self.values.is_empty() {
            return 0.0;
        }
        let place = (i as f32 / self.step as f32 - 0.5).max(0.0);
        let (low, fraction) = (place.floor() as usize, place.fract());
        let high = (low + 1).min(self.values.len() - 1);
        let low = low.min(self.values.len() - 1);
        self.values[low] * (1.0 - fraction) + self.values[high] * fraction
    }
}

const SIMILARITY_STEP: f32 = 0.04;

fn blend_similarity(mono: &[f32], end: usize, start: usize, blend: usize, rate: u32) -> Similarity {
    let step = ((SIMILARITY_STEP * rate as f32) as usize).max(16);
    let readings: Vec<f32> = (0..blend.div_ceil(step))
        .map(|k| {
            let from = k * step;
            let to = (from + step).min(blend);
            similarity(mono, end - blend + to, start - blend + to, to - from).clamp(-0.5, 1.0)
        })
        .collect();
    let values = (0..readings.len())
        .map(|k| {
            let before = readings[k.saturating_sub(1)];
            let after = readings[(k + 1).min(readings.len() - 1)];
            (before + 2.0 * readings[k] + after) / 4.0
        })
        .collect();
    Similarity { values, step }
}

/// Makes a held note loop within its steady part ([`plateau`]), from
/// [`LOOP_FROM`] of the way through it to an end chosen near its end,
/// where the sound before it most resembles the sound before the start (the same phase of any vibrato or beating, so the join continues
/// it), the end blended into what comes before the loop's start (at
/// constant power, over `blend_seconds`), so jumping from the end back to
/// the start continues the sound; cut at the loop's end.
fn bake_loop(take: &mut Take, blend_seconds: f32, longest: f32) {
    let frames = take.samples.len() / 2;
    let mono: Vec<f32> = take
        .samples
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| 0.5 * (frame[0] + frame[1]))
        .collect();
    let (steady_from, steady_to) = plateau(&mono, take.rate, longest);
    let span = steady_to - steady_from;
    let start = steady_from + (span as f32 * LOOP_FROM) as usize;
    // The blend reaches back from the start, and no further than the
    // steady part, nor further than half the loop.
    let blend = ((blend_seconds * take.rate as f32) as usize)
        .min(start - steady_from)
        .min((steady_to - start) / 2)
        .min(frames / 5);
    let (low, high) = (
        (steady_from + (span as f32 * LOOP_END.0) as usize).max(start + 2 * blend.max(1)),
        (steady_from + (span as f32 * LOOP_END.1) as usize).min(frames),
    );
    let mut end = high.max(low).min(frames);
    if blend > 0 && low < high {
        // Coarsely, on block averages, over the blend; then at full rate
        // over a few cycles of a low note.
        const BLOCK: usize = 16;
        let coarse: Vec<f32> = mono
            .as_chunks::<BLOCK>()
            .0
            .iter()
            .map(|block| block.iter().sum::<f32>() / BLOCK as f32)
            .collect();
        let window = (blend / BLOCK).max(8);
        let reference = start / BLOCK;
        let mut best = (f32::MIN, end);
        let mut candidate = low / BLOCK;
        while candidate <= high / BLOCK && candidate <= coarse.len() {
            if candidate >= window && reference >= window {
                let score = similarity(&coarse, candidate, reference, window);
                if score > best.0 {
                    best = (score, candidate * BLOCK);
                }
            }
            candidate += 1;
        }
        end = best.1;
        let fine = 4096.min(blend).min(start);
        let mut best = (f32::MIN, end);
        for candidate in end.saturating_sub(BLOCK)..=(end + BLOCK).min(frames) {
            if candidate >= fine {
                let score = similarity(&mono, candidate, start, fine);
                if score > best.0 {
                    best = (score, candidate);
                }
            }
        }
        end = best.1;
    }
    // The two halves are blended to keep the power level: as the ending
    // fades out by `a` and the beginning in by `t`, a² + t² + 2ρat = 1,
    // where ρ is how much alike they are right there (1: a straight
    // cross-fade; 0: the usual equal-power one; a little below 0, a
    // little more of both, for halves that cancel). Two tones of nearly
    // one pitch drift in and out of step over a blend, so ρ is read every
    // 40 ms along it, smoothed, and followed.
    let rhos = blend_similarity(&mono, end, start, blend, take.rate);
    for i in 0..blend {
        let t = i as f32 / blend as f32;
        let rho = rhos.at(i);
        let (fade_out, fade_in) = (-rho * t + (rho * rho * t * t - t * t + 1.0).sqrt(), t);
        let at = end - blend + i;
        let from = start - blend + i;
        for channel in 0..2 {
            let ending = take.samples[at * 2 + channel];
            let beginning = take.samples[from * 2 + channel];
            take.samples[at * 2 + channel] = ending * fade_out + beginning * fade_in;
        }
    }
    take.samples.truncate(end * 2);
    take.looped = Some((start as u64, end as u64));
}

/// True if both channels of interleaved stereo are the same throughout.
fn is_mono(samples: &[f32]) -> bool {
    samples
        .as_chunks::<2>()
        .0
        .iter()
        .all(|frame| frame[0] == frame[1])
}

/// 16-bit FLAC of interleaved stereo, as one channel where both are the
/// same (half the size; Olio reads it back to stereo).
fn flac(samples: &[f32], rate: u32) -> Result<Vec<u8>, String> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let mono = is_mono(samples);
    let ints: Vec<i32> = samples
        .iter()
        .step_by(if mono { 2 } else { 1 })
        .map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i32)
        .collect();
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|err| format!("{err:?}"))?;
    let source = flacenc::source::MemSource::from_samples(
        &ints,
        if mono { 1 } else { 2 },
        16,
        rate as usize,
    );
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|err| format!("{err:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|err| format!("{err:?}"))?;
    let mut bytes = sink.as_slice().to_vec();
    // The encoder gives the last block's size as the stream's smallest,
    // which the format excludes; a smallest under the largest reads as a
    // stream of varying blocks, and Symphonia then misreads every frame.
    // The blocks are all one size but the last: say so.
    if bytes.len() > 12 && &bytes[..4] == b"fLaC" {
        let largest = [bytes[10], bytes[11]];
        bytes[8..10].copy_from_slice(&largest);
    }
    Ok(bytes)
}

/// The furthest, in semitones, a layer's lowest or highest recording is
/// stretched beyond its root. A note beyond that, in a layer with nothing
/// nearer, is played from the nearest recording of any layer instead.
const STRETCH: u8 = 7;

/// How much of the difference in loudness between the notes of a layer is
/// evened out, so a melody does not jump in level from one note to the
/// next (0 leaves the recordings as they are, 1 makes them all alike), and
/// the furthest, as a factor, a note is raised or lowered for it.
const NOTE_LEVELLING: f32 = 0.7;
const NOTE_LIMIT: f32 = 8.0;
/// How much of the difference in loudness between a voice's layers is
/// evened out, and the most a quieter layer is raised. A soft recording is
/// much quieter than a loud one; playing it at a lower velocity as well
/// would double that, and make the level step at the line between them.
const LAYER_LEVELLING: f32 = 0.75;
const LAYER_BOOST: f32 = 6.0;

/// A take's loudness: for a held note its RMS inside its loop (the sound
/// it makes while it is played), for a struck or plucked one its loudest
/// 50 ms of the first 1.5 s (a tail, the rest of it, is not what is heard).
fn loudness(take: &Take) -> f32 {
    let rate = take.rate as usize;
    let frames = take.samples.len() / 2;
    let rms = |from: usize, to: usize| {
        let part = &take.samples[from.min(frames) * 2..to.min(frames) * 2];
        if part.is_empty() {
            0.0
        } else {
            (part.iter().map(|s| s * s).sum::<f32>() / part.len() as f32).sqrt()
        }
    };
    if let Some((start, end)) = take.looped {
        return rms(start as usize, end as usize);
    }
    let window = (rate / 20).max(1);
    (0..rate * 3 / 2)
        .step_by(window / 2 + 1)
        .map(|from| rms(from, from + window))
        .fold(0.0, f32::max)
}

/// The takes of one source-recorded layer (not those a recipe gives a
/// range, like drums).
fn in_layer(take: &Take, layer: usize) -> bool {
    take.layer == layer && take.range.is_none()
}

/// Brings the notes of each layer towards that layer's median loudness.
fn level_notes(takes: &mut [Take]) {
    let layers = takes
        .iter()
        .map(|take| take.layer)
        .max()
        .map_or(0, |l| l + 1);
    for layer in 0..layers {
        let mut loud: Vec<f32> = takes
            .iter()
            .filter(|take| in_layer(take, layer))
            .map(loudness)
            .filter(|level| *level > 0.0)
            .collect();
        loud.sort_by(f32::total_cmp);
        let Some(&median) = loud.get(loud.len() / 2) else {
            continue;
        };
        for take in takes.iter_mut().filter(|take| in_layer(take, layer)) {
            let level = loudness(take);
            if level > 0.0 {
                let factor = (median / level)
                    .powf(NOTE_LEVELLING)
                    .clamp(1.0 / NOTE_LIMIT, NOTE_LIMIT);
                for sample in &mut take.samples {
                    *sample *= factor;
                }
            }
        }
    }
}

/// How each take is scaled for the pack and the gain it plays at: layers
/// are each brought up to the full range of the file (so the quietest has
/// all the bits there are), and the gain brings them back down, but
/// not all the way to where they began: the quieter ones stay raised by a
/// share of the difference ([`LAYER_LEVELLING`]).
fn scales(takes: &[Take]) -> Vec<(f32, f32)> {
    let voice_peak = takes
        .iter()
        .map(|take| peak(&take.samples))
        .fold(0.0, f32::max);
    let voice = if voice_peak > 0.0 {
        PEAK / voice_peak
    } else {
        1.0
    };
    let layers = takes
        .iter()
        .map(|take| take.layer)
        .max()
        .map_or(0, |l| l + 1);
    let mean_power = |layer: usize| {
        let levels: Vec<f32> = takes
            .iter()
            .filter(|take| in_layer(take, layer))
            .map(loudness)
            .collect();
        (levels.iter().map(|l| l * l).sum::<f32>() / levels.len().max(1) as f32).sqrt()
    };
    let loudest = (0..layers).map(mean_power).fold(0.0, f32::max);
    takes
        .iter()
        .map(|take| {
            if take.range.is_some() {
                return (voice, 1.0);
            }
            let layer_peak = takes
                .iter()
                .filter(|other| in_layer(other, take.layer))
                .map(|other| peak(&other.samples))
                .fold(0.0, f32::max);
            let scale = if layer_peak > 0.0 {
                PEAK / layer_peak
            } else {
                voice
            };
            let level = mean_power(take.layer);
            let boost = if level > 0.0 {
                (loudest / level)
                    .powf(LAYER_LEVELLING)
                    .clamp(1.0, LAYER_BOOST)
            } else {
                1.0
            };
            (scale, voice / scale * boost)
        })
        .collect()
}

/// The notes each take plays: within a layer, halfway to the next root
/// either side, the first down to `STRETCH` below its root and the last up
/// to `STRETCH` above.
fn ranges(takes: &[Take]) -> Vec<(u8, u8)> {
    takes
        .iter()
        .map(|take| {
            if let Some(range) = take.range {
                return range;
            }
            let mut roots: Vec<u8> = takes
                .iter()
                .filter(|other| other.layer == take.layer && other.range.is_none())
                .map(|other| other.root)
                .collect();
            roots.sort_unstable();
            roots.dedup();
            let at = roots.binary_search(&take.root).unwrap_or(0);
            let low = if at == 0 {
                take.root.saturating_sub(STRETCH)
            } else {
                (roots[at - 1] + take.root) / 2 + 1
            };
            let high = roots
                .get(at + 1)
                .map_or(take.root.saturating_add(STRETCH).min(127), |next| {
                    (take.root + next) / 2
                });
            (low, high)
        })
        .collect()
}

/// A file name the pack can use anywhere.
fn file_name(stem: &str) -> String {
    stem.chars()
        .map(|c| match c {
            '#' => 's',
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' => c,
            _ => '_',
        })
        .collect()
}

/// Reports each recording's measured pitch against its name.
pub fn check(dir: &Path) -> Result<(), String> {
    let sources = fetch::sources();
    for recipe in RECIPES {
        let mut off = 0;
        let gathered = gather(recipe, &sources, dir)?;
        let count = gathered.len();
        for (path, mut take) in gathered {
            if !take.pitched || take.range.is_some() {
                continue;
            }
            (take.rate, take.samples) = decode(&path)?;
            trim_start(&mut take);
            match measure(&take) {
                Some(cents) if cents.abs() <= 60.0 => {}
                Some(cents) => {
                    off += 1;
                    println!(
                        "  {} {}: {:+.0} cents ({:+.1} semitones)",
                        recipe.name,
                        take.stem,
                        cents,
                        cents / 100.0
                    );
                }
                None => println!("  {} {}: no clear pitch", recipe.name, take.stem),
            }
        }
        println!(
            "{:?} {}: {count} recordings, {off} off pitch",
            recipe.instrument, recipe.name
        );
    }
    Ok(())
}

/// The licence and credit of the libraries a recipe's recordings come
/// from, each once.
fn provenance(recipe: &Recipe, sources: &Sources) -> Result<(String, String), String> {
    let mut licenses: Vec<&str> = Vec::new();
    let mut credits: Vec<&str> = Vec::new();
    for source in recipe.sources {
        let library = sources
            .libraries
            .iter()
            .find(|library| library.repo == source.repo)
            .ok_or_else(|| format!("no library {}", source.repo))?;
        if !licenses.contains(&library.license.as_str()) {
            licenses.push(&library.license);
        }
        if !credits.contains(&library.credit.as_str()) {
            credits.push(&library.credit);
        }
    }
    Ok((licenses.join("; "), credits.join("; ")))
}

/// Builds the pack from the recordings in `dir` into `out/olio-instruments`
/// and zips it to `out/olio-instruments-<version>.zip`.
pub fn build(dir: &Path, out: &Path) -> Result<(), String> {
    let sources = fetch::sources();
    let pack = out.join(NAME);
    if pack.exists() {
        fs::remove_dir_all(&pack).map_err(|err| format!("{}: {err}", pack.display()))?;
    }
    let mut instruments = Vec::new();
    let mut seconds_total = 0.0;
    for recipe in RECIPES {
        let mut takes = Vec::new();
        for (path, mut take) in gather(recipe, &sources, dir)? {
            (take.rate, take.samples) = decode(&path)?;
            trim_start(&mut take);
            // Tuned to the root where the measure is clear and close; a
            // reading far off is the detector, not the recording.
            if take.pitched && take.range.is_none() {
                take.tune = measure(&take)
                    .filter(|cents| cents.abs() <= 60.0)
                    .unwrap_or(0.0);
            }
            if recipe.looped {
                bake_loop(&mut take, recipe.loop_blend, recipe.seconds);
            } else {
                trim_end(&mut take, recipe.seconds, recipe.fade);
            }
            takes.push(take);
        }
        level_notes(&mut takes);
        let scales = scales(&takes);
        let folder = pack.join(recipe.dir);
        fs::create_dir_all(&folder).map_err(|err| format!("{}: {err}", folder.display()))?;
        let layers = recipe
            .sources
            .iter()
            .map(|source| source.layers.len().max(1))
            .max()
            .unwrap_or(1);
        let step = 1.0 / layers as f32;
        // Neighbouring layers blend no further than halfway into each other.
        let blend = recipe.blend.min(step / 2.0);
        let ranges = ranges(&takes);
        let mut zones = Vec::new();
        for ((take, (low, high)), (scale, gain)) in takes.iter_mut().zip(ranges).zip(scales) {
            for sample in &mut take.samples {
                *sample *= scale;
            }
            let file = format!("{}/{}.flac", recipe.dir, file_name(&take.stem));
            let bytes = flac(&take.samples, take.rate)?;
            fs::write(pack.join(&file), bytes).map_err(|err| format!("{file}: {err}"))?;
            // Read back as Olio will read it.
            let decoded = decode_file(&pack.join(&file)).map_err(|err| format!("{file}: {err}"))?;
            if decoded.frames() != take.samples.len() / 2 {
                return Err(format!("{file} reads back {} frames", decoded.frames()));
            }
            seconds_total += take.samples.len() as f32 / 2.0 / take.rate as f32;
            zones.push(ZoneManifest {
                file,
                root: take.root,
                low,
                high,
                velocity: (take.layer as f32 * step, (take.layer + 1) as f32 * step),
                looped: take.looped,
                gain: (gain * 1000.0).round() / 1000.0,
                pitched: take.pitched,
                tune: (take.tune * 10.0).round() / 10.0,
            });
        }
        println!(
            "{:?} {}: {} recordings",
            recipe.instrument,
            recipe.name,
            zones.len()
        );
        let (license, credit) = provenance(recipe, &sources)?;
        let mut entry = InstrumentManifest::new(
            recipe.instrument,
            recipe.attack,
            recipe.release,
            recipe.gain,
            zones,
        );
        entry.voice = recipe.voice;
        entry.name = recipe.name.to_owned();
        entry.license = license;
        entry.credit = credit;
        entry.blend = blend;
        entry.range = recipe.range;
        instruments.push(entry);
    }
    let manifest = Manifest {
        version: PACK_VERSION,
        instruments,
    };
    check_manifest(&manifest)?;
    let text = serde_json::to_string_pretty(&manifest).map_err(|err| err.to_string())?;
    fs::write(pack.join(MANIFEST), text).map_err(|err| format!("{MANIFEST}: {err}"))?;
    fs::write(pack.join("CREDITS.md"), credits(&sources, &manifest))
        .map_err(|err| err.to_string())?;

    let zip_path = out.join(format!("{NAME}-{PACK_VERSION}.zip"));
    let size = zip_folder(&pack, &zip_path)?;
    let hash = sha256(&zip_path)?;
    fs::write(zip_path.with_extension("zip.sha256"), format!("{hash}\n"))
        .map_err(|err| err.to_string())?;
    println!(
        "{} minutes of recordings; {} ({:.0} MB), SHA-256 {hash}",
        (seconds_total / 60.0).round(),
        zip_path.display(),
        size as f64 / 1e6
    );
    if size > SIZE_BUDGET {
        return Err(format!(
            "the pack is {:.0} MB, over its budget of {:.0} MB",
            size as f64 / 1e6,
            SIZE_BUDGET as f64 / 1e6
        ));
    }
    Ok(())
}

/// What a pack Olio publishes must have: every voice named once, each
/// with a licence and a credit, and voices numbered without gaps.
fn check_manifest(manifest: &Manifest) -> Result<(), String> {
    for entry in &manifest.instruments {
        let who = format!("{:?} voice {}", entry.instrument, entry.voice);
        if entry.name.is_empty() || entry.license.is_empty() || entry.credit.is_empty() {
            return Err(format!("{who} lacks a name, licence or credit"));
        }
        if entry.zones.is_empty() {
            return Err(format!("{who} has no recordings"));
        }
        // A loop under about a quarter second would buzz.
        if let Some(zone) = entry.zones.iter().find(|zone| {
            zone.looped
                .is_some_and(|(from, to)| to - from < MIN_LOOP_FRAMES)
        }) {
            return Err(format!("{who}: {} loops under a quarter second", zone.file));
        }
        // Its range is where its recordings are: one that is an octave out
        // (names read with the wrong middle C) strands the recordings.
        if let Some((low, high)) = entry.range {
            let (low, high) = (low.saturating_sub(3), high.saturating_add(3));
            if let Some(zone) = entry
                .zones
                .iter()
                .find(|zone| !(low..=high).contains(&zone.root))
            {
                return Err(format!(
                    "{who}: {} (root {}) lies outside its range",
                    zone.file, zone.root
                ));
            }
        }
        let same = manifest
            .instruments
            .iter()
            .filter(|other| other.instrument == entry.instrument);
        if same
            .clone()
            .filter(|other| other.voice == entry.voice)
            .count()
            != 1
        {
            return Err(format!("{who} is there twice"));
        }
        if entry.voice != 0 && !same.clone().any(|other| other.voice == 0) {
            return Err(format!("{who} has no voice 0 to fall back to"));
        }
    }
    Ok(())
}

/// The pack's CREDITS.md: who recorded what, under which licence, and
/// what Olio did to it; a line for every voice, from the manifest.
fn credits(sources: &Sources, manifest: &Manifest) -> String {
    let mut text = String::from(
        "# Olio instrument pack\n\n\
         Recordings from the Versilian Studios libraries, released by Sam \
         Gossner and Versilian Studios into the public domain (CC0 1.0). \
         Olio's pack trims them, loops the held notes, levels each \
         voice and re-encodes them as FLAC; it is CC0 as well.\n\n",
    );
    for library in &sources.libraries {
        text.push_str(&format!(
            "- {} ({}), {} at commit {}\n",
            library.name, library.license, library.url, library.commit
        ));
    }
    text.push_str("\nVoices:\n\n");
    for entry in &manifest.instruments {
        let recipe = RECIPES
            .iter()
            .find(|recipe| recipe.instrument == entry.instrument && recipe.voice == entry.voice);
        let folders: Vec<String> = recipe
            .map(|recipe| {
                recipe
                    .sources
                    .iter()
                    .map(|source| format!("{} {}", source.repo, source.folder))
                    .collect()
            })
            .unwrap_or_default();
        text.push_str(&format!(
            "- {:?}, {}: {} ({}; credit {})\n",
            entry.instrument,
            entry.name,
            folders.join("; "),
            entry.license,
            entry.credit
        ));
    }
    text
}

/// Stores `dir` (already compressed FLAC) in a zip under its own name.
fn zip_folder(dir: &Path, zip_path: &Path) -> Result<u64, String> {
    let file = fs::File::create(zip_path).map_err(|err| err.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    let mut paths = Vec::new();
    collect(dir, &mut paths)?;
    paths.sort();
    for path in paths {
        let relative = path.strip_prefix(dir).map_err(|err| err.to_string())?;
        let name = format!("{NAME}/{}", relative.to_string_lossy().replace('\\', "/"));
        zip.start_file(name, options)
            .map_err(|err| err.to_string())?;
        zip.write_all(&fs::read(&path).map_err(|err| err.to_string())?)
            .map_err(|err| err.to_string())?;
    }
    zip.finish().map_err(|err| err.to_string())?;
    Ok(fs::metadata(zip_path).map_err(|err| err.to_string())?.len())
}

fn collect(dir: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|err| err.to_string())? {
        let path = entry.map_err(|err| err.to_string())?.path();
        if path.is_dir() {
            collect(&path, paths)?;
        } else {
            paths.push(path);
        }
    }
    Ok(())
}

fn sha256(path: &Path) -> Result<String, String> {
    let data = fs::read(path).map_err(|err| err.to_string())?;
    Ok(Sha256::digest(&data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use olio_audio::ambience::Instrument;

    fn take(root: u8, layer: usize) -> Take {
        Take {
            stem: String::new(),
            root,
            layer,
            range: None,
            pitched: true,
            rate: 44_100,
            samples: Vec::new(),
            looped: None,
            tune: 0.0,
        }
    }

    #[test]
    fn each_recording_plays_the_notes_nearest_it_within_its_layer() {
        let takes = [take(48, 0), take(52, 0), take(60, 0), take(50, 1)];
        // The first recording of a layer plays down a fifth below its root
        // at most, the last up a fifth above; the layers cover alike.
        assert_eq!(ranges(&takes), [(41, 50), (51, 56), (57, 67), (43, 57)],);
        assert_eq!(layer("GPiano_sus_C4_v2_rr1", &["v1", "v2"]), Some(1));
        assert_eq!(layer("GPiano_sus_C4_v3_rr1", &["v1", "v2"]), None);
        assert_eq!(layer("sus_C4_r01_main", &[]), Some(0));
        assert_eq!(file_name("GPiano_sus_A#3_v1"), "GPiano_sus_As3_v1");
    }

    #[test]
    fn a_baked_loop_joins_without_a_jump() {
        let rate = 44_100;
        // A slowly varying tone: a jump at the join would show.
        let samples: Vec<f32> = (0..rate * 4)
            .flat_map(|i| {
                let t = i as f32 / rate as f32;
                let s = (t * 330.0 * std::f32::consts::TAU).sin() * (0.5 + 0.2 * (t * 1.3).sin());
                [s, s]
            })
            .collect();
        let mut held = Take {
            samples,
            ..take(64, 0)
        };
        bake_loop(&mut held, 0.5, 0.0);
        let (start, end) = held.looped.unwrap();
        assert_eq!(held.samples.len() as u64, end * 2);
        // Going round the loop, the step from the last frame to the
        // start is no bigger than the steps either side.
        let at = |frame: u64| held.samples[frame as usize * 2];
        let jump = (at(start) - at(end - 1)).abs();
        let usual = (at(end - 1) - at(end - 2))
            .abs()
            .max((at(start + 1) - at(start)).abs());
        assert!(jump <= usual * 1.5 + 1e-3, "{jump} vs {usual}");
    }

    /// A held note: a tone with vibrato and a slow swell, `seconds` long.
    fn vibrato_note(seconds: usize) -> Take {
        let rate = 44_100;
        let samples: Vec<f32> = (0..rate * seconds)
            .flat_map(|i| {
                let t = i as f32 / rate as f32;
                // A vibrato of 5.3 Hz.
                let phase = t * 330.0 + 0.6 * (t * 5.3 * std::f32::consts::TAU).sin();
                let swell = 0.6 + 0.3 * (t * 0.7 * std::f32::consts::TAU).sin();
                let s = (phase * std::f32::consts::TAU).sin() * swell;
                [s, s]
            })
            .collect();
        Take {
            samples,
            ..take(64, 0)
        }
    }

    #[test]
    fn a_loop_ends_where_the_sound_matches_the_start_better_than_at_a_fixed_place() {
        let held = vibrato_note(8);
        let mono: Vec<f32> = held
            .samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|f| f[0])
            .collect();
        let (from, to) = plateau(&mono, 44_100, 0.0);
        let span = to - from;
        let blend = 44_100;
        let start = from + (span as f32 * LOOP_FROM) as usize;
        // What a fixed end in the middle of the window would have joined,
        // against what was chosen.
        let fixed = similarity(&mono, from + span * 7 / 8, start, blend);
        let mut baked = held;
        bake_loop(&mut baked, 1.0, 0.0);
        let (loop_start, end) = baked.looped.unwrap();
        assert_eq!(loop_start as usize, start);
        let end = end as usize;
        assert!(
            (from + span * 3 / 4..=to).contains(&end),
            "{end} of {to}, from {from}"
        );
        let chosen = similarity(&mono, end, start, blend);
        assert!(chosen > 0.9, "the join matches: {chosen}");
        assert!(
            chosen >= fixed - 1e-3,
            "{chosen} against {fixed} at a fixed end"
        );
        // Going round the loop is a continuation: the last 50 ms are the
        // sound that came just before the start (the blend is complete),
        // and the step across the join is as small as the steps either
        // side of it.
        let rms = |samples: &[f32]| {
            (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
        };
        let original = vibrato_note(8);
        let before = rms(&baked.samples[(end - 2205) * 2..end * 2]);
        let pre_start = rms(&original.samples[(start - 2205) * 2..start * 2]);
        assert!(
            (before / pre_start - 1.0).abs() < 0.03,
            "{before} {pre_start}"
        );
        let at = |frame: usize| baked.samples[frame * 2];
        let jump = (at(start) - at(end - 1)).abs();
        let usual = (at(end - 1) - at(end - 2))
            .abs()
            .max((at(start + 1) - at(start)).abs());
        assert!(jump <= usual * 1.5 + 1e-3, "{jump} vs {usual}");
    }

    /// A bowed note: an attack of 0.3 s, a steady tone until `bowed`
    /// seconds, then it rings out, falling 80 dB in `ring` seconds.
    fn bowed_note(bowed: f32, ring: f32) -> Take {
        let rate = 44_100usize;
        let seconds = bowed + ring;
        let samples: Vec<f32> = (0..(rate as f32 * seconds) as usize)
            .flat_map(|i| {
                let t = i as f32 / rate as f32;
                let level = if t < 0.3 {
                    t / 0.3
                } else if t < bowed {
                    1.0
                } else {
                    10f32.powf(-4.0 * (t - bowed) / ring)
                };
                let s = (t * 330.0 * std::f32::consts::TAU).sin() * 0.7 * level;
                [s, s]
            })
            .collect();
        Take {
            samples,
            ..take(64, 0)
        }
    }

    #[test]
    fn the_steady_part_of_a_note_is_where_the_bow_is_on_the_string() {
        let note = bowed_note(5.0, 7.0);
        let mono: Vec<f32> = note
            .samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|f| f[0])
            .collect();
        let (from, to) = plateau(&mono, 44_100, 0.0);
        let seconds = |frames: usize| frames as f32 / 44_100.0;
        assert!(
            (0.1..0.5).contains(&seconds(from)),
            "starts {}",
            seconds(from)
        );
        // It ends where the sound has fallen to half, a little after the
        // bow lifts (-6 dB is 0.3 of the 7 s ring, 80 dB).
        assert!((5.0..5.8).contains(&seconds(to)), "ends {}", seconds(to));
        // A note with no ring-out: a steady stretch of the length wanted
        // (STEADY when nothing is asked, the whole of it when more is).
        let steady = bowed_note(8.0, 0.4);
        let mono: Vec<f32> = steady
            .samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|f| f[0])
            .collect();
        let (from, to) = plateau(&mono, 44_100, 0.0);
        assert!(
            (seconds(to - from) - STEADY).abs() < 0.1,
            "{} to {}",
            seconds(from),
            seconds(to)
        );
        let (from, to) = plateau(&mono, 44_100, 20.0);
        assert!(
            seconds(to - from) > 7.0,
            "{} to {}",
            seconds(from),
            seconds(to)
        );
    }

    #[test]
    fn a_loop_stays_in_the_steady_part_and_does_not_swell_each_time_round() {
        let mut note = bowed_note(5.0, 7.0);
        bake_loop(&mut note, 1.2, 0.0);
        let (start, end) = note.looped.unwrap();
        let (start, end) = (start as usize, end as usize);
        let seconds = |frames: usize| frames as f32 / 44_100.0;
        assert!(seconds(end) < 5.9, "the loop ends at {}", seconds(end));
        assert_eq!(note.samples.len(), end * 2, "the ring-out is cut away");
        // Every half second of the loop as loud as any other, to 2 dB.
        let rms = |from: usize| {
            let part = &note.samples[from * 2..(from + 22_050) * 2];
            (part.iter().map(|s| s * s).sum::<f32>() / part.len() as f32).sqrt()
        };
        let levels: Vec<f32> = (start..end - 22_050).step_by(11_025).map(rms).collect();
        let (low, high) = levels
            .iter()
            .fold((f32::MAX, 0.0f32), |(l, h), x| (l.min(*x), h.max(*x)));
        assert!(high / low < 1.26, "{low} to {high} in the loop");
    }

    #[test]
    fn notes_of_a_layer_come_towards_one_loudness_and_layers_keep_a_share_of_their_difference() {
        // Notes of 0.5, 0.05 and 0.005 (a 40 dB spread) in a soft layer and
        // one of 0.5 in a loud layer.
        let tone = |amplitude: f32, layer: usize| {
            let samples: Vec<f32> = (0..44_100 * 3)
                .flat_map(|i| {
                    let s = (i as f32 * 0.05).sin() * amplitude;
                    [s, s]
                })
                .collect();
            Take {
                samples,
                ..take(60, layer)
            }
        };
        let mut takes = vec![tone(0.5, 0), tone(0.05, 0), tone(0.005, 0), tone(0.5, 1)];
        let before: Vec<f32> = takes.iter().map(loudness).collect();
        level_notes(&mut takes);
        let after: Vec<f32> = takes.iter().map(loudness).collect();
        // The 40 dB between the loudest and the quietest note is now 12 dB
        // (30 % of it left): the median stays, the others come to it.
        let spread = |levels: &[f32]| 20.0 * (levels[0] / levels[2]).log10();
        assert!((spread(&before) - 40.0).abs() < 0.1);
        assert!((spread(&after) - 12.0).abs() < 0.3, "{}", spread(&after));
        assert!(
            (after[1] / before[1] - 1.0).abs() < 1e-4,
            "the median note is left"
        );
        assert!(
            (after[3] / before[3] - 1.0).abs() < 1e-4,
            "another layer is left"
        );

        // Layers: each has the file's full range, and the gain brings
        // the quieter back down by only part of the difference.
        let mut layered = vec![tone(0.05, 0), tone(0.5, 1)];
        level_notes(&mut layered);
        let chosen = scales(&layered);
        let played = |at: usize| loudness(&layered[at]) * chosen[at].0 * chosen[at].1;
        for (take, (scale, _)) in layered.iter().zip(&chosen) {
            assert!(
                (peak(&take.samples) * scale - PEAK).abs() < 1e-4,
                "full range"
            );
        }
        // 20 dB apart; the soft layer is lifted by LAYER_LEVELLING of it.
        let apart = 20.0 * (played(1) / played(0)).log10();
        let wanted = 20.0 * (1.0 - LAYER_LEVELLING);
        assert!(
            (apart - wanted).abs() < 0.3,
            "{apart} dB apart, not {wanted}"
        );
        // The loud layer is the voice's reference: its gain is 1.
        assert!((chosen[1].1 - 1.0).abs() < 1e-4);
        // A voice with one layer is as it was: one scale, a gain of 1.
        let mut one = vec![tone(0.3, 0), tone(0.1, 0)];
        level_notes(&mut one);
        assert!(
            scales(&one)
                .iter()
                .all(|(_, gain)| (gain - 1.0).abs() < 1e-4)
        );
    }

    #[test]
    fn no_note_or_layer_is_raised_further_than_its_limit() {
        let tone = |amplitude: f32, layer: usize| {
            let samples: Vec<f32> = (0..44_100 * 3)
                .flat_map(|i| {
                    let s = (i as f32 * 0.05).sin() * amplitude;
                    [s, s]
                })
                .collect();
            Take {
                samples,
                ..take(60, layer)
            }
        };
        // A note 66 dB under the median is raised by NOTE_LIMIT, no more.
        let mut takes = vec![tone(0.5, 0), tone(0.5, 0), tone(0.00025, 0)];
        let before = loudness(&takes[2]);
        level_notes(&mut takes);
        let raised = loudness(&takes[2]) / before;
        assert!((raised - NOTE_LIMIT).abs() < 0.01, "raised {raised} times");
        // A layer 80 dB under another is raised by LAYER_BOOST, no more.
        let layered = vec![tone(0.00005, 0), tone(0.5, 1)];
        let chosen = scales(&layered);
        let voice = PEAK / peak(&layered[1].samples);
        // gain = voice / scale * boost, so boost = gain * scale / voice.
        let lifted = chosen[0].1 * chosen[0].0 / voice;
        assert!((lifted - LAYER_BOOST).abs() < 0.01, "lifted {lifted} times");
        let loud = chosen[1].1 * chosen[1].0 / voice;
        assert!((loud - 1.0).abs() < 1e-4, "the loudest layer is not lifted");
    }

    #[test]
    fn a_pluck_is_as_loud_as_its_pluck_and_a_held_note_as_its_loop() {
        let rate = 44_100usize;
        // A pluck: loud for 50 ms, then a long quiet tail.
        let pluck: Vec<f32> = (0..rate * 4)
            .flat_map(|i| {
                let level = if i < rate / 20 { 0.8 } else { 0.01 };
                let s = (i as f32 * 0.1).sin() * level;
                [s, s]
            })
            .collect();
        let pluck = Take {
            samples: pluck,
            ..take(60, 0)
        };
        let heard = loudness(&pluck);
        assert!(heard > 0.3, "the pluck's own level, not its tail: {heard}");
        // A held note is what it is inside its loop, whatever comes before.
        let mut held = bowed_note(5.0, 7.0);
        let loud = loudness(&held);
        held.looped = Some((2 * rate as u64, 4 * rate as u64));
        let in_loop = loudness(&held);
        assert!(in_loop > 0.4 && (in_loop - loud).abs() > 1e-4 || in_loop > 0.4);
    }

    #[test]
    fn a_note_that_swells_all_the_way_still_gets_a_loop_to_hide_its_join_in() {
        // Rising for ten seconds, cut at seven: no steady part by its
        // level, but the loop still has at least MIN_STEADY to run in.
        let rate = 44_100usize;
        let samples: Vec<f32> = (0..rate * 7)
            .flat_map(|i| {
                let t = i as f32 / rate as f32;
                let s = (t * 440.0 * std::f32::consts::TAU).sin() * (t / 10.0);
                [s, s]
            })
            .collect();
        let mut note = Take {
            samples,
            ..take(69, 0)
        };
        let mono: Vec<f32> = note
            .samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|f| f[0])
            .collect();
        let (from, to) = plateau(&mono, 44_100, 0.0);
        assert!(
            to - from >= (MIN_STEADY * 44_100.0) as usize - 1,
            "{from} to {to}"
        );
        bake_loop(&mut note, 1.2, 0.0);
        let (start, end) = note.looped.unwrap();
        assert!(
            end - start >= MIN_LOOP_FRAMES,
            "a loop of {} frames",
            end - start
        );
        // And the steady stretch can be capped.
        let (from, to) = plateau(&mono, 44_100, 2.0);
        assert!(to - from <= 2 * 44_100, "{from} to {to}");
    }

    #[test]
    fn a_join_between_tones_that_drift_apart_does_not_dip() {
        // 330 Hz gliding up by half a hertz over eight seconds: the two
        // ends of any loop are slightly different pitches, in and out of
        // step along the blend.
        let rate = 44_100usize;
        let samples: Vec<f32> = (0..rate * 8)
            .flat_map(|i| {
                let t = i as f32 / rate as f32;
                let phase = 330.0 * t + 0.5 * t * t / 16.0;
                let s = (phase * std::f32::consts::TAU).sin() * 0.5;
                [s, s]
            })
            .collect();
        let mut note = Take {
            samples,
            ..take(64, 0)
        };
        bake_loop(&mut note, 1.2, 0.0);
        let (start, end) = note.looped.unwrap();
        let (start, end) = (start as usize, end as usize);
        // The loudest and quietest 50 ms of the whole loop, ends included.
        let window = 2205;
        let levels: Vec<f32> = (start - 22_050..end - window)
            .step_by(window / 2)
            .map(|from| {
                let part = &note.samples[from * 2..(from + window) * 2];
                (part.iter().map(|s| s * s).sum::<f32>() / part.len() as f32).sqrt()
            })
            .collect();
        let low = levels.iter().copied().fold(f32::MAX, f32::min);
        let high = levels.iter().copied().fold(0.0f32, f32::max);
        assert!(
            high / low < 1.25,
            "{low} to {high}: a dip of {} dB",
            20.0 * (high / low).log10()
        );
    }

    #[test]
    fn thinning_keeps_every_nth_root_of_a_layer_and_the_last() {
        let source = |root: u8, layer: usize| (PathBuf::new(), take(root, layer));
        let mut takes = vec![source(10, 0)]; // from another source: untouched
        let first = takes.len();
        for root in (40..=60).step_by(2) {
            takes.push(source(root, 0));
            takes.push(source(root, 0)); // two takes of a root go together
        }
        takes.push(source(41, 1));
        takes.push(source(43, 1));
        thin(&mut takes, first, 3);
        let mut roots: Vec<(u8, usize)> = takes[1..]
            .iter()
            .map(|(_, take)| (take.root, take.layer))
            .collect();
        roots.dedup();
        // Layer 0 has eleven roots: the 1st, 4th, 7th, 10th and the last.
        assert_eq!(
            roots,
            [
                (40, 0),
                (46, 0),
                (52, 0),
                (58, 0),
                (60, 0),
                (41, 1),
                (43, 1)
            ]
        );
        assert_eq!(takes[0].1.root, 10);
        let kept = takes[1..]
            .iter()
            .filter(|(_, take)| take.root == 40)
            .count();
        assert_eq!(kept, 2, "both takes of a root, or neither");
        // One is the default.
        let mut all = vec![source(40, 0), source(42, 0)];
        thin(&mut all, 0, 1);
        assert_eq!(all.len(), 2);
    }

    fn entry(instrument: Instrument, voice: u8) -> InstrumentManifest {
        let mut entry = InstrumentManifest::new(
            instrument,
            0.01,
            0.5,
            1.0,
            vec![ZoneManifest {
                file: "x.flac".to_owned(),
                root: 60,
                low: 0,
                high: 127,
                velocity: (0.0, 1.0),
                looped: None,
                gain: 1.0,
                pitched: true,
                tune: 0.0,
            }],
        );
        entry.voice = voice;
        entry.name = format!("Voice {voice}");
        entry.license = "CC0-1.0".to_owned();
        entry.credit = "Somebody".to_owned();
        entry
    }

    fn manifest(instruments: Vec<InstrumentManifest>) -> Manifest {
        Manifest {
            version: PACK_VERSION,
            instruments,
        }
    }

    #[test]
    fn a_pack_needs_a_name_licence_and_credit_for_every_voice_and_a_voice_0() {
        use olio_audio::ambience::Instrument::{Harp, Piano};
        assert!(
            check_manifest(&manifest(vec![
                entry(Harp, 0),
                entry(Harp, 1),
                entry(Piano, 0)
            ]))
            .is_ok()
        );
        for blank in ["name", "license", "credit"] {
            let mut bad = entry(Harp, 0);
            match blank {
                "name" => bad.name.clear(),
                "license" => bad.license.clear(),
                _ => bad.credit.clear(),
            }
            let err = check_manifest(&manifest(vec![bad])).unwrap_err();
            assert!(err.contains("lacks a name, licence or credit"), "{err}");
        }
        // A loop that buzzes, and a range that strands the recordings.
        let mut buzzing = entry(Harp, 0);
        buzzing.zones[0].looped = Some((1000, 7000));
        let err = check_manifest(&manifest(vec![buzzing])).unwrap_err();
        assert!(err.contains("under a quarter second"), "{err}");
        let mut stranded = entry(Harp, 0);
        stranded.range = Some((40, 52));
        let err = check_manifest(&manifest(vec![stranded])).unwrap_err();
        assert!(err.contains("outside its range"), "{err}");
        let mut fits = entry(Harp, 0);
        fits.range = Some((57, 63));
        assert!(
            check_manifest(&manifest(vec![fits])).is_ok(),
            "root 60 within"
        );
        let err = check_manifest(&manifest(vec![entry(Harp, 1)])).unwrap_err();
        assert!(err.contains("no voice 0"), "{err}");
        let err = check_manifest(&manifest(vec![entry(Harp, 0), entry(Harp, 0)])).unwrap_err();
        assert!(err.contains("twice"), "{err}");
        let mut empty = entry(Harp, 0);
        empty.zones.clear();
        assert!(
            check_manifest(&manifest(vec![empty]))
                .unwrap_err()
                .contains("no recordings")
        );
    }

    #[test]
    fn the_credits_have_a_line_for_every_voice() {
        use olio_audio::ambience::Instrument::Harp;
        let sources = fetch::sources();
        let all: Vec<InstrumentManifest> = RECIPES
            .iter()
            .map(|recipe| {
                let (license, credit) = provenance(recipe, &sources).unwrap();
                let mut e = entry(recipe.instrument, recipe.voice);
                e.name = recipe.name.to_owned();
                e.license = license;
                e.credit = credit;
                e
            })
            .collect();
        let text = credits(&sources, &manifest(all));
        for recipe in RECIPES {
            let line = text
                .lines()
                .find(|line| line.contains(&format!(", {}: ", recipe.name)))
                .unwrap_or_else(|| panic!("no credit for {}", recipe.name));
            assert!(
                line.contains("CC0-1.0") && line.contains("Sam Gossner"),
                "{line}"
            );
            assert!(line.contains(recipe.sources[0].folder));
        }
        assert!(text.contains("CC0 1.0") && text.contains("commit"));
        let _ = Harp;
    }

    #[test]
    fn the_recipes_are_well_formed_and_the_pinned_list_is_exactly_what_they_take() {
        let sources = fetch::sources();
        let mut taken: Vec<(String, String)> = Vec::new();
        let mut dirs: Vec<&str> = Vec::new();
        for recipe in RECIPES {
            assert!(!recipe.name.is_empty(), "{:?}", recipe.instrument);
            assert!(
                !dirs.contains(&recipe.dir),
                "{} is the folder of two voices",
                recipe.dir
            );
            dirs.push(recipe.dir);
            let same = RECIPES
                .iter()
                .filter(|r| r.instrument == recipe.instrument && r.voice == recipe.voice)
                .count();
            assert_eq!(same, 1, "{} twice", recipe.name);
            assert!(
                recipe.voice == 0
                    || RECIPES
                        .iter()
                        .any(|r| r.instrument == recipe.instrument && r.voice == 0),
                "{} has no voice 0",
                recipe.name
            );
            let layers = recipe
                .sources
                .iter()
                .map(|s| s.layers.len().max(1))
                .max()
                .unwrap();
            assert!(
                recipe.blend <= 0.5 / layers as f32 + 1e-6,
                "{} blends too far",
                recipe.name
            );
            if recipe.looped {
                assert!(
                    recipe.loop_blend > 0.0,
                    "{} loops without a blend",
                    recipe.name
                );
            }
            // Every recording it takes is in the pinned list, and the list
            // has nothing else.
            let takes = gather(recipe, &sources, Path::new("")).unwrap();
            assert!(!takes.is_empty());
            for (path, _) in takes {
                let path = path.to_string_lossy().into_owned();
                let (repo, rest) = path.split_once('/').unwrap();
                taken.push((repo.to_owned(), rest.to_owned()));
            }
        }
        taken.sort();
        taken.dedup();
        let mut listed: Vec<(String, String)> = sources
            .files
            .iter()
            .map(|file| (file.repo.clone(), file.path.clone()))
            .collect();
        listed.sort();
        assert_eq!(
            taken, listed,
            "sources.json is not what the recipes take: run `olio-packer pin`"
        );
    }
}

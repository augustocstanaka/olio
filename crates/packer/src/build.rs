//! Building the pack: each recipe's recordings decoded, checked for
//! pitch, trimmed, held ones looped seamlessly, brought to one level per
//! instrument, written as 16-bit FLAC, with the manifest and credits, and
//! zipped with its SHA-256.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use olio_audio::decode_file;
use olio_audio::instruments::{InstrumentManifest, MANIFEST, Manifest, PACK_VERSION, ZoneManifest};
use sha2::{Digest, Sha256};

use crate::fetch::{self, Sources};
use crate::pitch;
use crate::recipes::{RECIPES, Recipe, Source};

/// The loudest note of each instrument peaks here.
const PEAK: f32 = 0.89;
/// The pack's name, and its folder in the zip.
pub const NAME: &str = "olio-instruments";

/// A recording, decoded and placed.
struct Take {
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

/// The recordings a recipe takes, with their notes and layers.
fn gather(recipe: &Recipe, sources: &Sources, dir: &Path) -> Result<Vec<(PathBuf, Take)>, String> {
    let mut takes = Vec::new();
    for source in recipe.sources {
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
    }
    if takes.is_empty() {
        return Err(format!("no recordings for {:?}", recipe.instrument));
    }
    Ok(takes)
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

/// Makes a held note loop: between 40 % and 80 % of it, the end blended
/// into what comes before the loop's start (equal power), so jumping from
/// the end back to the start continues the sound; cut at the loop's end.
fn bake_loop(take: &mut Take) {
    let frames = take.samples.len() / 2;
    let start = frames * 2 / 5;
    let end = frames * 4 / 5;
    let blend = ((0.5 * take.rate as f32) as usize)
        .min((end - start) / 2)
        .min(start);
    for i in 0..blend {
        let t = i as f32 / blend as f32;
        let (fade_out, fade_in) = ((1.0 - t).sqrt(), t.sqrt());
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

/// 16-bit FLAC of interleaved stereo.
fn flac(samples: &[f32], rate: u32) -> Result<Vec<u8>, String> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let ints: Vec<i32> = samples
        .iter()
        .map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i32)
        .collect();
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|err| format!("{err:?}"))?;
    let source = flacenc::source::MemSource::from_samples(&ints, 2, 16, rate as usize);
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

/// The notes each take plays: within a layer, halfway to the next root
/// either side, the first from 0 and the last to 127.
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
                0
            } else {
                (roots[at - 1] + take.root) / 2 + 1
            };
            let high = roots.get(at + 1).map_or(127, |next| (take.root + next) / 2);
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
                        "  {:?} {}: {:+.0} cents ({:+.1} semitones)",
                        recipe.instrument,
                        take.stem,
                        cents,
                        cents / 100.0
                    );
                }
                None => println!("  {:?} {}: no clear pitch", recipe.instrument, take.stem),
            }
        }
        println!(
            "{:?}: {count} recordings, {off} off pitch",
            recipe.instrument
        );
    }
    Ok(())
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
                bake_loop(&mut take);
            } else {
                trim_end(&mut take, recipe.seconds, recipe.fade);
            }
            takes.push(take);
        }
        let loudest = takes
            .iter()
            .map(|take| peak(&take.samples))
            .fold(0.0, f32::max);
        let scale = if loudest > 0.0 { PEAK / loudest } else { 1.0 };
        let folder = pack.join(recipe.dir);
        fs::create_dir_all(&folder).map_err(|err| format!("{}: {err}", folder.display()))?;
        let layers = recipe
            .sources
            .iter()
            .map(|source| source.layers.len().max(1))
            .max()
            .unwrap_or(1);
        let ranges = ranges(&takes);
        let mut zones = Vec::new();
        for (take, (low, high)) in takes.iter_mut().zip(ranges) {
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
            let step = 1.0 / layers as f32;
            zones.push(ZoneManifest {
                file,
                root: take.root,
                low,
                high,
                velocity: (take.layer as f32 * step, (take.layer + 1) as f32 * step),
                looped: take.looped,
                gain: 1.0,
                pitched: take.pitched,
                tune: (take.tune * 10.0).round() / 10.0,
            });
        }
        println!("{:?}: {} recordings", recipe.instrument, zones.len());
        instruments.push(InstrumentManifest {
            instrument: recipe.instrument,
            attack: recipe.attack,
            release: recipe.release,
            gain: recipe.gain,
            zones,
        });
    }
    let manifest = Manifest {
        version: PACK_VERSION,
        instruments,
    };
    let text = serde_json::to_string_pretty(&manifest).map_err(|err| err.to_string())?;
    fs::write(pack.join(MANIFEST), text).map_err(|err| format!("{MANIFEST}: {err}"))?;
    fs::write(pack.join("CREDITS.md"), credits(&sources)).map_err(|err| err.to_string())?;

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
    Ok(())
}

fn credits(sources: &Sources) -> String {
    let mut text = String::from(
        "# Olio instrument pack\n\n\
         Recordings from the Versilian Studios libraries, released by Sam \
         Gossner and Versilian Studios into the public domain (CC0 1.0). \
         Olio's pack trims them, loops the held notes, levels each \
         instrument and re-encodes them as FLAC; it is CC0 as well.\n\n",
    );
    for library in &sources.libraries {
        text.push_str(&format!(
            "- {} ({}), {} at commit {}\n",
            library.name, library.license, library.url, library.commit
        ));
    }
    text.push_str("\nInstruments:\n\n");
    for recipe in RECIPES {
        let folders: Vec<String> = recipe
            .sources
            .iter()
            .map(|source| format!("{} {}", source.repo, source.folder))
            .collect();
        text.push_str(&format!(
            "- {:?}: {}\n",
            recipe.instrument,
            folders.join("; ")
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
        assert_eq!(ranges(&takes), [(0, 50), (51, 56), (57, 127), (0, 127)]);
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
        bake_loop(&mut held);
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
}

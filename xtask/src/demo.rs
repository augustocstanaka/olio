//! A demo sound pack, synthesised so the repository ships no recordings:
//! twelve short sounds and a board that lays them out with keys, colours,
//! loops, fades and reverb. Some start or end with silence, to show
//! trimming.

use std::f32::consts::TAU;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const RATE: u32 = 44_100;

/// A tiny deterministic noise source (xorshift), so the pack is identical
/// on every run.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

fn seconds(duration: f32) -> usize {
    (duration * RATE as f32) as usize
}

fn time(i: usize) -> f32 {
    i as f32 / RATE as f32
}

fn silence(duration: f32) -> Vec<f32> {
    vec![0.0; seconds(duration)]
}

fn tone(frequency: f32, duration: f32, decay: f32) -> Vec<f32> {
    (0..seconds(duration))
        .map(|i| {
            let t = time(i);
            (TAU * frequency * t).sin() * (-t * decay).exp()
        })
        .collect()
}

fn chord(frequencies: &[f32], duration: f32, decay: f32) -> Vec<f32> {
    let mut out = vec![0.0; seconds(duration)];
    for &frequency in frequencies {
        for (sample, value) in out.iter_mut().zip(tone(frequency, duration, decay)) {
            *sample += value / frequencies.len() as f32;
        }
    }
    out
}

fn sweep(from: f32, to: f32, duration: f32) -> Vec<f32> {
    let mut phase = 0.0f32;
    let total = seconds(duration);
    (0..total)
        .map(|i| {
            let progress = i as f32 / total as f32;
            phase += TAU * (from + (to - from) * progress) / RATE as f32;
            phase.sin() * (1.0 - progress).sqrt()
        })
        .collect()
}

fn kick() -> Vec<f32> {
    let mut phase = 0.0f32;
    (0..seconds(0.5))
        .map(|i| {
            let t = time(i);
            phase += TAU * (40.0 + 120.0 * (-t * 25.0).exp()) / RATE as f32;
            phase.sin() * (-t * 7.0).exp()
        })
        .collect()
}

fn snare(noise: &mut Noise) -> Vec<f32> {
    (0..seconds(0.4))
        .map(|i| {
            let t = time(i);
            (noise.next() * 0.7 + (TAU * 190.0 * t).sin() * 0.3) * (-t * 14.0).exp()
        })
        .collect()
}

fn applause(noise: &mut Noise) -> Vec<f32> {
    let total = seconds(3.0);
    let mut out = vec![0.0f32; total];
    // Hundreds of tiny claps at irregular times.
    for _ in 0..600 {
        let start = ((noise.next() * 0.5 + 0.5) * (total - 2_000) as f32) as usize;
        let level = 0.3 + 0.3 * noise.next().abs();
        for j in 0..1_500 {
            out[start + j] += noise.next() * level * (-(j as f32) / 180.0).exp();
        }
    }
    let swell = |i: usize| {
        let t = time(i) / 3.0;
        (t * 4.0).min(1.0) * (1.0 - t).min(0.35) / 0.35
    };
    out.iter_mut()
        .enumerate()
        .for_each(|(i, sample)| *sample *= swell(i) * 0.5);
    out
}

fn siren() -> Vec<f32> {
    let mut phase = 0.0f32;
    (0..seconds(2.4))
        .map(|i| {
            let t = time(i);
            phase += TAU * (700.0 + 250.0 * (TAU * 0.8 * t).sin()) / RATE as f32;
            phase.sin() * 0.6
        })
        .collect()
}

fn arpeggio(notes: &[f32], step: f32) -> Vec<f32> {
    notes
        .iter()
        .flat_map(|&frequency| chord(&[frequency, frequency * 2.0], step, 6.0))
        .collect()
}

fn pad() -> Vec<f32> {
    // Four bars of a minor chord with a slow tremolo; loops cleanly.
    let duration = 4.0;
    (0..seconds(duration))
        .map(|i| {
            let t = time(i);
            let tremolo = 0.75 + 0.25 * (TAU * 0.5 * t).sin();
            [110.0, 130.81, 164.81, 220.0]
                .iter()
                .map(|&f| (TAU * f * t).sin())
                .sum::<f32>()
                * 0.18
                * tremolo
        })
        .collect()
}

fn normalise(samples: &mut [f32], peak: f32) {
    let loudest = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if loudest > 0.0 {
        samples.iter_mut().for_each(|s| *s *= peak / loudest);
    }
}

fn join(parts: &[Vec<f32>]) -> Vec<f32> {
    parts.concat()
}

fn write_wav(path: &Path, samples: &[f32]) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    let data = (samples.len() * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::File::create(path)
        .and_then(|mut file| file.write_all(&bytes))
        .map_err(|err| format!("{}: {err}", path.display()))
}

const BOARD: &str = r#"{
  "sets": [
    {
      "name": "Demo Board",
      "replace": true,
      "columns": 3,
      "tiles": [
        { "file": "Applause.wav", "key": "1", "color": "purple", "fade_out": 1.5, "emoji": "clap" },
        { "file": "Big Win.wav", "key": "2", "color": "pink", "reverb": 0.35, "emoji": "trophy" },
        { "file": "Siren.wav", "key": "3", "color": "red", "loop": true, "fade_in": 0.3, "fade_out": 0.8, "emoji": "rotating_light" },
        { "file": "Tense Pad.wav", "key": "Q", "color": "purple", "loop": true, "fade_in": 2, "fade_out": 2, "reverb": 0.5 },
        { "file": "Bell.wav", "key": "W", "color": "pink", "reverb": 0.6, "emoji": "bell" },
        { "file": "Riser.wav", "key": "E", "color": "red", "fade_in": 0.5 },
        { "file": "Laser.wav", "key": "A", "color": "purple" },
        { "file": "Chime.wav", "key": "S", "color": "pink", "reverb": 0.8 },
        { "file": "Wrong Answer.wav", "key": "D", "color": "red", "emoji": "x" },
        { "file": "Kick.wav", "key": "Z", "color": "purple", "emoji": "drum" },
        { "file": "Snare.wav", "key": "X", "color": "pink", "reverb": 0.25 },
        { "file": "Bloop.wav", "key": "C", "color": "red" }
      ]
    },
    { "name": "Demo Folder", "replace": true, "folder": "." }
  ]
}
"#;

/// Writes the sounds and `board.json` into `dir`; returns the board's path.
pub fn write(dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let mut noise = Noise(0x2545_f491);
    let sounds: Vec<(&str, Vec<f32>)> = vec![
        ("Applause", applause(&mut noise)),
        (
            "Big Win",
            arpeggio(&[523.25, 659.25, 783.99, 1046.5, 1318.5], 0.12),
        ),
        ("Siren", siren()),
        ("Tense Pad", pad()),
        // Silence before and after: trimmed on the board.
        (
            "Bell",
            join(&[
                silence(0.6),
                chord(&[880.0, 1320.0, 2210.0], 2.0, 2.2),
                silence(0.8),
            ]),
        ),
        ("Riser", sweep(200.0, 2000.0, 2.0)),
        ("Laser", sweep(2400.0, 180.0, 0.35)),
        (
            "Chime",
            join(&[silence(0.3), arpeggio(&[1567.98, 1318.51, 1046.5], 0.35)]),
        ),
        (
            "Wrong Answer",
            join(&[tone(311.13, 0.35, 1.0), tone(233.08, 0.7, 2.0)]),
        ),
        ("Kick", kick()),
        ("Snare", snare(&mut noise)),
        ("Bloop", join(&[sweep(300.0, 900.0, 0.18), silence(0.4)])),
    ];
    for (name, mut samples) in sounds {
        normalise(&mut samples, 0.8);
        write_wav(&dir.join(format!("{name}.wav")), &samples)?;
    }
    let board = dir.join("board.json");
    fs::write(&board, BOARD).map_err(|err| format!("{}: {err}", board.display()))?;
    Ok(board)
}

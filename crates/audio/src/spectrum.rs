//! The spectrogram drawn under a tile's waveform: how a sound's energy is
//! spread over frequency as it plays. Computed once per clip off the audio
//! thread, with a small FFT of our own.

use std::f32::consts::PI;

use crate::clip::Clip;

/// Frames per analysis window; a power of two for the FFT.
const WINDOW: usize = 1024;
/// The lowest band's bottom edge, in Hz. Below that little of a
/// soundboard's sounds lives, and the bands would be a few bins wide.
const LOW_HZ: f32 = 60.0;
/// Range of levels shown: the loudest cell down to this many dB below it.
const RANGE_DB: f32 = 60.0;

/// A grid of levels: `columns` steps of time by `bands` steps of
/// frequency, low band first, 0 (silent) to 255 (the loudest cell).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spectrogram {
    columns: usize,
    bands: usize,
    /// Row-major by column: `cells[column * bands + band]`.
    cells: Vec<u8>,
}

impl Spectrogram {
    /// Analyses the clip (mixed to mono) into `columns` columns and `bands`
    /// log-spaced bands from [`LOW_HZ`] to half the sample rate.
    pub fn of(clip: &Clip, columns: usize, bands: usize) -> Self {
        let columns = columns.max(1);
        let bands = bands.max(1);
        let frames = clip.frames();
        let mut cells = vec![0u8; columns * bands];
        if frames == 0 {
            return Self {
                columns,
                bands,
                cells,
            };
        }
        let window: Vec<f32> = (0..WINDOW)
            .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / WINDOW as f32).cos())
            .collect();
        let edges = band_edges(clip.rate(), bands);
        let mut buffer = vec![Complex::ZERO; WINDOW];
        let mut levels = vec![0.0f32; columns * bands];
        let mut loudest = 0.0f32;
        for column in 0..columns {
            // Each column looks at a window centred on its share of the clip.
            let centre = (column as f64 + 0.5) / columns as f64 * frames as f64;
            let start = (centre as i64 - WINDOW as i64 / 2).max(0) as usize;
            for (n, (slot, weight)) in buffer.iter_mut().zip(&window).enumerate() {
                let frame = start + n;
                *slot = if frame < frames {
                    let (left, right) = clip.stereo_frame(frame);
                    Complex::real(0.5 * (left + right) * weight)
                } else {
                    Complex::ZERO
                };
            }
            fft(&mut buffer);
            for band in 0..bands {
                let (from, to) = (edges[band], edges[band + 1].max(edges[band] + 1));
                let energy: f32 = buffer[from..to].iter().map(Complex::norm_squared).sum();
                let level = energy / (to - from) as f32;
                levels[column * bands + band] = level;
                loudest = loudest.max(level);
            }
        }
        if loudest > 0.0 {
            for (cell, &level) in cells.iter_mut().zip(&levels) {
                let db = 10.0 * (level / loudest).max(1e-12).log10();
                *cell = ((db + RANGE_DB) / RANGE_DB * 255.0).clamp(0.0, 255.0) as u8;
            }
        }
        Self {
            columns,
            bands,
            cells,
        }
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn bands(&self) -> usize {
        self.bands
    }

    /// The level at a column and band (band 0 is the lowest).
    pub fn cell(&self, column: usize, band: usize) -> u8 {
        self.cells[column * self.bands + band]
    }

    /// Every cell, column by column, low band first within each.
    pub fn cells(&self) -> &[u8] {
        &self.cells
    }
}

/// FFT bin indices bounding each band: `bands + 1` edges, log-spaced from
/// [`LOW_HZ`] to the Nyquist frequency.
fn band_edges(rate: u32, bands: usize) -> Vec<usize> {
    let nyquist = rate as f32 / 2.0;
    let bin_hz = rate as f32 / WINDOW as f32;
    let low = LOW_HZ.min(nyquist / 2.0);
    (0..=bands)
        .map(|band| {
            let hz = low * (nyquist / low).powf(band as f32 / bands as f32);
            ((hz / bin_hz).round() as usize).clamp(1, WINDOW / 2)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    const ZERO: Self = Self { re: 0.0, im: 0.0 };

    fn real(re: f32) -> Self {
        Self { re, im: 0.0 }
    }

    fn norm_squared(&self) -> f32 {
        self.re * self.re + self.im * self.im
    }

    fn mul(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }

    fn add(self, other: Self) -> Self {
        Self {
            re: self.re + other.re,
            im: self.im + other.im,
        }
    }

    fn sub(self, other: Self) -> Self {
        Self {
            re: self.re - other.re,
            im: self.im - other.im,
        }
    }
}

/// An in-place radix-2 FFT; the length must be a power of two.
fn fft(data: &mut [Complex]) {
    let n = data.len();
    assert!(n.is_power_of_two(), "the FFT length must be a power of two");
    // Bit-reversal permutation.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            data.swap(i, j);
        }
    }
    let mut length = 2;
    while length <= n {
        let angle = -2.0 * PI / length as f32;
        let root = Complex {
            re: angle.cos(),
            im: angle.sin(),
        };
        for start in (0..n).step_by(length) {
            let mut twiddle = Complex::real(1.0);
            for k in 0..length / 2 {
                let even = data[start + k];
                let odd = data[start + k + length / 2].mul(twiddle);
                data[start + k] = even.add(odd);
                data[start + k + length / 2] = even.sub(odd);
                twiddle = twiddle.mul(root);
            }
        }
        length *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dft(input: &[Complex]) -> Vec<Complex> {
        let n = input.len();
        (0..n)
            .map(|k| {
                let mut sum = Complex::ZERO;
                for (t, x) in input.iter().enumerate() {
                    let angle = -2.0 * PI * (k * t) as f32 / n as f32;
                    sum = sum.add(x.mul(Complex {
                        re: angle.cos(),
                        im: angle.sin(),
                    }));
                }
                sum
            })
            .collect()
    }

    #[test]
    fn the_fft_matches_a_plain_dft() {
        let input: Vec<Complex> = (0..64)
            .map(|n| Complex::real(((n * 7) % 11) as f32 / 11.0 - 0.5))
            .collect();
        let mut fast = input.clone();
        fft(&mut fast);
        for (a, b) in fast.iter().zip(dft(&input)) {
            assert!((a.re - b.re).abs() < 1e-3 && (a.im - b.im).abs() < 1e-3);
        }
    }

    fn sine(rate: u32, hz: f32, seconds: f32) -> Clip {
        let samples: Vec<f32> = (0..(rate as f32 * seconds) as usize)
            .map(|n| (2.0 * PI * hz * n as f32 / rate as f32).sin() * 0.5)
            .collect();
        Clip::new(rate, 1, samples)
    }

    #[test]
    fn a_tone_lights_its_band_and_leaves_the_rest_dark() {
        let rate = 48_000;
        let spectrogram = Spectrogram::of(&sine(rate, 1000.0, 1.0), 8, 24);
        let edges = band_edges(rate, 24);
        let bin = 1000.0 / (rate as f32 / WINDOW as f32);
        let tone = (0..24)
            .find(|&band| (edges[band] as f32..edges[band + 1] as f32).contains(&bin))
            .unwrap();
        for column in 0..8 {
            assert!(spectrogram.cell(column, tone) >= 240, "column {column}");
            for band in 0..24usize {
                if band.abs_diff(tone) > 1 {
                    assert!(
                        spectrogram.cell(column, band) < 120,
                        "band {band} at column {column} is {}",
                        spectrogram.cell(column, band)
                    );
                }
            }
        }
    }

    #[test]
    fn silence_and_emptiness_are_all_zero() {
        let silent = Clip::new(44_100, 2, vec![0.0; 8820]);
        assert!(
            Spectrogram::of(&silent, 16, 8)
                .cells()
                .iter()
                .all(|&c| c == 0)
        );
        let empty = Spectrogram::of(&Clip::new(44_100, 1, Vec::new()), 4, 3);
        assert_eq!(
            (empty.columns(), empty.bands(), empty.cells().len()),
            (4, 3, 12)
        );
    }

    #[test]
    fn bands_rise_from_the_low_edge_to_the_nyquist() {
        let edges = band_edges(48_000, 24);
        assert_eq!(edges.len(), 25);
        assert!(edges.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(edges[24], WINDOW / 2);
        assert!(edges[0] >= 1);
    }
}

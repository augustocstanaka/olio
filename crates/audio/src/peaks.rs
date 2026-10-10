//! Waveform overviews.

use crate::clip::Clip;

/// The lowest and highest sample in each of a number of equal slices of a
/// clip, over all its channels: enough to draw its waveform at any width up
/// to the number of slices.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Peaks {
    slices: Vec<(f32, f32)>,
}

impl Peaks {
    /// Computes `slices` peaks for `clip`, or one per frame if the clip is
    /// shorter than that.
    pub fn of(clip: &Clip, slices: usize) -> Self {
        let frames = clip.frames();
        let slices = slices.min(frames);
        if slices == 0 {
            return Self::default();
        }
        let channels = usize::from(clip.channels());
        let mut peaks = vec![(f32::INFINITY, f32::NEG_INFINITY); slices];
        for (frame, samples) in clip.samples().chunks_exact(channels).enumerate() {
            let slice = frame * slices / frames;
            let (low, high) = &mut peaks[slice];
            for &sample in samples {
                *low = low.min(sample);
                *high = high.max(sample);
            }
        }
        Self { slices: peaks }
    }

    pub fn len(&self) -> usize {
        self.slices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slices.is_empty()
    }

    /// `(lowest, highest)` sample of each slice, in order.
    pub fn slices(&self) -> &[(f32, f32)] {
        &self.slices
    }

    /// The peaks regrouped into `columns` equal columns for drawing: each
    /// column is the extreme of the slices it covers. With more columns than
    /// slices, slices repeat.
    pub fn columns(&self, columns: usize) -> Vec<(f32, f32)> {
        self.columns_between(columns, 0.0, 1.0)
    }

    /// Like [`columns`](Self::columns), for the part of the clip between
    /// fractions `from` and `to` of its length.
    pub fn columns_between(&self, columns: usize, from: f64, to: f64) -> Vec<(f32, f32)> {
        let len = self.slices.len();
        if len == 0 || columns == 0 {
            return Vec::new();
        }
        let from = from.clamp(0.0, 1.0);
        let to = to.clamp(from, 1.0);
        let first = ((from * len as f64) as usize).min(len - 1);
        let span = (((to - from) * len as f64).round() as usize).clamp(1, len - first);
        (0..columns)
            .map(|column| {
                let start = first + column * span / columns;
                let end = (first + (column + 1) * span / columns)
                    .max(start + 1)
                    .min(len);
                self.slices[start..end].iter().fold(
                    (f32::INFINITY, f32::NEG_INFINITY),
                    |(low, high), &(slice_low, slice_high)| {
                        (low.min(slice_low), high.max(slice_high))
                    },
                )
            })
            .collect()
    }
}

//! Note names and pitches: reading "A#3" from a file name, and measuring
//! the pitch actually recorded (YIN), so a mislabelled or octave-shifted
//! library shows up before it is packed.

/// The MIDI note of a name like "C4", "A#-1" or "Bb2", with `octave`
/// added to the octave written (libraries number them differently).
pub fn note(name: &str, octave: i32) -> Option<u8> {
    let mut chars = name.chars();
    let class = match chars.next()? {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (shift, number) = match rest.chars().next()? {
        '#' => (1, &rest[1..]),
        'b' if rest.len() > 1 => (-1, &rest[1..]),
        _ => (0, rest),
    };
    let written: i32 = number.parse().ok()?;
    let midi = (written + octave + 1) * 12 + class + shift;
    u8::try_from(midi).ok().filter(|midi| *midi <= 127)
}

/// The note named in a file's stem: its first `_`-separated part that is
/// a note name.
pub fn note_in(stem: &str, octave: i32) -> Option<u8> {
    stem.split('_').find_map(|part| note(part, octave))
}

/// The frequency of a MIDI note.
pub fn hz(midi: u8) -> f32 {
    440.0 * 2f32.powf((f32::from(midi) - 69.0) / 12.0)
}

/// The fundamental of `samples` (mono) at `rate`, between 25 Hz and
/// 4.5 kHz, by YIN: the lag whose cumulative-mean-normalised difference
/// first dips under a threshold, refined between frames. `None` if the
/// sound has no clear pitch.
pub fn detect(samples: &[f32], rate: u32) -> Option<f32> {
    let max_lag = (rate as f32 / 25.0) as usize;
    let min_lag = ((rate as f32 / 4500.0) as usize).max(2);
    if samples.len() < max_lag * 2 + 2 {
        return None;
    }
    let window = samples.len() - max_lag - 1;
    let mut difference = vec![0.0f32; max_lag + 2];
    for (lag, value) in difference.iter_mut().enumerate().skip(1) {
        *value = samples[..window]
            .iter()
            .zip(&samples[lag..lag + window])
            .map(|(a, b)| (a - b) * (a - b))
            .sum();
    }
    let mut normalised = vec![1.0f32; max_lag + 2];
    let mut running = 0.0;
    for lag in 1..=max_lag + 1 {
        running += difference[lag];
        normalised[lag] = if running > 0.0 {
            difference[lag] * lag as f32 / running
        } else {
            1.0
        };
    }
    let mut lag = (min_lag..=max_lag).find(|&lag| normalised[lag] < 0.15);
    if lag.is_none() {
        // No clear dip: the deepest, if it is deep enough.
        let (best, value) = (min_lag..=max_lag)
            .map(|lag| (lag, normalised[lag]))
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        lag = (value < 0.4).then_some(best);
    }
    let mut lag = lag?;
    while lag < max_lag && normalised[lag + 1] < normalised[lag] {
        lag += 1;
    }
    let (a, b, c) = (normalised[lag - 1], normalised[lag], normalised[lag + 1]);
    let bend = a - 2.0 * b + c;
    let shift = if bend.abs() > f32::EPSILON {
        0.5 * (a - c) / bend
    } else {
        0.0
    };
    Some(rate as f32 / (lag as f32 + shift))
}

/// How far `measured` is from `expected`, in cents.
pub fn cents(measured: f32, expected: f32) -> f32 {
    1200.0 * (measured / expected).log2()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names_read_as_midi_whatever_the_octave_numbering() {
        assert_eq!(note("C4", 0), Some(60));
        assert_eq!(note("A#3", 0), Some(58));
        assert_eq!(note("Bb2", 0), Some(46));
        assert_eq!(note("A#-1", 1), Some(22), "A#0, counted an octave low");
        assert_eq!(note("H3", 0), None);
        assert_eq!(note("v1", 0), None);
        assert_eq!(note_in("GPiano_sus_A#-1_v2_rr1_Player", 1), Some(22));
        assert_eq!(note_in("LLVln_Pizz_A3_f_RR1", 0), Some(57));
        assert_eq!(note_in("HDrumL_Hit_v2_rr1_Sum", 0), None);
    }

    #[test]
    fn the_pitch_of_a_recording_is_measured_from_the_sound() {
        let rate = 44_100;
        for midi in [28u8, 45, 60, 81, 100] {
            let f = hz(midi);
            // A few harmonics, as an instrument has.
            let samples: Vec<f32> = (0..16_384)
                .map(|i| {
                    let t = i as f32 / rate as f32 * std::f32::consts::TAU * f;
                    t.sin() + 0.5 * (2.0 * t).sin() + 0.25 * (3.0 * t).sin()
                })
                .collect();
            let measured = detect(&samples, rate).unwrap();
            assert!(cents(measured, f).abs() < 10.0, "{midi}: {measured} Hz");
        }
        let silence = vec![0.0; 16_384];
        assert_eq!(detect(&silence, rate), None);
    }
}

//! Decides what the instruments play and when, without making a sound.
//!
//! A slow harmony (a chord degree of the scale) underlies everything: the
//! bass and strings lean on it, the harp and bells mostly touch its tones,
//! the pad voices it. Declarative presence puts the notes on a pulse and
//! walks a cadence; the other presences drift. Pieces have an intro, a
//! close that thins out and lands on the root, and a pause.

use super::{AmbienceSettings, Form, Instrument, Rng};

/// One note for an instrument.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub instrument: Instrument,
    pub midi: u8,
    /// 0 to 1.
    pub velocity: f32,
    /// -1 (left) to 1 (right).
    pub pan: f32,
    /// Frames to hold a sustained note; 0 lets a plucked or struck one
    /// ring out on its own.
    pub duration: u64,
}

/// A note at a time, in output frames since the start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub at: u64,
    pub note: Note,
}

/// Most events waiting to play. Pads voice three notes at once, phrases
/// queue a few; more than this and the extra are dropped.
const QUEUE: usize = 32;

/// `seconds` as a count of frames at `rate`.
fn frames(rate: u32, seconds: f32) -> u64 {
    (seconds.max(0.0) * rate as f32) as u64
}

/// Where a piece is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Playing,
    /// Weather only, until this frame.
    Pausing(u64),
}

#[derive(Clone, Debug)]
pub struct Composer {
    rate: u32,
    rng: Rng,
    seed: u64,
    started: bool,
    /// Events due, in time order.
    queue: [Option<Event>; QUEUE],
    queued: usize,
    /// When each instrument next plays, by [`Instrument::ALL`] index.
    next: [u64; 6],
    /// The harp's last scale degree and how many notes of a phrase are left.
    harp_degree: i32,
    phrase_left: u8,
    /// The harmony: a chord on this scale degree, until this frame.
    chord: i32,
    next_harmony: u64,
    /// Where Declarative presence is in its cadence.
    cadence_step: usize,
    /// The drone's last note, so it is only retuned on a change.
    drone_midi: Option<u8>,
    /// A closing piece has reached its last chord, and whether the bass
    /// has sounded the root to end on.
    home: bool,
    ended_on_root: bool,
    /// The piece.
    origin: u64,
    phase: Phase,
    /// What the instruments play at: 1, falling to 0 as a piece closes.
    music_gain: f32,
}

/// Declarative presence walks this, a chord every two bars: I IV V I,
/// then VI IV V I.
const CADENCE: [i32; 8] = [0, 3, 4, 0, 5, 3, 4, 0];

impl Composer {
    pub fn new(rate: u32, seed: u64) -> Self {
        Self {
            rate,
            rng: Rng::new(seed),
            seed,
            started: false,
            queue: [None; QUEUE],
            queued: 0,
            next: [0; 6],
            harp_degree: 4,
            phrase_left: 0,
            chord: 0,
            next_harmony: 0,
            cadence_step: 0,
            drone_midi: None,
            home: false,
            ended_on_root: false,
            origin: 0,
            phase: Phase::Playing,
            music_gain: 1.0,
        }
    }

    /// Starts afresh with `seed` from the next poll.
    pub fn reset(&mut self, seed: u64) {
        self.seed = seed;
        self.started = false;
        self.clear_queue();
        self.drone_midi = None;
    }

    fn clear_queue(&mut self) {
        self.queued = 0;
        self.queue = [None; QUEUE];
    }

    fn start(&mut self, settings: &AmbienceSettings, now: u64) {
        self.rng = Rng::new(self.seed);
        self.clear_queue();
        self.origin = now;
        self.phase = Phase::Playing;
        self.music_gain = 1.0;
        self.chord = 0;
        self.cadence_step = 0;
        self.next_harmony = now + self.harmony_span(settings);
        self.harp_degree = 4;
        self.phrase_left = 0;
        self.home = false;
        self.ended_on_root = false;
        for instrument in Instrument::ALL {
            // The pad and drone open; the rest come in a little later.
            let wait = match instrument {
                Instrument::Pad | Instrument::Drone => self.rng.range(0.3, 1.2),
                _ => settings.spacing(instrument) * self.rng.range(0.25, 0.7),
            };
            self.next[instrument.index()] = now + frames(self.rate, wait);
        }
        self.drone_midi = None;
        self.started = true;
    }

    /// Frames per beat of the pulse.
    fn beat(&self, settings: &AmbienceSettings) -> u64 {
        (60.0 / f32::from(settings.tempo.max(1)) * self.rate as f32) as u64
    }

    /// How long the harmony stays put.
    fn harmony_span(&mut self, settings: &AmbienceSettings) -> u64 {
        if settings.presence.pulsed() {
            self.beat(settings) * 8
        } else {
            frames(self.rate, self.rng.range(18.0, 45.0))
        }
    }

    /// The gain the instruments play at now: 1, or less as a piece closes.
    pub fn music_gain(&self) -> f32 {
        self.music_gain
    }

    /// Takes the next event due at or before `now`.
    pub fn due(&mut self, now: u64) -> Option<Event> {
        let first = self.queue[0]?;
        if first.at > now {
            return None;
        }
        self.queue.copy_within(1..self.queued, 0);
        self.queued -= 1;
        self.queue[self.queued] = None;
        Some(first)
    }

    fn push(&mut self, event: Event) {
        if self.queued == QUEUE {
            return;
        }
        // Keep the queue in time order.
        let mut index = self.queued;
        while index > 0 && self.queue[index - 1].is_some_and(|earlier| earlier.at > event.at) {
            self.queue[index] = self.queue[index - 1];
            index -= 1;
        }
        self.queue[index] = Some(event);
        self.queued += 1;
    }

    /// Plans the events for `from..to`.
    pub fn poll(&mut self, settings: &AmbienceSettings, from: u64, to: u64) {
        if !self.started {
            self.start(settings, from);
        }
        if let Phase::Pausing(until) = self.phase {
            if from >= until {
                // The next piece, a variation of this one.
                self.seed = self.seed.wrapping_add(1);
                self.start(settings, from);
            } else {
                return;
            }
        }
        let progress = self.piece_progress(settings, from);
        if let Form::Piece {
            minutes,
            pause_seconds,
        } = settings.form
        {
            // The piece ends when this poll reaches its end: nothing is
            // planned past it, and nothing already planned plays on.
            let end = self.origin + frames(self.rate, minutes * 60.0);
            if to >= end {
                self.phase = Phase::Pausing(end + frames(self.rate, pause_seconds));
                self.music_gain = 0.0;
                self.clear_queue();
                return;
            }
        }
        self.music_gain = match progress {
            Some(p) if p > 0.95 => ((1.0 - p) / 0.05).clamp(0.0, 1.0),
            _ => 1.0,
        };
        self.move_harmony(settings, from, progress);
        for instrument in Instrument::ALL {
            let layer = settings.layer(instrument);
            if !layer.enabled {
                continue;
            }
            match instrument {
                Instrument::Drone => self.drone(settings, from),
                _ => self.schedule(settings, instrument, from, to, progress),
            }
        }
    }

    /// How far through a piece, 0 to 1; `None` when endless.
    fn piece_progress(&self, settings: &AmbienceSettings, now: u64) -> Option<f32> {
        match settings.form {
            Form::Endless => None,
            Form::Piece { minutes, .. } => {
                let length = frames(self.rate, minutes * 60.0).max(1);
                Some((now.saturating_sub(self.origin)) as f32 / length as f32)
            }
        }
    }

    /// Moves to the next chord when its time is up; a closing piece heads
    /// for the dominant and then the root.
    fn move_harmony(&mut self, settings: &AmbienceSettings, now: u64, progress: Option<f32>) {
        let closing = progress.is_some_and(|p| p >= 0.85);
        if closing {
            let home = progress.is_some_and(|p| p >= 0.92);
            if home && !self.home {
                // The bass ends the piece on the root straight away.
                self.next[Instrument::Bass.index()] = now;
            }
            self.home = home;
            let wanted = if home { 0 } else { 4 };
            if self.chord != wanted {
                self.chord = wanted;
                self.next[Instrument::Pad.index()] = now;
            }
            return;
        }
        if now < self.next_harmony {
            return;
        }
        self.chord = if settings.presence.pulsed() {
            self.cadence_step = (self.cadence_step + 1) % CADENCE.len();
            CADENCE[self.cadence_step]
        } else {
            // Mostly home, with the subdominant, submediant and supertonic
            // for colour; never the same chord twice running.
            let mut chord = self.chord;
            while chord == self.chord {
                chord = match self.rng.below(10) {
                    0..=3 => 0,
                    4..=5 => 3,
                    6..=7 => 5,
                    8 => 1,
                    _ => 4,
                };
            }
            chord
        };
        self.next_harmony = now + self.harmony_span(settings);
        // The pad voices the new chord soon.
        let pad = Instrument::Pad.index();
        self.next[pad] = self.next[pad].min(now + frames(self.rate, self.rng.range(0.3, 1.5)));
    }

    /// Keeps the drone on the root, retuning it when the key changes.
    fn drone(&mut self, settings: &AmbienceSettings, now: u64) {
        let midi = settings.note(Instrument::Drone, 0);
        if self.drone_midi != Some(midi) {
            self.drone_midi = Some(midi);
            self.push(Event {
                at: now,
                note: Note {
                    instrument: Instrument::Drone,
                    midi,
                    velocity: 1.0,
                    pan: 0.0,
                    duration: 0,
                },
            });
        }
    }

    /// Schedules an instrument's next notes in `from..to`.
    fn schedule(
        &mut self,
        settings: &AmbienceSettings,
        instrument: Instrument,
        from: u64,
        to: u64,
        progress: Option<f32>,
    ) {
        let index = instrument.index();
        // An intro has only the drone and pad; a close thins out.
        let intro = progress.is_some_and(|p| p < 0.08)
            && !matches!(instrument, Instrument::Pad | Instrument::Drone);
        let thinning = progress.map_or(1.0, |p| {
            if p > 0.85 {
                1.0 + 4.0 * (p - 0.85) / 0.15
            } else {
                1.0
            }
        });
        if intro {
            self.next[index] = self.next[index].max(to);
            return;
        }
        let mut guard = 0;
        while self.next[index] < to && guard < 8 {
            guard += 1;
            let mut at = self.next[index].max(from);
            if settings.presence.pulsed() {
                at = self.on_pulse(settings, at, instrument);
            }
            let spacing = settings.spacing(instrument) * thinning;
            if instrument == Instrument::Bass && self.home {
                // The piece ends on the root, once.
                if !self.ended_on_root {
                    self.ended_on_root = true;
                    let midi = settings.note(Instrument::Bass, 0);
                    self.note(at, Instrument::Bass, midi, 0.9, 0.0, 0);
                }
                self.next[index] = to;
                return;
            }
            let next_gap = match instrument {
                Instrument::Bass => self.bass(settings, at, spacing),
                Instrument::Harp => self.harp(settings, at, spacing),
                Instrument::Bells => self.bells(settings, at, spacing),
                Instrument::Strings => self.strings(settings, at, spacing),
                Instrument::Pad => self.pad(settings, at, spacing),
                Instrument::Drone => 0,
            };
            self.next[index] = at + next_gap.max(1);
        }
    }

    /// The next beat at or after `at`; the harp may fall on the half beat.
    fn on_pulse(&self, settings: &AmbienceSettings, at: u64, instrument: Instrument) -> u64 {
        let beat = self.beat(settings).max(1);
        let grid = if instrument == Instrument::Harp {
            (beat / 2).max(1)
        } else {
            beat
        };
        let since = at.saturating_sub(self.origin);
        let steps = since.div_ceil(grid);
        self.origin + steps * grid
    }

    fn note(
        &mut self,
        at: u64,
        instrument: Instrument,
        midi: u8,
        velocity: f32,
        pan: f32,
        duration: u64,
    ) {
        self.push(Event {
            at,
            note: Note {
                instrument,
                midi,
                velocity,
                pan,
                duration,
            },
        });
    }

    /// The root of the chord, its fifth, the third now and then, and the
    /// root an octave down for weight, rarely. Returns the gap to the next.
    fn bass(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let roll = self.rng.unit();
        let degree = self.chord
            + if roll < 0.5 {
                0
            } else if roll < 0.75 {
                4
            } else if roll < 0.9 {
                2
            } else {
                -settings.scale.degrees()
            };
        let midi = settings.note(Instrument::Bass, degree);
        let velocity = self.rng.range(0.55, 0.95);
        let pan = self.rng.range(-0.15, 0.15);
        self.note(at, Instrument::Bass, midi, velocity, pan, 0);
        frames(self.rate, spacing * self.rng.range(0.55, 1.6))
    }

    /// Mostly the chord's tones, else a small step; now and then a short
    /// rising phrase.
    fn harp(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let len = settings.scale.degrees();
        let degree = if self.phrase_left > 0 {
            self.harp_degree + 1 + self.rng.below(2) as i32
        } else if self.rng.chance(0.6) {
            let tones = [0, 2, 4, len, len + 2];
            self.chord + tones[self.rng.below(tones.len() as u32) as usize]
        } else {
            let mut step = 0;
            while step == 0 {
                step = self.rng.below(7) as i32 - 3;
            }
            self.harp_degree + step
        };
        self.harp_degree = degree.clamp(0, 2 * len);
        let midi = settings.note(Instrument::Harp, self.harp_degree);
        let velocity = self.rng.range(0.35, 0.8);
        let pan = self.rng.range(-0.65, 0.65);
        self.note(at, Instrument::Harp, midi, velocity, pan, 0);
        if self.phrase_left > 0 {
            self.phrase_left -= 1;
            return frames(self.rate, self.rng.range(0.13, 0.24));
        }
        if self.rng.chance(0.22) {
            self.phrase_left = 1 + self.rng.below(2) as u8;
            return frames(self.rate, self.rng.range(0.13, 0.24));
        }
        frames(self.rate, spacing * self.rng.range(0.5, 1.7))
    }

    /// Chord tones high up, one at a time, far apart.
    fn bells(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let len = settings.scale.degrees();
        let tones = [0, 2, 4, len, len + 2, len + 4];
        let degree = self.chord + tones[self.rng.below(tones.len() as u32) as usize];
        let midi = settings.note(Instrument::Bells, degree);
        let velocity = self.rng.range(0.3, 0.7);
        let pan = self.rng.range(-0.7, 0.7);
        self.note(at, Instrument::Bells, midi, velocity, pan, 0);
        frames(self.rate, spacing * self.rng.range(0.4, 2.0))
    }

    /// One long bowed note at a time: the chord's root, third or fifth.
    fn strings(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let degree = self.chord + [0, 2, 4, 0][self.rng.below(4) as usize];
        let midi = settings.note(Instrument::Strings, degree);
        let seconds = self.rng.range(4.0, 9.0);
        let duration = frames(self.rate, seconds);
        let velocity = self.rng.range(0.4, 0.8);
        let pan = self.rng.range(-0.3, 0.3);
        self.note(at, Instrument::Strings, midi, velocity, pan, duration);
        frames(
            self.rate,
            (seconds * self.rng.range(0.8, 1.3)).max(spacing * 0.5),
        )
    }

    /// The chord as three slow notes, a little staggered.
    fn pad(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let seconds = self.rng.range(12.0, 22.0);
        let duration = frames(self.rate, seconds);
        let velocity = self.rng.range(0.5, 0.7);
        for (i, tone) in [0, 2, 4].into_iter().enumerate() {
            let midi = settings.note(Instrument::Pad, self.chord + tone);
            let stagger = frames(self.rate, 0.35 * i as f32);
            let pan = (i as f32 - 1.0) * 0.4;
            self.note(at + stagger, Instrument::Pad, midi, velocity, pan, duration);
        }
        // Until the chord changes, or the notes die away.
        let gap = frames(self.rate, seconds * 0.9).min(self.next_harmony.saturating_sub(at).max(1));
        gap.max(frames(self.rate, spacing * 0.3))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Density, Layer, Presence, Scale, ScaleKind, Style, Weather};
    use super::*;

    /// Every event in `minutes` of music, in order.
    fn play(settings: &AmbienceSettings, seed: u64, minutes: u64) -> Vec<Event> {
        let rate = 48_000;
        let mut composer = Composer::new(rate, seed);
        let mut events = Vec::new();
        let (mut at, end) = (0, rate as u64 * 60 * minutes);
        while at < end {
            composer.poll(settings, at, at + 4096);
            for now in at..at + 4096 {
                while let Some(event) = composer.due(now) {
                    events.push(event);
                }
            }
            at += 4096;
        }
        events
    }

    fn of(events: &[Event], instrument: Instrument) -> Vec<&Event> {
        events
            .iter()
            .filter(|e| e.note.instrument == instrument)
            .collect()
    }

    #[test]
    fn every_note_is_in_the_scale_and_each_instrument_keeps_to_its_register() {
        let mut settings = Style::RainStudy.settings();
        settings.enabled = true;
        settings.scale = Scale {
            root: 5,
            kind: ScaleKind::Mode(Mode::Lydian),
        };
        for layer in &mut settings.layers {
            *layer = Layer::on(0.8, Density::Lively);
        }
        let events = play(&settings, 7, 4);
        assert!(
            events.len() > 150,
            "{} events in four minutes",
            events.len()
        );
        for event in &events {
            let note = event.note;
            assert!(
                settings.scale.contains(note.midi),
                "{} is not in {}",
                note.midi,
                settings.scale.name()
            );
            assert!((0.0..=1.0).contains(&note.velocity));
        }
        let span = |instrument: Instrument| {
            let notes = of(&events, instrument);
            let low = notes.iter().map(|e| e.note.midi).min().unwrap();
            let high = notes.iter().map(|e| e.note.midi).max().unwrap();
            (low, high)
        };
        assert!(span(Instrument::Bass).1 <= 60, "bass stays low");
        assert!(span(Instrument::Harp).0 >= 60, "harp stays high");
        assert!(span(Instrument::Bells).0 >= 72, "bells higher still");
        assert_eq!(
            span(Instrument::Drone),
            (29, 29),
            "the drone holds the root, F1"
        );
        assert!(events.windows(2).all(|pair| pair[0].at <= pair[1].at));
        assert!(
            of(&events, Instrument::Strings)
                .iter()
                .all(|e| e.note.duration > 0)
        );
    }

    use super::super::Mode;

    #[test]
    fn presence_and_density_change_how_much_plays() {
        let count = |presence, density| {
            let mut settings = Style::RainStudy.settings();
            settings.enabled = true;
            settings.presence = presence;
            for layer in &mut settings.layers {
                layer.density = density;
            }
            play(&settings, 3, 3).len()
        };
        let background = count(Presence::Background, Density::Sparse);
        let ambient = count(Presence::Ambient, Density::Sparse);
        let lively = count(Presence::Ambient, Density::Lively);
        let declarative = count(Presence::Declarative, Density::Lively);
        assert!(background < ambient && ambient < lively && lively < declarative);
        assert!(background * 3 < declarative);
    }

    #[test]
    fn declarative_presence_keeps_a_pulse_and_walks_its_cadence() {
        let mut settings = Style::Affirmation.settings();
        settings.enabled = true;
        settings.form = Form::Endless;
        settings.tempo = 60;
        let events = play(&settings, 5, 2);
        let beat = 48_000u64;
        for event in &events {
            if event.note.instrument == Instrument::Pad {
                continue; // staggered on purpose
            }
            let grid = if event.note.instrument == Instrument::Harp {
                beat / 2
            } else {
                beat
            };
            assert_eq!(
                event.at % grid,
                0,
                "{:?} is off the pulse",
                event.note.instrument
            );
        }
        // The bass follows the cadence: after the first chord it visits
        // the fourth and fifth degrees of the scale, not only the root.
        let bass: Vec<u8> = of(&events, Instrument::Bass)
            .iter()
            .map(|e| e.note.midi % 12)
            .collect();
        let root = settings.scale.root;
        assert!(bass.contains(&((root + 5) % 12)), "the subdominant, F");
        assert!(bass.contains(&((root + 7) % 12)), "the dominant, G");
    }

    #[test]
    fn a_piece_opens_with_the_pad_closes_on_the_root_and_pauses() {
        let mut settings = Style::Affirmation.settings();
        settings.enabled = true;
        settings.weather = Weather::None;
        settings.form = Form::Piece {
            minutes: 1.0,
            pause_seconds: 10.0,
        };
        let rate = 48_000u64;
        let events = play(&settings, 9, 2);
        let intro = rate * 60 / 100 * 8;
        let early: Vec<Instrument> = events
            .iter()
            .filter(|e| e.at < intro)
            .map(|e| e.note.instrument)
            .collect();
        assert!(!early.is_empty());
        assert!(
            early
                .iter()
                .all(|i| matches!(i, Instrument::Pad | Instrument::Drone)),
            "{early:?}"
        );
        // The last bass note of the piece is the root.
        let piece_end = rate * 60;
        let last_bass = of(&events, Instrument::Bass)
            .into_iter()
            .rfind(|e| e.at < piece_end)
            .unwrap();
        assert_eq!(last_bass.note.midi % 12, settings.scale.root);
        // Nothing during the pause; then the next piece.
        let pause = piece_end..piece_end + rate * 10;
        assert!(!events.iter().any(|e| pause.contains(&e.at)));
        assert!(events.iter().any(|e| e.at > pause.end + rate));
    }

    #[test]
    fn the_queue_drops_extras_rather_than_growing() {
        let mut composer = Composer::new(48_000, 1);
        for i in 0..(QUEUE as u64 + 10) {
            composer.push(Event {
                at: i,
                note: Note {
                    instrument: Instrument::Bells,
                    midi: 72,
                    velocity: 0.5,
                    pan: 0.0,
                    duration: 0,
                },
            });
        }
        assert_eq!(composer.queued, QUEUE);
        let mut taken = 0;
        while composer.due(u64::MAX).is_some() {
            taken += 1;
        }
        assert_eq!(taken, QUEUE);
    }
}

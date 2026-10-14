//! Decides what the instruments play and when, without making a sound.
//!
//! A slow harmony (a chord degree of the scale) underlies everything: the
//! bass and strings lean on it, the harp, piano, violin and bells mostly
//! touch its tones, the pad voices it, the frame drum keeps to the root.
//! Declarative presence puts the notes on a pulse and walks a cadence; the
//! other presences drift. Pieces have an intro, a close that thins out and
//! lands on the root, and a pause.
//!
//! The character shapes the lines. Variety: the harp, piano and violin
//! each remember a short motif and come back to it, and its rhythm, more
//! often as variety falls; as it rises there are more chords, changing
//! sooner, and freer gaps. Concentration: the lines keep nearer the chord
//! and the home note, in smaller steps and a narrower range. Chaos: notes
//! a semitone or a tritone off, clashing doubles and clusters, chords from
//! anywhere in the key, looser timing. Chaos at 0 and concentration in the
//! middle draw nothing from the random source; variety in the middle
//! still brings a motif back now and then.

use super::{AmbienceSettings, Form, Instrument, Rng};

/// The note the drums play for a shaker. Any lower note is a frame drum
/// tuned to it.
pub const SHAKER: u8 = 120;

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

/// The most notes a motif holds.
const MOTIF: usize = 8;

/// How many notes a motif has: three when variety is lowest, eight at
/// the most.
fn motif_length(variety: f32) -> usize {
    3 + (variety * 5.0).round() as usize
}

/// How often a line plays its motif through again rather than new notes.
fn repeat_chance(variety: f32) -> f32 {
    (1.0 - variety).powi(2) * 0.85
}

/// How far gaps spread around their middle: 1 at the middle variety.
fn spread(variety: f32) -> f32 {
    0.3 + 1.4 * variety
}

/// The degrees a line keeps between, above its register: two octaves at
/// the middle concentration, one when concentrated, three when not.
fn span(concentration: f32, len: i32) -> (i32, i32) {
    let low = (len as f32 * (concentration - 0.5).max(0.0)).round() as i32;
    let high = low + (len as f32 * (3.0 - 2.0 * concentration)).round() as i32;
    (low, high)
}

/// A melodic instrument's memory: its last degree, and a short motif
/// (degrees above the chord, and the gaps after them) it comes back to.
#[derive(Clone, Copy, Debug)]
struct Line {
    degree: i32,
    motif: [(i32, f32); MOTIF],
    /// Notes written into the motif, up to [`MOTIF`].
    written: usize,
    /// The motif's next note and its last; whether this time through
    /// the motif repeats, and whether the last note was the motif's.
    next: usize,
    slot: usize,
    repeating: bool,
    again: bool,
}

impl Line {
    fn new(degree: i32) -> Self {
        Self {
            degree,
            motif: [(0, 1.0); MOTIF],
            written: 0,
            next: 0,
            slot: 0,
            repeating: false,
            again: false,
        }
    }
}

/// How a melodic instrument picks its new notes, at the middle
/// concentration.
struct Shape<'a> {
    /// How often a chord tone rather than a step.
    chord: f32,
    /// The chord tones it reaches for, as degrees above the chord.
    tones: &'a [i32],
    /// Its widest step, in degrees.
    widest: i32,
}

/// Where a piece is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Playing,
    /// The background only, until this frame.
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
    next: [u64; Instrument::COUNT],
    /// The melodic lines, and how many notes of a harp phrase are left.
    harp: Line,
    phrase_left: u8,
    piano: Line,
    violin: Line,
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
            next: [0; Instrument::COUNT],
            harp: Line::new(4),
            phrase_left: 0,
            piano: Line::new(4),
            violin: Line::new(2),
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
        self.harp = Line::new(4);
        self.phrase_left = 0;
        self.piano = Line::new(4);
        self.violin = Line::new(2);
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

    /// How long the harmony stays put: in free time, longer as variety
    /// falls.
    fn harmony_span(&mut self, settings: &AmbienceSettings) -> u64 {
        if settings.presence.pulsed() {
            self.beat(settings) * 8
        } else {
            let seconds = self.rng.range(18.0, 45.0) * (1.6 - 1.2 * settings.variety);
            frames(self.rate, seconds)
        }
    }

    /// True with `probability`, drawing nothing when it is 0 or less.
    fn maybe(&mut self, probability: f32) -> bool {
        probability > 0.0 && self.rng.chance(probability)
    }

    /// The gain the instruments play at now: 1, or less as a piece closes.
    pub fn music_gain(&self) -> f32 {
        self.music_gain
    }

    /// True once the music has begun: from the first poll after a reset.
    pub fn is_started(&self) -> bool {
        self.started
    }

    /// Where the piece (or the endless music) began, in frames.
    pub fn origin(&self) -> u64 {
        self.origin
    }

    /// The key changed at `now`: what was waiting is dropped, the harmony
    /// goes home in the new key, and the pad and strings voice it soon.
    pub fn modulate(&mut self, settings: &AmbienceSettings, now: u64) {
        if !self.started {
            return;
        }
        self.clear_queue();
        self.chord = 0;
        self.cadence_step = 0;
        self.next_harmony = now + self.harmony_span(settings);
        for (instrument, seconds) in [(Instrument::Pad, 0.6), (Instrument::Strings, 1.5)] {
            let next = &mut self.next[instrument.index()];
            *next = (*next).min(now + frames(self.rate, seconds));
        }
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
                if instrument == Instrument::Drone {
                    // Sounded again when it comes back.
                    self.drone_midi = None;
                }
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
            let mut chord = CADENCE[self.cadence_step];
            // Variety stands in the supertonic, submediant or mediant now
            // and then; chaos any chord.
            if self.maybe((settings.variety - 0.5) * 0.6) {
                chord = [1, 5, 2][self.rng.below(3) as usize];
            }
            if self.maybe(settings.chaos * 0.5) {
                chord = self.rng.below(settings.scale.degrees() as u32) as i32;
            }
            chord
        } else {
            // Never the same chord twice running.
            let mut chord = self.chord;
            while chord == self.chord {
                chord = self.pick_chord(settings);
            }
            chord
        };
        self.next_harmony = now + self.harmony_span(settings);
        // The pad voices the new chord soon.
        let pad = Instrument::Pad.index();
        self.next[pad] = self.next[pad].min(now + frames(self.rate, self.rng.range(0.3, 1.5)));
    }

    /// A chord in free time: mostly home (more so with concentration, less
    /// with variety), the subdominant, submediant and dominant for colour,
    /// the supertonic and mediant as variety rises; with chaos, any.
    fn pick_chord(&mut self, settings: &AmbienceSettings) -> i32 {
        if self.maybe(settings.chaos * 0.5) {
            return self.rng.below(settings.scale.degrees() as u32) as i32;
        }
        let variety = settings.variety;
        let weights = [
            (0, 4.0 * (1.5 - variety) * (0.5 + settings.concentration)),
            (3, 2.0),
            (5, 2.0),
            (1, 2.0 * variety),
            (4, 1.0),
            (2, 2.0 * (2.0 * variety - 1.0).max(0.0)),
        ];
        let total: f32 = weights.iter().map(|(_, weight)| weight).sum();
        let mut roll = self.rng.unit() * total;
        for (chord, weight) in weights {
            if roll < weight {
                return chord;
            }
            roll -= weight;
        }
        0
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
            let mut next_gap = match instrument {
                Instrument::Bass => self.bass(settings, at, spacing),
                Instrument::Harp => self.harp(settings, at, spacing),
                Instrument::Bells => self.bells(settings, at, spacing),
                Instrument::Strings => self.strings(settings, at, spacing),
                Instrument::Pad => self.pad(settings, at, spacing),
                Instrument::Piano => self.piano(settings, at, spacing),
                Instrument::Violin => self.violin(settings, at, spacing),
                Instrument::Drums => self.drums(settings, at, spacing),
                Instrument::Drone => 0,
            };
            // Chaos loosens free time: a note early or late. The pad keeps
            // to the harmony.
            let chaos = settings.chaos;
            if instrument != Instrument::Pad && !settings.presence.pulsed() && self.maybe(chaos) {
                let stretch = self.rng.range(1.0 - 0.6 * chaos, 1.0 + 0.6 * chaos);
                next_gap = (next_gap as f32 * stretch) as u64;
            }
            self.next[index] = at + next_gap.max(1);
        }
    }

    /// The next beat at or after `at`; the harp, violin and drums may
    /// fall on the half beat.
    fn on_pulse(&self, settings: &AmbienceSettings, at: u64, instrument: Instrument) -> u64 {
        let grid = Self::grid(self.beat(settings), instrument);
        let since = at.saturating_sub(self.origin);
        let steps = since.div_ceil(grid);
        self.origin + steps * grid
    }

    /// The finest step an instrument keeps to on the pulse.
    fn grid(beat: u64, instrument: Instrument) -> u64 {
        match instrument {
            Instrument::Harp | Instrument::Violin | Instrument::Drums => (beat / 2).max(1),
            _ => beat.max(1),
        }
    }

    /// How long after a note its answer comes: half a beat on the pulse,
    /// else a moment.
    fn answer(&mut self, settings: &AmbienceSettings, low: f32, high: f32) -> u64 {
        if settings.presence.pulsed() {
            (self.beat(settings) / 2).max(1)
        } else {
            frames(self.rate, self.rng.range(low, high))
        }
    }

    /// A gap factor somewhere in `low..high`: nearer the middle when
    /// variety is low, further out when high.
    fn gap_factor(&mut self, settings: &AmbienceSettings, low: f32, high: f32) -> f32 {
        let middle = (low + high) / 2.0;
        let half = (high - low) / 2.0 * spread(settings.variety);
        self.rng.range((middle - half).max(0.1), middle + half)
    }

    /// A line's next degree. Each time through, the motif may repeat
    /// (more often as variety falls), a note of it changing now and then
    /// (more often as it rises). Otherwise a new note: the home note or a
    /// chord tone, more often as concentration rises, else a step from the
    /// last, wider as it falls. Kept to the line's span.
    fn melody(&mut self, settings: &AmbienceSettings, line: &mut Line, shape: &Shape) -> i32 {
        let len = settings.scale.degrees();
        let concentration = settings.concentration;
        let length = motif_length(settings.variety);
        let slot = line.next % length;
        line.slot = slot;
        line.next = (slot + 1) % length;
        if slot == 0 {
            line.repeating = line.written >= length && self.maybe(repeat_chance(settings.variety));
        }
        line.again = line.repeating && !self.maybe(settings.variety * 0.5);
        let degree = if line.again {
            self.chord + line.motif[slot].0
        } else if self.maybe(concentration - 0.5) {
            // The home note, in the octave nearest the last note.
            (line.degree as f32 / len as f32).round() as i32 * len
        } else if self
            .rng
            .chance((shape.chord + (concentration - 0.5) * 0.8).clamp(0.1, 0.97))
        {
            self.chord + shape.tones[self.rng.below(shape.tones.len() as u32) as usize]
        } else {
            let widest = (shape.widest as f32 * 2.0 * (1.0 - concentration))
                .round()
                .max(1.0) as i32;
            let mut step = 0;
            while step == 0 {
                step = self.rng.below(2 * widest as u32 + 1) as i32 - widest;
            }
            line.degree + step
        };
        let (low, high) = span(concentration, len);
        line.degree = degree.clamp(low, high);
        if !line.again {
            line.motif[slot].0 = line.degree - self.chord;
            line.written = (line.written + 1).min(MOTIF);
        }
        line.degree
    }

    /// The gap after a line's note, `spacing` times a factor in
    /// `low..high`: the motif's own when the note was the motif again, so
    /// a motif keeps its rhythm; else a new one, remembered.
    fn line_gap(
        &mut self,
        settings: &AmbienceSettings,
        line: &mut Line,
        spacing: f32,
        low: f32,
        high: f32,
    ) -> u64 {
        let factor = if line.again {
            line.motif[line.slot].1
        } else {
            let factor = self.gap_factor(settings, low, high);
            line.motif[line.slot].1 = factor;
            factor
        };
        frames(self.rate, spacing * factor)
    }

    /// Under chaos, now and then a note a semitone off, out of the key
    /// when it can be; at high chaos, sometimes a tritone away instead.
    fn unsettle(&mut self, settings: &AmbienceSettings, midi: u8, chance: f32) -> u8 {
        let chaos = settings.chaos;
        if !self.maybe(chaos * chance) {
            return midi;
        }
        if self.maybe(chaos - 0.6) {
            return if midi >= 66 { midi - 6 } else { midi + 6 };
        }
        let (up, down) = (midi.saturating_add(1).min(127), midi.saturating_sub(1));
        let (first, second) = if self.rng.chance(0.5) {
            (up, down)
        } else {
            (down, up)
        };
        if settings.scale.contains(first) && !settings.scale.contains(second) {
            second
        } else {
            first
        }
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

    /// The root of the chord (more often with concentration), its fifth,
    /// the third now and then, and the root an octave down for weight,
    /// rarely; with chaos, a tritone above the root. Returns the gap to
    /// the next.
    fn bass(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let root = (0.5 + (settings.concentration - 0.5) * 0.8).clamp(0.1, 0.9);
        let rest = 1.0 - root;
        let roll = self.rng.unit();
        let degree = self.chord
            + if roll < root {
                0
            } else if roll < root + rest * 0.5 {
                4
            } else if roll < root + rest * 0.8 {
                2
            } else {
                -settings.scale.degrees()
            };
        let mut midi = settings.note(Instrument::Bass, degree);
        if self.maybe(settings.chaos * 0.3) {
            midi = settings.note(Instrument::Bass, self.chord) + 6;
        }
        let velocity = self.rng.range(0.55, 0.95);
        let pan = self.rng.range(-0.15, 0.15);
        self.note(at, Instrument::Bass, midi, velocity, pan, 0);
        frames(self.rate, spacing * self.gap_factor(settings, 0.55, 1.6))
    }

    /// Mostly the chord's tones, else a small step; now and then a short
    /// rising phrase, more often with variety.
    fn harp(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let len = settings.scale.degrees();
        let mut line = self.harp;
        if self.phrase_left > 0 {
            let (low, high) = span(settings.concentration, len);
            line.degree = (line.degree + 1 + self.rng.below(2) as i32).clamp(low, high);
        } else {
            let shape = Shape {
                chord: 0.6,
                tones: &[0, 2, 4, len, len + 2],
                widest: 3,
            };
            self.melody(settings, &mut line, &shape);
        }
        let midi = settings.note(Instrument::Harp, line.degree);
        let midi = self.unsettle(settings, midi, 0.4);
        let velocity = self.rng.range(0.35, 0.8);
        let pan = self.rng.range(-0.65, 0.65);
        self.note(at, Instrument::Harp, midi, velocity, pan, 0);
        let gap = if self.phrase_left > 0 {
            self.phrase_left -= 1;
            frames(self.rate, self.rng.range(0.13, 0.24))
        } else if self.rng.chance(0.22 * spread(settings.variety)) {
            self.phrase_left = 1 + self.rng.below(2) as u8;
            frames(self.rate, self.rng.range(0.13, 0.24))
        } else {
            self.line_gap(settings, &mut line, spacing, 0.5, 1.7)
        };
        self.harp = line;
        gap
    }

    /// Chord tones high up, one at a time, far apart; the home note
    /// more often with concentration.
    fn bells(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let len = settings.scale.degrees();
        let degree = if self.maybe(settings.concentration - 0.5) {
            len
        } else {
            let tones = [0, 2, 4, len, len + 2, len + 4];
            self.chord + tones[self.rng.below(tones.len() as u32) as usize]
        };
        let midi = settings.note(Instrument::Bells, degree);
        let midi = self.unsettle(settings, midi, 0.4);
        let velocity = self.rng.range(0.3, 0.7);
        let pan = self.rng.range(-0.7, 0.7);
        self.note(at, Instrument::Bells, midi, velocity, pan, 0);
        frames(self.rate, spacing * self.gap_factor(settings, 0.4, 2.0))
    }

    /// One long bowed note at a time: the chord's root, third or fifth;
    /// the home note more often with concentration.
    fn strings(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let degree = if self.maybe(settings.concentration - 0.5) {
            0
        } else {
            self.chord + [0, 2, 4, 0][self.rng.below(4) as usize]
        };
        let midi = settings.note(Instrument::Strings, degree);
        let midi = self.unsettle(settings, midi, 0.25);
        let seconds = self.rng.range(4.0, 9.0);
        let duration = frames(self.rate, seconds);
        let velocity = self.rng.range(0.4, 0.8);
        let pan = self.rng.range(-0.3, 0.3);
        self.note(at, Instrument::Strings, midi, velocity, pan, duration);
        let factor = self.gap_factor(settings, 0.8, 1.3);
        frames(self.rate, (seconds * factor).max(spacing * 0.5))
    }

    /// A struck note, mostly a chord tone, else a step from the last; a
    /// third or a sixth below it now and then, softer, or with chaos a
    /// second or a tritone.
    fn piano(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let len = settings.scale.degrees();
        let mut line = self.piano;
        let shape = Shape {
            chord: 0.55,
            tones: &[0, 2, 4, len],
            widest: 2,
        };
        let degree = self.melody(settings, &mut line, &shape);
        let midi = settings.note(Instrument::Piano, degree);
        let midi = self.unsettle(settings, midi, 0.35);
        let velocity = self.rng.range(0.3, 0.65);
        let pan = self.rng.range(-0.35, 0.35);
        self.note(at, Instrument::Piano, midi, velocity, pan, 0);
        if self.rng.chance(0.3) {
            let low = if self.maybe(settings.chaos * 0.7) {
                midi.saturating_sub([1, 2, 6][self.rng.below(3) as usize])
            } else {
                let below = [2, 5][self.rng.below(2) as usize];
                settings.note(Instrument::Piano, degree - below)
            };
            self.note(at, Instrument::Piano, low, velocity * 0.7, pan - 0.1, 0);
        }
        let gap = self.line_gap(settings, &mut line, spacing, 0.5, 1.7);
        self.piano = line;
        gap
    }

    /// A short pluck high up on a chord tone or a step away; a quarter of
    /// the time a neighbour answers it.
    fn violin(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let len = settings.scale.degrees();
        let mut line = self.violin;
        let shape = Shape {
            chord: 0.6,
            tones: &[0, 2, 4, len, len + 2],
            widest: 2,
        };
        let degree = self.melody(settings, &mut line, &shape);
        let midi = settings.note(Instrument::Violin, degree);
        let midi = self.unsettle(settings, midi, 0.45);
        let velocity = self.rng.range(0.35, 0.7);
        let pan = self.rng.range(-0.5, 0.5);
        self.note(at, Instrument::Violin, midi, velocity, pan, 0);
        if self.rng.chance(0.25) {
            let step = if self.rng.chance(0.5) { 1 } else { -1 };
            let answer = settings.note(Instrument::Violin, (degree + step).max(0));
            let answer = self.unsettle(settings, answer, 0.45);
            let later = at + self.answer(settings, 0.25, 0.4);
            self.note(later, Instrument::Violin, answer, velocity * 0.8, pan, 0);
        }
        let gap = self.line_gap(settings, &mut line, spacing, 0.5, 1.8);
        self.violin = line;
        gap
    }

    /// A soft frame drum on the root, or a shaker; now and then two
    /// together, the second half a beat later.
    fn drums(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let pan = self.rng.range(-0.3, 0.3);
        if self.rng.chance(0.55) {
            let midi = settings.note(Instrument::Drums, 0);
            let velocity = self.rng.range(0.4, 0.75);
            self.note(at, Instrument::Drums, midi, velocity, pan, 0);
        } else {
            let velocity = self.rng.range(0.25, 0.5);
            self.note(at, Instrument::Drums, SHAKER, velocity, pan * 2.0, 0);
        }
        if self.rng.chance(0.3) {
            let later = at + self.answer(settings, 0.3, 0.5);
            let velocity = self.rng.range(0.2, 0.4);
            self.note(later, Instrument::Drums, SHAKER, velocity, -pan, 0);
        }
        frames(self.rate, spacing * self.gap_factor(settings, 0.7, 1.8))
    }

    /// The chord as three slow notes, a little staggered; with chaos, a
    /// note bent off now and then and a fourth that rubs against them: the
    /// second, or at high chaos a semitone above the root.
    fn pad(&mut self, settings: &AmbienceSettings, at: u64, spacing: f32) -> u64 {
        let seconds = self.rng.range(12.0, 22.0);
        let duration = frames(self.rate, seconds);
        let velocity = self.rng.range(0.5, 0.7);
        for (i, tone) in [0, 2, 4].into_iter().enumerate() {
            let midi = settings.note(Instrument::Pad, self.chord + tone);
            let midi = self.unsettle(settings, midi, 0.2);
            let stagger = frames(self.rate, 0.35 * i as f32);
            let pan = (i as f32 - 1.0) * 0.4;
            self.note(at + stagger, Instrument::Pad, midi, velocity, pan, duration);
        }
        if self.maybe(settings.chaos * 0.8) {
            let midi = if self.maybe(settings.chaos - 0.5) {
                settings.note(Instrument::Pad, self.chord) + 1
            } else {
                settings.note(Instrument::Pad, self.chord + 1)
            };
            let stagger = frames(self.rate, 1.05);
            self.note(
                at + stagger,
                Instrument::Pad,
                midi,
                velocity * 0.8,
                0.0,
                duration,
            );
        }
        // Until the chord changes, or the notes die away.
        let gap = frames(self.rate, seconds * 0.9).min(self.next_harmony.saturating_sub(at).max(1));
        gap.max(frames(self.rate, spacing * 0.3))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Background, Density, Layer, Presence, Scale, ScaleKind, Style};
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
            if note.instrument == Instrument::Drums && note.midi == SHAKER {
                continue;
            }
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
        assert!(span(Instrument::Violin).0 >= 60, "the violin plays high");
        let piano = span(Instrument::Piano);
        assert!(piano.0 >= 40 && piano.1 <= 90, "the piano in the middle");
        assert!(
            of(&events, Instrument::Drums)
                .iter()
                .all(|e| e.note.midi == SHAKER || e.note.midi % 12 == settings.scale.root),
            "the frame drum keeps to the root"
        );
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
        for instrument in [Instrument::Violin, Instrument::Drums, Instrument::Piano] {
            *settings.layer_mut(instrument) = Layer::on(0.6, Density::Lively);
        }
        let events = play(&settings, 5, 2);
        for instrument in [Instrument::Violin, Instrument::Drums, Instrument::Piano] {
            assert!(!of(&events, instrument).is_empty(), "{instrument:?} plays");
        }
        let beat = 48_000u64;
        for event in &events {
            if event.note.instrument == Instrument::Pad {
                continue; // staggered on purpose
            }
            let grid = Composer::grid(beat, event.note.instrument);
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
        settings.background = Background::None;
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
    fn drums_are_very_sparse_and_drift_off_the_beat_without_a_pulse() {
        let mut settings = Style::Campfire.settings();
        settings.enabled = true;
        let events = play(&settings, 4, 3);
        let drums = of(&events, Instrument::Drums);
        // Sparse, at the Ambient presence: a hit every ten seconds or more.
        assert!(
            (3..=40).contains(&drums.len()),
            "{} hits in three minutes",
            drums.len()
        );
        let beat = 48_000u64;
        assert!(
            drums.iter().any(|e| e.at % (beat / 2) != 0),
            "free time, not on a grid"
        );
    }

    /// The piano alone, lively, at a character; its main notes (not the
    /// doubles under them) in order.
    fn piano_line(variety: f32, concentration: f32, chaos: f32, seed: u64) -> Vec<Event> {
        let mut settings = Style::SeasidePiano.settings();
        settings.enabled = true;
        for instrument in Instrument::ALL {
            settings.layer_mut(instrument).enabled = false;
        }
        *settings.layer_mut(Instrument::Piano) = Layer::on(0.7, Density::Lively);
        settings.variety = variety;
        settings.concentration = concentration;
        settings.chaos = chaos;
        let mut events: Vec<Event> = Vec::new();
        for event in play(&settings, seed, 6) {
            if events.last().is_none_or(|last| last.at != event.at) {
                events.push(event);
            }
        }
        events
    }

    /// The share of four-note figures, as steps through the scale (so a
    /// motif moved to another chord is still the same figure), that
    /// already came before.
    fn repeated_figures(settings: &AmbienceSettings, events: &[Event]) -> f32 {
        let intervals = settings.scale.kind.intervals();
        let degree = |midi: u8| {
            let above = i32::from(midi) - i32::from(settings.scale.root);
            let class = above.rem_euclid(12) as u8;
            let index = intervals.iter().position(|i| *i == class).unwrap() as i32;
            above.div_euclid(12) * intervals.len() as i32 + index
        };
        let figures: Vec<[i32; 3]> = events
            .windows(4)
            .map(|w| {
                let d: Vec<i32> = w.iter().map(|e| degree(e.note.midi)).collect();
                [d[1] - d[0], d[2] - d[1], d[3] - d[2]]
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        let repeats = figures.iter().filter(|f| !seen.insert(**f)).count();
        repeats as f32 / figures.len().max(1) as f32
    }

    #[test]
    fn low_variety_comes_back_to_a_motif_and_high_variety_keeps_changing() {
        let settings = Style::SeasidePiano.settings();
        let measure = |variety| {
            let line = piano_line(variety, 0.5, 0.0, 3);
            // How unevenly the notes come: the spread of the gaps over
            // their mean.
            let gaps: Vec<f32> = line
                .windows(2)
                .map(|pair| (pair[1].at - pair[0].at) as f32)
                .collect();
            let mean = gaps.iter().sum::<f32>() / gaps.len() as f32;
            let deviation = gaps.iter().map(|gap| (gap - mean).powi(2)).sum::<f32>();
            let uneven = (deviation / gaps.len() as f32).sqrt() / mean;
            (repeated_figures(&settings, &line), uneven)
        };
        let (still, middle, restless) = (measure(0.0), measure(0.5), measure(1.0));
        assert!(
            still.0 > 0.3 && still.0 > restless.0 * 3.0,
            "figures come back: {still:?} {restless:?}"
        );
        assert!(
            still.1 < 0.15 && still.1 < middle.1 && middle.1 < restless.1 && restless.1 > 0.4,
            "and keep their rhythm: {still:?} {middle:?} {restless:?}"
        );
    }

    #[test]
    fn concentration_gathers_the_notes_around_home() {
        let root = Style::SeasidePiano.settings().scale.root;
        let measure = |concentration| {
            let line = piano_line(0.5, concentration, 0.0, 5);
            let low = line.iter().map(|e| e.note.midi).min().unwrap();
            let high = line.iter().map(|e| e.note.midi).max().unwrap();
            let home = line.iter().filter(|e| e.note.midi % 12 == root).count();
            (high - low, home as f32 / line.len() as f32)
        };
        let (wide, middle, close) = (measure(0.0), measure(0.5), measure(1.0));
        assert!(
            wide.0 >= 30 && middle.0 < wide.0 && close.0 <= 12,
            "the range narrows: {wide:?} {middle:?} {close:?}"
        );
        assert!(
            close.1 > 0.35 && close.1 > wide.1 + 0.1,
            "and keeps to the home note: {wide:?} {close:?}"
        );
    }

    #[test]
    fn chaos_brings_notes_from_outside_the_key_and_clusters() {
        let measure = |chaos| {
            let mut settings = Style::RainStudy.settings();
            settings.enabled = true;
            for layer in &mut settings.layers {
                *layer = Layer::on(0.8, Density::Lively);
            }
            settings.chaos = chaos;
            let events = play(&settings, 7, 4);
            let pitched: Vec<&Event> = events
                .iter()
                .filter(|e| !(e.note.instrument == Instrument::Drums && e.note.midi == SHAKER))
                .collect();
            let outside = pitched
                .iter()
                .filter(|e| !settings.scale.contains(e.note.midi))
                .count();
            // The pad's chords: a chord's notes are all held as long. A
            // cluster has two a semitone or a tone apart.
            let mut chords: Vec<(u64, Vec<u8>)> = Vec::new();
            for pad in of(&events, Instrument::Pad) {
                let (duration, midi) = (pad.note.duration, pad.note.midi);
                match chords.iter_mut().find(|(held, _)| *held == duration) {
                    Some((_, chord)) => chord.push(midi),
                    None => chords.push((duration, vec![midi])),
                }
            }
            let rubs = |(_, chord): &&(u64, Vec<u8>)| {
                chord.iter().any(|a| {
                    chord.iter().any(|b| {
                        let class = a.abs_diff(*b) % 12;
                        matches!(class, 1 | 2 | 10 | 11)
                    })
                })
            };
            let clusters = chords.iter().filter(rubs).count();
            (
                outside as f32 / pitched.len() as f32,
                clusters as f32 / chords.len() as f32,
            )
        };
        let (calm, some, wild) = (measure(0.0), measure(0.25), measure(1.0));
        assert_eq!(calm, (0.0, 0.0), "in the key, and no clusters");
        assert!(
            some.0 > 0.02 && wild.0 > some.0 && wild.0 > 0.2,
            "notes from outside the key: {some:?} {wild:?}"
        );
        assert!(
            wild.1 > 0.4 && wild.1 > some.1,
            "the pad rubs: {some:?} {wild:?}"
        );
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

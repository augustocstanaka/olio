//! How the music changes over time ([`Journey`]): steady; building up,
//! the instruments entering one at a time while the mood opens; or
//! evolving, building up and then moving on every few minutes: to a
//! related key, a nearby mood, a related background, an instrument resting
//! or coming back.
//!
//! [`Course`] works out, from the settings and the time, what plays now.
//! It allocates nothing, for the audio thread, and the same seed takes the
//! same course, so a track renders the same every time.

use serde::{Deserialize, Serialize};

use super::{AmbienceSettings, Background, Form, Instrument, Mode, Rng, Scale, ScaleKind};

/// How the music changes over time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Journey {
    /// The same all the way through.
    #[default]
    Steady,
    /// One instrument first, the others joining one at a time.
    Build,
    /// Builds up, then moves on every few minutes.
    Evolve,
}

impl Journey {
    pub const ALL: [Self; 3] = [Self::Steady, Self::Build, Self::Evolve];

    pub fn name(self) -> &'static str {
        match self {
            Self::Steady => "Steady",
            Self::Build => "Builds up",
            Self::Evolve => "Evolves",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Steady => "the same all the way through",
            Self::Build => {
                "one instrument first, the others joining one at a time as the mood opens up"
            }
            Self::Evolve => {
                "builds up, then every few minutes moves on: to a related key, a nearby mood, \
                 a related background, an instrument resting or coming back"
            }
        }
    }
}

/// The order instruments enter in: the ground first, then the low and
/// long, the melodic, the bright, the drums.
const ENTRY_ORDER: [Instrument; Instrument::COUNT] = [
    Instrument::Drone,
    Instrument::Pad,
    Instrument::Bass,
    Instrument::Strings,
    Instrument::Harp,
    Instrument::Piano,
    Instrument::Violin,
    Instrument::Bells,
    Instrument::Drums,
];

/// The most seconds between two entries; in pieces, the build takes at
/// most this share of a piece.
const ENTRY_SECONDS: f32 = 24.0;
const PIECE_BUILD_SHARE: f32 = 0.4;
/// An instrument that enters swells to its level over this long, from
/// this share of it.
const SWELL_SECONDS: f32 = 12.0;
const SWELL_FROM: f32 = 0.35;
/// While building, the mood starts this still: little variety, close to
/// home, no chaos.
const STILL_VARIETY: f32 = 0.2;
const STILL_CONCENTRATION: f32 = 0.8;
/// How far the mood drifts from the settings' own, a step at a time.
const MOOD_REACH: f32 = 0.3;
const MOOD_STEP: f32 = 0.15;
const CHAOS_REACH: f32 = 0.2;
const CHAOS_STEP: f32 = 0.1;

/// The modes from brightest to darkest: a neighbour differs by one note.
const BRIGHTNESS: [Mode; 7] = [
    Mode::Lydian,
    Mode::Ionian,
    Mode::Mixolydian,
    Mode::Dorian,
    Mode::Aeolian,
    Mode::Phrygian,
    Mode::Locrian,
];

/// Backgrounds one can cross to from another.
fn related(background: Background) -> &'static [Background] {
    match background {
        Background::None => &[],
        Background::Rain => &[Background::Thunder, Background::Forest, Background::Wind],
        Background::Thunder => &[Background::Rain],
        Background::Wind => &[Background::Winter, Background::Desert, Background::Waves],
        Background::Winter => &[Background::Wind],
        Background::Waves => &[Background::Wind, Background::Rain],
        Background::Forest => &[Background::Rain, Background::Wind],
        Background::Desert => &[Background::Wind],
        Background::Fire => &[Background::Wind, Background::Forest],
    }
}

/// Where an evolving course has got to, against the settings.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Drift {
    /// Semitones the root has moved up, 0 to 11.
    root: u8,
    /// A neighbouring mode in place of the settings' own.
    mode: Option<Mode>,
    background: Option<Background>,
    variety: f32,
    concentration: f32,
    chaos: f32,
    resting: Option<Instrument>,
}

impl Drift {
    fn apply(&self, base: &AmbienceSettings) -> AmbienceSettings {
        let mut out = *base;
        out.scale.root = (base.scale.root + self.root) % 12;
        if let Some(mode) = self.mode {
            out.scale.kind = ScaleKind::Mode(mode);
        }
        if let Some(background) = self.background {
            out.background = background;
        }
        out.variety = (base.variety + self.variety).clamp(0.0, 1.0);
        out.concentration = (base.concentration + self.concentration).clamp(0.0, 1.0);
        out.chaos = (base.chaos + self.chaos).clamp(0.0, 1.0);
        if let Some(instrument) = self.resting {
            out.layers[instrument].enabled = false;
        }
        out
    }
}

/// What a building or evolving ambience plays now, for the app to show.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Now {
    pub scale: Scale,
    pub background: Background,
    /// The instrument resting for a while, if one is.
    pub resting: Option<Instrument>,
    /// Still building up.
    pub building: bool,
}

impl Now {
    /// As bits for an atomic: never 0, which means nothing to show.
    pub(crate) fn pack(self) -> u64 {
        let kind = ScaleKind::all()
            .iter()
            .position(|kind| *kind == self.scale.kind)
            .unwrap_or(0) as u64;
        let background = Background::ALL
            .iter()
            .position(|background| *background == self.background)
            .unwrap_or(0) as u64;
        let resting = self
            .resting
            .map_or(0, |instrument| instrument.index() as u64 + 1);
        1 | u64::from(self.scale.root % 12) << 1
            | kind << 5
            | background << 9
            | resting << 13
            | u64::from(self.building) << 17
    }

    pub(crate) fn unpack(bits: u64) -> Option<Self> {
        if bits & 1 == 0 {
            return None;
        }
        let field = |shift: u32| ((bits >> shift) & 0xF) as usize;
        Some(Self {
            scale: Scale {
                root: field(1) as u8,
                kind: *ScaleKind::all().get(field(5))?,
            },
            background: *Background::ALL.get(field(9))?,
            resting: field(13)
                .checked_sub(1)
                .and_then(|index| Instrument::ALL.get(index).copied()),
            building: bits >> 17 & 1 == 1,
        })
    }
}

/// The course the music takes from its start: the build, and what has
/// drifted since.
pub(super) struct Course {
    rate: u32,
    rng: Rng,
    drift: Drift,
    /// Where the build began: the start, or the piece's.
    origin: u64,
    /// When the next change is due, in endless play; `None` until the
    /// build's length is known.
    next_change: Option<u64>,
    /// The piece the course is in, to move on between pieces.
    piece: Option<u64>,
    building: bool,
}

impl Course {
    pub(super) fn new(rate: u32, seed: u64) -> Self {
        Self {
            rate,
            rng: Rng::new(seed),
            drift: Drift::default(),
            origin: 0,
            next_change: None,
            piece: None,
            building: false,
        }
    }

    /// Starts again at `now`: the build from the beginning, nothing
    /// drifted.
    pub(super) fn reset(&mut self, seed: u64, now: u64) {
        self.rng = Rng::new(seed ^ 0x6A09_E667_F3BC_C908);
        self.drift = Drift::default();
        self.origin = now;
        self.next_change = None;
        self.piece = None;
        self.building = false;
    }

    fn frames(&self, seconds: f32) -> u64 {
        (seconds.max(0.0) * self.rate as f32) as u64
    }

    /// What plays at `now` for `base`, in the piece that began at `piece`
    /// (ignored when endless).
    pub(super) fn settings(
        &mut self,
        base: &AmbienceSettings,
        now: u64,
        piece: u64,
    ) -> AmbienceSettings {
        if base.journey == Journey::Steady {
            self.building = false;
            return *base;
        }
        let pieces = matches!(base.form, Form::Piece { .. });
        if pieces && self.piece != Some(piece) {
            // Each piece builds up again; an evolving one moves on first.
            if self.piece.is_some() && base.journey == Journey::Evolve {
                self.change(base);
            }
            self.piece = Some(piece);
            self.origin = piece;
        }
        if base.journey == Journey::Evolve && !pieces {
            let due = match self.next_change {
                Some(due) => due,
                None => {
                    let built = self.origin + self.frames(self.build_seconds(base));
                    let due = built + self.interval(base);
                    self.next_change = Some(due);
                    due
                }
            };
            if now >= due {
                self.change(base);
                self.next_change = Some(now + self.interval(base));
            }
        }
        let mut out = self.drift.apply(base);
        self.build(&mut out, base, now);
        out
    }

    /// The instrument resting and whether it is still building, for
    /// [`Now`].
    pub(super) fn now(&self, heard: &AmbienceSettings) -> Now {
        Now {
            scale: heard.scale,
            background: heard.background,
            resting: self.drift.resting,
            building: self.building,
        }
    }

    /// Seconds between two entries for these settings.
    fn entry_seconds(base: &AmbienceSettings, entering: usize) -> f32 {
        match base.form {
            Form::Endless => ENTRY_SECONDS,
            Form::Piece { minutes, .. } => {
                let share = minutes * 60.0 * PIECE_BUILD_SHARE;
                (share / entering.saturating_sub(1).max(1) as f32).min(ENTRY_SECONDS)
            }
        }
    }

    /// How long building up takes.
    fn build_seconds(&self, base: &AmbienceSettings) -> f32 {
        let entering = self.drift.apply(base).playing().count();
        entering.saturating_sub(1) as f32 * Self::entry_seconds(base, entering) + SWELL_SECONDS
    }

    /// Leaves out the instruments yet to enter, swells those just in, and
    /// opens the mood as they come.
    fn build(&mut self, out: &mut AmbienceSettings, base: &AmbienceSettings, now: u64) {
        let since = now.saturating_sub(self.origin) as f32 / self.rate as f32;
        let entering = out.playing().count();
        let step = Self::entry_seconds(base, entering);
        let total = entering.saturating_sub(1) as f32 * step + SWELL_SECONDS;
        self.building = since < total;
        if !self.building {
            return;
        }
        let mut rank = 0;
        for instrument in ENTRY_ORDER {
            let layer = &mut out.layers[instrument];
            if !layer.enabled {
                continue;
            }
            let enters = rank as f32 * step;
            rank += 1;
            if since < enters {
                layer.enabled = false;
            } else {
                let swell = ((since - enters) / SWELL_SECONDS).clamp(0.0, 1.0);
                layer.level *= SWELL_FROM + (1.0 - SWELL_FROM) * swell;
            }
        }
        let open = (since / total).clamp(0.0, 1.0);
        let towards = |from: f32, to: f32| from + (to - from) * open;
        out.variety = towards(out.variety.min(STILL_VARIETY), out.variety);
        out.concentration = towards(
            out.concentration.max(STILL_CONCENTRATION),
            out.concentration,
        );
        out.chaos *= open;
    }

    /// Frames until the next change: a few minutes, sooner with variety.
    fn interval(&mut self, base: &AmbienceSettings) -> u64 {
        let seconds = self.rng.range(120.0, 240.0) * (1.6 - 1.2 * base.variety);
        self.frames(seconds)
    }

    /// One step on: the key, the mood, the background or an instrument.
    fn change(&mut self, base: &AmbienceSettings) {
        let background = if related(base.background).is_empty() {
            0.0
        } else {
            2.0
        };
        let instruments = if base.playing().count() >= 2 {
            2.0
        } else {
            0.0
        };
        let weights = [3.0, 2.0, background, instruments];
        let total: f32 = weights.iter().sum();
        let mut roll = self.rng.unit() * total;
        let mut pick = 0;
        for (index, weight) in weights.into_iter().enumerate() {
            if roll < weight {
                pick = index;
                break;
            }
            roll -= weight;
        }
        match pick {
            0 => self.change_key(base),
            1 => self.change_mood(base),
            2 => self.change_background(base),
            _ => self.change_instruments(base),
        }
    }

    /// Home again, or a fifth up or down, or a neighbouring mode.
    fn change_key(&mut self, base: &AmbienceSettings) {
        let away = self.drift.root != 0 || self.drift.mode.is_some();
        if away && self.rng.chance(0.4) {
            self.drift.root = 0;
            self.drift.mode = None;
            return;
        }
        if let ScaleKind::Mode(own) = base.scale.kind
            && self.rng.chance(0.4)
        {
            let current = self.drift.mode.unwrap_or(own);
            let at = BRIGHTNESS
                .iter()
                .position(|mode| *mode == current)
                .unwrap_or(0);
            // Locrian only for those who chose it.
            let last = if own == Mode::Locrian { 6 } else { 5 };
            let brighter = at > 0 && (at >= last || self.rng.chance(0.5));
            let next = if brighter { at - 1 } else { (at + 1).min(last) };
            let mode = BRIGHTNESS[next];
            self.drift.mode = (mode != own).then_some(mode);
            return;
        }
        let fifth = if self.rng.chance(0.5) { 7 } else { 5 };
        self.drift.root = (self.drift.root + fifth) % 12;
    }

    /// A step in variety and concentration; in chaos too, if there is some.
    fn change_mood(&mut self, base: &AmbienceSettings) {
        let step = |value: f32, size: f32, reach: f32, rng: &mut Rng| {
            let size = if rng.chance(0.5) { size } else { -size };
            (value + size).clamp(-reach, reach)
        };
        self.drift.variety = step(self.drift.variety, MOOD_STEP, MOOD_REACH, &mut self.rng);
        self.drift.concentration = step(
            self.drift.concentration,
            MOOD_STEP,
            MOOD_REACH,
            &mut self.rng,
        );
        if base.chaos > 0.0 {
            self.drift.chaos = step(self.drift.chaos, CHAOS_STEP, CHAOS_REACH, &mut self.rng);
        }
    }

    /// Back to the settings' own, or across to one related to it: never
    /// further, so the place stays the one chosen.
    fn change_background(&mut self, base: &AmbienceSettings) {
        if self.drift.background.is_some() && self.rng.chance(0.5) {
            self.drift.background = None;
            return;
        }
        let choices = related(base.background);
        if choices.is_empty() {
            return;
        }
        let next = choices[self.rng.below(choices.len() as u32) as usize];
        self.drift.background = Some(next);
    }

    /// The instrument resting comes back, or one rests: never the first to
    /// enter, which holds the ground.
    fn change_instruments(&mut self, base: &AmbienceSettings) {
        if self.drift.resting.take().is_some() {
            return;
        }
        let ground = ENTRY_ORDER
            .into_iter()
            .find(|instrument| base.layer(*instrument).enabled);
        let candidates = ENTRY_ORDER
            .into_iter()
            .filter(|instrument| base.layer(*instrument).enabled && Some(*instrument) != ground);
        let count = candidates.clone().count();
        if count == 0 {
            return;
        }
        let pick = self.rng.below(count as u32) as usize;
        self.drift.resting = candidates.clone().nth(pick);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Density, Layer, Style};
    use super::*;

    const RATE: u32 = 1000;

    fn evolving() -> AmbienceSettings {
        let mut settings = Style::RainStudy.settings();
        settings.enabled = true;
        settings.journey = Journey::Evolve;
        *settings.layer_mut(Instrument::Pad) = Layer::on(0.6, Density::SPARSE);
        *settings.layer_mut(Instrument::Bells) = Layer::on(0.5, Density::SPARSE);
        settings
    }

    #[test]
    fn a_steady_journey_plays_the_settings_as_they_are() {
        let settings = Style::Campfire.settings();
        let mut course = Course::new(RATE, 1);
        course.reset(1, 0);
        for at in [0, RATE as u64 * 30, RATE as u64 * 3600] {
            assert_eq!(course.settings(&settings, at, 0), settings);
        }
    }

    #[test]
    fn a_build_brings_the_instruments_in_one_at_a_time_as_the_mood_opens() {
        let mut settings = evolving();
        settings.journey = Journey::Build;
        settings.chaos = 0.4;
        let mut course = Course::new(RATE, 2);
        course.reset(2, 0);
        let at = |seconds: f32| (seconds * RATE as f32) as u64;
        let playing = |settings: &AmbienceSettings| settings.playing().collect::<Vec<_>>();

        // The pad first, alone, quiet and still.
        let start = course.settings(&settings, 0, 0);
        assert_eq!(playing(&start), [Instrument::Pad]);
        assert!(start.layer(Instrument::Pad).level < 0.6 * 0.5);
        assert!(start.variety <= STILL_VARIETY && start.concentration >= STILL_CONCENTRATION);
        assert_eq!(start.chaos, 0.0);
        // Then the bass, then the harp, then the bells.
        let soon = course.settings(&settings, at(ENTRY_SECONDS + 1.0), 0);
        assert_eq!(playing(&soon), [Instrument::Bass, Instrument::Pad]);
        let later = course.settings(&settings, at(ENTRY_SECONDS * 2.0 + 1.0), 0);
        assert_eq!(playing(&later).len(), 3);
        assert!(course.building);
        // Built: everything as asked, and it stays so.
        let built = at(ENTRY_SECONDS * 3.0 + SWELL_SECONDS + 1.0);
        for at in [built, built * 10] {
            let now = course.settings(&settings, at, 0);
            assert_eq!(now, settings);
        }
        assert!(!course.building);
    }

    #[test]
    fn a_piece_builds_up_within_itself_each_time() {
        let mut settings = evolving();
        settings.journey = Journey::Build;
        settings.form = Form::Piece {
            minutes: 1.0,
            pause_seconds: 10.0,
        };
        let mut course = Course::new(RATE, 3);
        course.reset(3, 0);
        // Four instruments over 40 % of a minute: one every eight seconds.
        let now = course.settings(&settings, 9 * RATE as u64, 0);
        assert_eq!(now.playing().count(), 2);
        // The next piece starts over.
        let next = 70 * RATE as u64;
        let now = course.settings(&settings, next + RATE as u64, next);
        assert_eq!(now.playing().count(), 1);
    }

    #[test]
    fn an_evolving_course_moves_on_now_and_then_and_stays_related() {
        let settings = evolving();
        let mut course = Course::new(RATE, 4);
        course.reset(4, 0);
        let mut changes = 0;
        let mut last = course.settings(&settings, 0, 0);
        let mut keys = std::collections::HashSet::new();
        let hour = 3600 * RATE as u64;
        for at in (0..hour).step_by(RATE as usize) {
            let now = course.settings(&settings, at, 0);
            if course.building {
                last = now;
                continue;
            }
            if now != last {
                changes += 1;
            }
            // Never further than the drift allows.
            assert!((now.variety - settings.variety).abs() <= MOOD_REACH + 1e-6);
            assert!((now.concentration - settings.concentration).abs() <= MOOD_REACH + 1e-6);
            assert_eq!(now.chaos, 0.0, "no chaos where there was none");
            assert!(
                now.background == settings.background
                    || related(settings.background).contains(&now.background),
                "{:?}",
                now.background
            );
            let resting = Instrument::ALL
                .into_iter()
                .filter(|i| settings.layer(*i).enabled && !now.layer(*i).enabled)
                .count();
            assert!(resting <= 1);
            assert!(now.layer(Instrument::Pad).enabled, "the ground stays");
            if let ScaleKind::Mode(mode) = now.scale.kind {
                assert_ne!(mode, Mode::Locrian);
            }
            keys.insert(now.scale);
            last = now;
        }
        // Every two to four minutes: about twenty in an hour.
        assert!((12..=32).contains(&changes), "{changes} changes in an hour");
        assert!(keys.len() >= 2, "the key moved");

        // The same seed takes the same course.
        let mut again = Course::new(RATE, 4);
        again.reset(4, 0);
        let mut first = Course::new(RATE, 4);
        first.reset(4, 0);
        for at in (0..hour).step_by(RATE as usize * 10) {
            assert_eq!(
                first.settings(&settings, at, 0),
                again.settings(&settings, at, 0)
            );
        }
    }

    #[test]
    fn no_background_stays_none_and_one_instrument_never_rests() {
        let mut settings = Style::Cathedral.settings();
        settings.enabled = true;
        settings.journey = Journey::Evolve;
        for instrument in Instrument::ALL {
            settings.layers[instrument].enabled = instrument == Instrument::Bells;
        }
        let mut course = Course::new(RATE, 5);
        course.reset(5, 0);
        for at in (0..3600 * RATE as u64).step_by(RATE as usize) {
            let now = course.settings(&settings, at, 0);
            assert_eq!(now.background, Background::None);
            assert!(now.layer(Instrument::Bells).enabled);
        }
    }

    #[test]
    fn what_plays_now_survives_the_trip_through_an_atomic() {
        let now = Now {
            scale: Scale {
                root: 11,
                kind: ScaleKind::WholeTone,
            },
            background: Background::Fire,
            resting: Some(Instrument::Drums),
            building: true,
        };
        assert_eq!(Now::unpack(now.pack()), Some(now));
        let plain = Now {
            resting: None,
            building: false,
            ..now
        };
        assert_eq!(Now::unpack(plain.pack()), Some(plain));
        assert_eq!(Now::unpack(0), None);
    }
}

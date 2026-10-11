//! Level meters: how a peak level read from the engine is shown.
//!
//! The engine reports the loudest sample since the last read. A [`Meter`]
//! rises to it at once and falls back steadily, holds the highest level
//! for a moment as a tick, and lights a clip lamp when the output went
//! over full scale. Levels are in dBFS: 0 is full scale.

use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Ui, Vec2, WidgetInfo, WidgetType};

/// The quietest level drawn; anything below reads as silence.
pub const FLOOR_DB: f32 = -48.0;
/// How fast the bar falls back.
const FALL_DB_PER_SECOND: f32 = 26.0;
/// How long the peak tick stays before it falls.
const HOLD_SECONDS: f64 = 1.2;
/// How long the clip lamp stays lit.
const CLIP_SECONDS: f64 = 2.0;
/// Where the colours change: green below, amber, then red near full scale.
const AMBER_DB: f32 = -12.0;
const RED_DB: f32 = -3.0;

#[derive(Clone, Copy, Debug)]
pub struct Meter {
    level_db: f32,
    hold_db: f32,
    held_since: f64,
    clipped_at: Option<f64>,
    last_fed: Option<f64>,
}

impl Default for Meter {
    fn default() -> Self {
        Self {
            level_db: f32::NEG_INFINITY,
            hold_db: f32::NEG_INFINITY,
            held_since: 0.0,
            clipped_at: None,
            last_fed: None,
        }
    }
}

impl Meter {
    /// Takes the loudest level since the last call (linear, 1.0 is full
    /// scale) at time `now` in seconds.
    pub fn feed(&mut self, peak: f32, now: f64) {
        let elapsed = self.last_fed.map_or(0.0, |last| (now - last).max(0.0));
        self.last_fed = Some(now);
        let fall = FALL_DB_PER_SECOND * elapsed as f32;
        let db = to_db(peak);
        self.level_db = db.max(self.level_db - fall);
        if db >= self.hold_db {
            self.hold_db = db;
            self.held_since = now;
        } else if now - self.held_since > HOLD_SECONDS {
            self.hold_db = (self.hold_db - fall).max(self.level_db);
        }
        if peak > 1.0 {
            self.clipped_at = Some(now);
        }
    }

    pub fn level_db(&self) -> f32 {
        self.level_db
    }

    pub fn hold_db(&self) -> f32 {
        self.hold_db
    }

    /// The level went over full scale in the last couple of seconds.
    pub fn clipped(&self, now: f64) -> bool {
        self.clipped_at
            .is_some_and(|at| (0.0..CLIP_SECONDS).contains(&(now - at)))
    }

    /// Nothing to draw: silent, no tick, no clip lamp. Until then the
    /// window keeps redrawing so the bar can fall.
    pub fn is_resting(&self, now: f64) -> bool {
        self.level_db < FLOOR_DB && self.hold_db < FLOOR_DB && !self.clipped(now)
    }
}

pub fn to_db(linear: f32) -> f32 {
    if linear > 0.0 {
        20.0 * linear.log10()
    } else {
        f32::NEG_INFINITY
    }
}

/// How far along a bar a level reaches: 0 at the floor, 1 at full scale.
pub fn fraction(db: f32) -> f32 {
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

/// "-12 dB", "0 dB", "+2 dB", or "silent".
pub fn readout(db: f32) -> String {
    if db < FLOOR_DB {
        return "silent".to_owned();
    }
    let whole = db.round() as i32;
    if whole > 0 {
        format!("+{whole} dB")
    } else {
        format!("{whole} dB")
    }
}

/// How a bar is coloured.
#[derive(Clone, Copy)]
pub enum Colors {
    /// Green, amber and red, as on a mixing desk.
    Zones,
    /// One colour (a tile's), turning red near full scale.
    Tinted(Color32),
}

fn zone_colors(dark: bool) -> [Color32; 3] {
    if dark {
        [
            Color32::from_rgb(52, 199, 89),
            Color32::from_rgb(255, 184, 0),
            Color32::from_rgb(255, 69, 58),
        ]
    } else {
        [
            Color32::from_rgb(36, 160, 70),
            Color32::from_rgb(222, 145, 0),
            Color32::from_rgb(215, 45, 45),
        ]
    }
}

pub fn track_color(dark: bool) -> Color32 {
    if dark {
        Color32::from_white_alpha(18)
    } else {
        Color32::from_black_alpha(22)
    }
}

/// Paints one bar: along `rect` from the left, or from the bottom when it
/// is taller than wide.
pub fn paint_bar(painter: &Painter, rect: Rect, meter: &Meter, colors: Colors, dark: bool) {
    let vertical = rect.height() > rect.width();
    let span = |from: f32, to: f32| -> Rect {
        if vertical {
            Rect::from_x_y_ranges(
                rect.x_range(),
                rect.bottom() - to * rect.height()..=rect.bottom() - from * rect.height(),
            )
        } else {
            Rect::from_x_y_ranges(
                rect.left() + from * rect.width()..=rect.left() + to * rect.width(),
                rect.y_range(),
            )
        }
    };
    let radius = (rect.width().min(rect.height()) / 2.0).min(3.0) as u8;
    painter.rect_filled(rect, radius, track_color(dark));

    let [green, amber, red] = zone_colors(dark);
    let zones = match colors {
        Colors::Zones => [
            (0.0, fraction(AMBER_DB), green),
            (fraction(AMBER_DB), fraction(RED_DB), amber),
            (fraction(RED_DB), 1.0, red),
        ],
        Colors::Tinted(color) => [
            (0.0, fraction(RED_DB), color),
            (fraction(RED_DB), fraction(RED_DB), color),
            (fraction(RED_DB), 1.0, red),
        ],
    };
    let level = fraction(meter.level_db());
    let clipped = painter.with_clip_rect(rect);
    for (from, to, color) in zones {
        let to = to.min(level);
        if to > from {
            clipped.rect_filled(span(from, to), 0, color);
        }
    }
    let hold = fraction(meter.hold_db());
    if hold > 0.0 {
        let color = zones
            .iter()
            .find(|(from, to, _)| (*from..=*to).contains(&hold))
            .map_or(red, |zone| zone.2);
        let tick = 2.0
            / if vertical {
                rect.height()
            } else {
                rect.width()
            };
        clipped.rect_filled(span((hold - tick).max(0.0), hold), 0, color);
    }
}

/// The output meter in the toolbar: a bar per channel and a clip lamp.
pub fn stereo(ui: &mut Ui, meters: &[Meter; 2], now: f64, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let dark = ui.visuals().dark_mode;
    let lamp = size.y * 0.55;
    let bars = Rect::from_min_max(
        rect.min,
        Pos2::new(rect.right() - lamp - 6.0, rect.bottom()),
    );
    let gap = 2.0;
    let height = (bars.height() - gap) / 2.0;
    for (index, meter) in meters.iter().enumerate() {
        let top = bars.top() + index as f32 * (height + gap);
        let bar = Rect::from_x_y_ranges(bars.x_range(), top..=top + height);
        paint_bar(ui.painter(), bar, meter, Colors::Zones, dark);
    }
    let clipped = meters.iter().any(|meter| meter.clipped(now));
    let lamp_color = if clipped {
        zone_colors(dark)[2]
    } else {
        track_color(dark)
    };
    ui.painter().circle_filled(
        Pos2::new(rect.right() - lamp / 2.0, rect.center().y),
        lamp / 2.0,
        lamp_color,
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Output level"));
    let [left, right] = meters.map(|meter| readout(meter.hold_db()));
    let mut text = format!("Output level, loudest lately: left {left}, right {right}");
    if clipped {
        text.push_str("\nThe output clipped: lower the master volume or the tiles.");
    }
    response.on_hover_text(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meter_jumps_up_and_falls_back_steadily() {
        let mut meter = Meter::default();
        meter.feed(0.5, 0.0);
        assert!((meter.level_db() - -6.02).abs() < 0.01);
        // Half a second of silence: down by 13 dB.
        meter.feed(0.0, 0.5);
        assert!((meter.level_db() - -19.02).abs() < 0.01);
        // A louder peak is shown at once.
        meter.feed(1.0, 0.6);
        assert_eq!(meter.level_db(), 0.0);
    }

    #[test]
    fn the_peak_tick_holds_then_falls_and_the_meter_comes_to_rest() {
        let mut meter = Meter::default();
        meter.feed(0.5, 0.0);
        for step in 1..=10 {
            meter.feed(0.0, f64::from(step) * 0.1);
        }
        // Held for 1.2 s.
        assert!((meter.hold_db() - -6.02).abs() < 0.01);
        let mut now = 1.0;
        while now < 4.0 {
            now += 0.05;
            meter.feed(0.0, now);
        }
        assert!(meter.is_resting(now));
    }

    #[test]
    fn going_over_full_scale_lights_the_clip_lamp_for_a_while() {
        let mut meter = Meter::default();
        meter.feed(1.0, 0.0);
        assert!(!meter.clipped(0.0), "full scale is not over it");
        meter.feed(1.3, 1.0);
        assert!(meter.clipped(2.5));
        assert!(!meter.clipped(3.5));
    }

    #[test]
    fn readouts_round_and_name_silence() {
        assert_eq!(readout(f32::NEG_INFINITY), "silent");
        assert_eq!(readout(-60.0), "silent");
        assert_eq!(readout(-12.4), "-12 dB");
        assert_eq!(readout(-0.2), "0 dB");
        assert_eq!(readout(2.3), "+2 dB");
        assert_eq!(fraction(0.0), 1.0);
        assert_eq!(fraction(FLOOR_DB), 0.0);
    }
}

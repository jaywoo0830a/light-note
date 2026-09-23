//! Ink: what a pen sample becomes, and why it feels the way it does.
//!
//! A digitizer streams reports (often 133–400 per second).  Turning that stream
//! into something that looks like handwriting is three small decisions, all of
//! them here:
//!
//! * **width follows pressure** — `width_pt * (0.35 + 0.65 * pressure)`, so a
//!   light touch is thinner but never invisible;
//! * **pressure is smoothed** — an exponential filter, because digitizer
//!   pressure is noisy and an unsmoothed line looks furry;
//! * **samples are decimated** — points closer than 0.4 pt are dropped *unless*
//!   the pressure moved, which keeps the model small without flattening the
//!   line's dynamics;
//! * **tilt and the barrel's rotation shape the nib** — a pen lying flat leaves
//!   a wider mark, and a flat nib turned by the barrel draws a thin line along
//!   its axis and a thick one across it ([`Nib`]).
//!
//! Everything here is a plain value: `Send`, no platform type, no lock.

use serde::{Deserialize, Serialize};

use crate::geom::{Pt, Rect};

/// A pen that reports no pressure still leaves 35% of the nib width.
///
/// `f64` because [`width_at`] computes the nib width in `f64`: as `f32`, `0.35`
/// is 0.3499999940395355, and a 2 pt nib at no pressure would come out as
/// 0.699999988079071 instead of 0.7.
pub const MIN_WIDTH_RATIO: f64 = 0.35;
/// How much of a pressure change is applied per sample (exponential filter).
pub const PRESSURE_SMOOTHING: f32 = 0.35;
/// Samples closer than this (pt) are dropped unless the pressure changed.
pub const MIN_SAMPLE_DISTANCE_PT: f32 = 0.4;
/// A pressure change smaller than this is noise, not intent.
pub const MIN_PRESSURE_DELTA: f32 = 0.02;
/// Speed-derived pressure never goes below this (a fast stroke is still a line).
pub const MIN_SPEED_PRESSURE: f32 = 0.15;
/// Speed (pt/ms) at which speed-derived pressure bottoms out.
pub const SPEED_FOR_MIN_PRESSURE: f32 = 2.5;
/// How much wider a fully tilted (flat) nib draws: a pen lying down leaves
/// `1 + TILT_WIDTH_GAIN` times the nib's width.
///
/// `f64` for the same reason as [`MIN_WIDTH_RATIO`]: the geometry layer is
/// `f64`, and the ratio is multiplied before anything is rounded.
pub const TILT_WIDTH_GAIN: f64 = 0.6;
/// Tilt (deg) at which the nib counts as fully flat.
pub const MAX_TILT_DEG: f32 = 90.0;

/// What the pen is doing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum Tool {
    /// Ink that follows pressure.
    #[default]
    Pen,
    /// Translucent ink of a constant width.
    Highlighter,
    /// Removes whole strokes instead of drawing.
    Eraser,
}

impl Tool {
    /// Does this tool put ink on the page?
    pub const fn writes_ink(self) -> bool {
        matches!(self, Self::Pen | Self::Highlighter)
    }

    /// Does this tool remove strokes?
    pub const fn erases(self) -> bool {
        matches!(self, Self::Eraser)
    }

    /// The width a new stroke of this tool starts with (pt).
    pub const fn default_width_pt(self) -> f32 {
        match self {
            Self::Pen => 2.0,
            Self::Highlighter => 14.0,
            Self::Eraser => 12.0,
        }
    }

    /// The color ink of this tool is drawn with.
    pub const fn color(self) -> Rgba {
        match self {
            Self::Pen => Rgba::rgb(28, 32, 38),
            Self::Highlighter => Rgba::rgba(255, 214, 64, 90),
            Self::Eraser => Rgba::rgb(160, 166, 176),
        }
    }
}

/// Straight 8-bit RGBA.  Premultiplication happens only inside the rasterizer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// True when the color is fully opaque (no compositing needed).
    pub const fn is_opaque(self) -> bool {
        self.a == 255
    }

    /// This color as it looks on white paper.
    ///
    /// Pdfium page objects carry no alpha channel, so translucent ink is
    /// exported as the color it *appears* to be on white — what a reader sees
    /// matches what the app showed.
    pub fn over_white(self) -> Self {
        let alpha = self.a as u32;
        let blend = |channel: u8| ((channel as u32 * alpha + 255 * (255 - alpha)) / 255) as u8;
        Self::rgb(blend(self.r), blend(self.g), blend(self.b))
    }
}

/// How a stroke is drawn: tool, nib width and color.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub tool: Tool,
    pub width_pt: f32,
    pub color: Rgba,
}

impl Style {
    /// A style for `tool`, with the tool's default color.
    pub fn new(tool: Tool, width_pt: f32) -> Self {
        Self {
            tool,
            width_pt: width_pt.clamp(0.1, 200.0),
            color: tool.color(),
        }
    }

    /// The same style in another color (the palette, later).
    pub fn with_color(mut self, color: Rgba) -> Self {
        self.color = color;
        self
    }
}

/// How the pen is held: tilt (deg, as the tablet reports it) and the barrel's
/// rotation (deg).  Both are zero when the hardware does not report them, which
/// is why [`Nib::default`] means "a pen held upright".
///
/// The two effects this produces:
///
/// * **a flat nib draws wider** — the mark grows up to [`TILT_WIDTH_GAIN`] with
///   the tilt, so pressure is not the only thing that changes the line's weight;
/// * **the direction matters** — a flat nib's edge lies along the lean
///   direction (turned by the barrel's rotation), so a stroke across that axis
///   is thick and a stroke along it is thin, like a chisel-tip marker
///   ([`Nib::width_ratio`]).
///
/// The angle in the stream is the lean, so `atan2` never fails; only the
/// *magnitude* is used when the direction is unknown (a dot).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Nib {
    /// Tilt towards +x, degrees (0 = upright).
    pub tilt_x: f32,
    /// Tilt towards +y, degrees.
    pub tilt_y: f32,
    /// Barrel rotation, degrees (0 = untouched).
    pub rotation: f32,
}

impl Nib {
    /// A pen held straight up: a round nib, no direction.
    pub const UPRIGHT: Self = Self {
        tilt_x: 0.0,
        tilt_y: 0.0,
        rotation: 0.0,
    };

    /// A nib from the three numbers a tablet reports.
    pub const fn new(tilt_x: f32, tilt_y: f32, rotation: f32) -> Self {
        Self {
            tilt_x,
            tilt_y,
            rotation,
        }
    }

    /// Is the pen upright (no tilt worth applying)?
    pub fn is_upright(&self) -> bool {
        self.flatness() <= 0.0
    }

    /// `0` for an upright pen, `1` for one lying flat on the tablet.
    ///
    /// A tablet that reports a wild tilt (or `NaN`) is clamped, never trusted:
    /// the width it produces has to stay finite.
    pub fn flatness(&self) -> f32 {
        let magnitude = (self.tilt_x * self.tilt_x + self.tilt_y * self.tilt_y).sqrt();
        if !magnitude.is_finite() {
            return 0.0;
        }
        (magnitude / MAX_TILT_DEG).clamp(0.0, 1.0)
    }

    /// The angle (radians) the nib's flat edge lies along.
    ///
    /// The lean gives the direction the nib points *down* to; the barrel's
    /// rotation turns that edge.  `None` for an upright pen, whose nib is round
    /// and therefore has no axis.
    ///
    /// The ratio built from this axis uses `|sin|`, so it is π-periodic: a nib
    /// rotated by half a turn is the same nib, which is exactly how a symmetric
    /// flat nib behaves.
    pub fn axis(&self) -> Option<f32> {
        if self.is_upright() {
            return None;
        }
        Some(self.tilt_y.atan2(self.tilt_x) + self.rotation.to_radians())
    }

    /// How much wider than the upright nib the mark is for a stroke heading in
    /// `direction` (radians, page axes; `None` = the heading is unknown).
    ///
    /// `|sin|` of the angle between the nib's edge and the heading: 1 across the
    /// edge (widest), 0 along it (only the nib's thickness).  With no heading
    /// the widest value is used, which is what a dot and a bounding box need.
    pub fn width_ratio(&self, direction: Option<f32>) -> f64 {
        let flatness = self.flatness() as f64;
        if flatness <= 0.0 {
            return 1.0;
        }
        let Some(axis) = self.axis() else {
            return 1.0;
        };
        let alignment = match direction {
            Some(direction) => (direction - axis).sin().abs() as f64,
            None => 1.0,
        };
        1.0 + TILT_WIDTH_GAIN * flatness * alignment
    }
}

/// One sample of a stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InkPoint {
    /// Position on the page (pt, top-left origin).
    pub pos: Pt,
    /// Pressure in `0..=1` (already normalized by the tablet spec).
    pub pressure: f32,
    /// How the pen was held when this sample was taken.
    pub nib: Nib,
    /// Time since the stroke started (ms) — used to derive speed.
    pub time_ms: f64,
}

impl InkPoint {
    /// An upright-pen sample.
    pub const fn new(pos: Pt, pressure: f32, time_ms: f64) -> Self {
        Self {
            pos,
            pressure,
            nib: Nib::UPRIGHT,
            time_ms,
        }
    }

    /// A sample from a pen held at an angle.
    pub const fn with_nib(pos: Pt, pressure: f32, nib: Nib, time_ms: f64) -> Self {
        Self {
            pos,
            pressure,
            nib,
            time_ms,
        }
    }
}

/// The width a stroke of `style` has at `pressure` with the nib `nib`, in pt.
///
/// `direction` is the heading of the stroke where this width is used (radians);
/// `None` means "unknown", and then a tilted nib contributes its **widest**
/// footprint — the value a dot, a bounding box and a hit test all need.
///
/// The ratio is computed in `f64` and rounded once at the end.  The same
/// expression in `f32` gives 1.3499999 for a 2 pt nib at half pressure (because
/// `0.35f32` and `0.65f32` are both a little low), and the geometry layer —
/// which is `f64` — would carry that error into every span width.
pub fn width_at(style: &Style, pressure: f32, nib: Nib, direction: Option<f32>) -> f64 {
    if style.tool == Tool::Highlighter {
        // A highlighter is a felt marker: pressure and tilt do not change its
        // line, only the tool's width does.
        return style.width_pt as f64;
    }
    let ratio = MIN_WIDTH_RATIO + (1.0 - MIN_WIDTH_RATIO) * clamp01(pressure) as f64;
    style.width_pt as f64 * ratio * nib.width_ratio(direction)
}

/// The heading (radians) from one sample to the next, or `None` when the pen did
/// not move (a dot has no direction, so a tilted nib keeps its widest mark).
pub fn heading(from: Pt, to: Pt) -> Option<f32> {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    if dx == 0.0 && dy == 0.0 {
        None
    } else {
        Some(dy.atan2(dx))
    }
}

/// Pressure derived from speed (pt/ms) for tablets that report none.
pub fn pressure_from_speed(speed_pt_per_ms: f32) -> f32 {
    if !speed_pt_per_ms.is_finite() {
        return 1.0;
    }
    (1.0 - speed_pt_per_ms / SPEED_FOR_MIN_PRESSURE).clamp(MIN_SPEED_PRESSURE, 1.0)
}

/// One step of the pressure filter: `previous` moves [`PRESSURE_SMOOTHING`] of
/// the way towards `raw`.
pub fn smooth_pressure(previous: f32, raw: f32) -> f32 {
    let previous = clamp01(previous);
    previous + (clamp01(raw) - previous) * PRESSURE_SMOOTHING
}

/// Should `next` be added to a stroke whose last point is `last`?
///
/// The answer is "yes" when the pen moved far enough *or* when the pressure
/// changed enough — the second half is what keeps a slow, deliberate stroke
/// from being flattened into a straight line.
pub fn keep_sample(last: &InkPoint, next: &InkPoint) -> bool {
    last.pos.distance_to(next.pos) >= MIN_SAMPLE_DISTANCE_PT
        || (next.pressure - last.pressure).abs() >= MIN_PRESSURE_DELTA
}

fn clamp01(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// A stroke: a style plus the points that survived [`keep_sample`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    style: Style,
    points: Vec<InkPoint>,
}

impl Stroke {
    pub fn new(style: Style) -> Self {
        Self {
            style,
            points: Vec::new(),
        }
    }

    /// Adds a sample.  The first sample is always kept; later ones go through
    /// [`keep_sample`], so a stationary pen does not grow the model.
    pub fn push(&mut self, point: InkPoint) {
        match self.points.last() {
            None => self.points.push(point),
            Some(last) if keep_sample(last, &point) => self.points.push(point),
            Some(_) => {}
        }
    }

    pub fn style(&self) -> &Style {
        &self.style
    }

    pub fn points(&self) -> &[InkPoint] {
        &self.points
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// The area the stroke covers, nib width included.
    ///
    /// A tilted nib is measured with its widest footprint (`None` heading), so
    /// the box never under-covers the ink it is asked to describe.
    pub fn bounds(&self) -> Rect {
        let mut rect = Rect::default();
        for point in &self.points {
            let half = (width_at(&self.style, point.pressure, point.nib, None) * 0.5) as f32;
            rect = rect.union(Rect::new(
                point.pos.x - half,
                point.pos.y - half,
                half * 2.0,
                half * 2.0,
            ));
        }
        rect
    }

    /// Is `point` within `radius` of the stroke's centerline?
    ///
    /// This is the eraser's hit test, and it is why the nib erases what the eye
    /// thinks it touched: the stroke's own width counts, not just its path.  The
    /// widest footprint is used, so a flat nib erases as much as it can touch.
    pub fn touches(&self, point: Pt, radius: f32) -> bool {
        let points = &self.points;
        if points.is_empty() {
            return false;
        }
        if points.len() == 1 {
            let half = (width_at(&self.style, points[0].pressure, points[0].nib, None) * 0.5) as f32;
            return points[0].pos.distance_to(point) <= radius + half;
        }
        points.windows(2).any(|pair| {
            let nib = pair[0].nib;
            let half =
                (width_at(&self.style, (pair[0].pressure + pair[1].pressure) * 0.5, nib, None) * 0.5)
                    as f32;
            distance_to_segment(point, pair[0].pos, pair[1].pos) <= radius + half
        })
    }
}

/// Distance from `point` to the segment `from`–`to` (pt).
pub fn distance_to_segment(point: Pt, from: Pt, to: Pt) -> f32 {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f32::EPSILON {
        return point.distance_to(from);
    }
    let t = (((point.x - from.x) * dx + (point.y - from.y) * dy) / length_squared).clamp(0.0, 1.0);
    point.distance_to(Pt::new(from.x + t * dx, from.y + t * dy))
}

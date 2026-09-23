//! Units: **pt** (model) and **px** (drawing), and the one value between them.
//!
//! A page is measured in points (1/72 inch) because that is what PDF and ink
//! use.  WinUI and bitmaps are measured in pixels.  `scale` is the only
//! conversion factor in the app: `px = pt * scale`, and at 100% zoom it is
//! [`DEFAULT_SCALE`] — a 1.5x screen, so a 96 dpi display shows a PDF at a
//! comfortable size instead of the 1.33x that "96/72" would give.

use serde::{Deserialize, Serialize};

/// Pixels per point at 100% zoom.
pub const DEFAULT_SCALE: f32 = 1.5;

/// A point on a page (pt, origin top-left, y grows downwards).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

impl Pt {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Euclidean distance (pt).
    pub fn distance_to(self, other: Self) -> f32 {
        let dx = other.x - self.x;
        let dy = other.y - self.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// A width/height in pt (a page) or px (a bitmap).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// Pixel dimensions for a bitmap, rounded and never zero.
    ///
    /// A zero-width bitmap is not a bitmap, and a page smaller than one pixel
    /// still has to be drawn somewhere.
    pub fn to_pixels(self, scale: Scale) -> (u16, u16) {
        let width = (self.width * scale.get()).round().max(1.0);
        let height = (self.height * scale.get()).round().max(1.0);
        (
            width.min(u16::MAX as f32) as u16,
            height.min(u16::MAX as f32) as u16,
        )
    }

    /// The largest size with this aspect ratio that fits into `target`.
    pub fn fit_into(self, target: Size) -> Size {
        if self.width <= 0.0 || self.height <= 0.0 || target.width <= 0.0 || target.height <= 0.0 {
            return Size::new(0.0, 0.0);
        }
        let factor = (target.width / self.width).min(target.height / self.height);
        Size::new(self.width * factor, self.height * factor)
    }
}

/// An axis-aligned rectangle in pt (or px, when the caller says so).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(self, point: Pt) -> bool {
        point.x >= self.x
            && point.x <= self.x + self.width
            && point.y >= self.y
            && point.y <= self.y + self.height
    }

    /// The same rectangle grown by `margin` on every side.
    pub fn expand(self, margin: f32) -> Self {
        Self::new(
            self.x - margin,
            self.y - margin,
            self.width + margin * 2.0,
            self.height + margin * 2.0,
        )
    }

    /// The smallest rectangle containing both.
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Self::new(left, top, right - left, bottom - top)
    }

    /// Overlap, not touch: two rectangles that share an edge do not intersect.
    pub fn intersects(self, other: Self) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }

    pub fn center(self) -> Pt {
        Pt::new(self.x + self.width * 0.5, self.y + self.height * 0.5)
    }

    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    /// The bounding box of a set of points (empty for no points).
    pub fn from_points(points: &[Pt]) -> Self {
        let mut left = f32::MAX;
        let mut top = f32::MAX;
        let mut right = f32::MIN;
        let mut bottom = f32::MIN;
        for point in points {
            left = left.min(point.x);
            top = top.min(point.y);
            right = right.max(point.x);
            bottom = bottom.max(point.y);
        }
        if left > right {
            return Self::default();
        }
        Self::new(left, top, right - left, bottom - top)
    }
}

/// Pixels per point, clamped to a zoom range that can still be drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale(f32);

impl Scale {
    /// 100% zoom.
    pub const DEFAULT: Self = Self(DEFAULT_SCALE);
    /// 10% — a whole page is still a page.
    pub const MIN: Self = Self(0.1);
    /// 2000% — past the point where a single point covers the screen.
    pub const MAX: Self = Self(20.0);

    /// Clamps the value; `NaN` falls back to [`Scale::MIN`], infinity to the cap.
    pub fn new(value: f32) -> Self {
        if value.is_nan() {
            return Self::MIN;
        }
        Self(value.clamp(Self::MIN.0, Self::MAX.0))
    }

    /// Zoom in percent (`100.0` is [`Scale::DEFAULT`]).
    pub fn from_zoom(percent: f32) -> Self {
        Self::new(DEFAULT_SCALE * percent / 100.0)
    }

    pub fn get(self) -> f32 {
        self.0
    }

    /// Points to pixels.
    pub fn px(self, points: f32) -> f32 {
        points * self.0
    }

    /// Pixels to points.
    pub fn pt(self, pixels: f32) -> f32 {
        pixels / self.0
    }

    /// A point in pt to a pixel pair.
    pub fn px_point(self, point: Pt) -> (f32, f32) {
        (self.px(point.x), self.px(point.y))
    }
}

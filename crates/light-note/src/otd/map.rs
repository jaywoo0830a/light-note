//! Tablet → page mapping.
//!
//! OTD reports **device units** because the plugin cannot know the window size,
//! the zoom level or the page size — and that is a good thing: if the plugin
//! mapped to screen coordinates, the ink would slide around whenever the window
//! was resized.  The mapping belongs to the app, and it is a *centered fit*:
//!
//! ```text
//! factor = min((page.w - 2m) / tablet.max_x, (page.h - 2m) / tablet.max_y)
//! ink    = tablet.max * factor, centered inside the page's margins
//! ```
//!
//! Two properties are pinned by the tests: the tablet's aspect ratio is kept
//! (a circle on the tablet is a circle on the page), and the mapping depends
//! only on the tablet, the page and the margin — never on the window.

use crate::geom::{Pt, Size};

/// What the app knows about the tablet, from the shared-memory header.
#[derive(Clone, Debug, PartialEq)]
pub struct TabletSpec {
    pub name: String,
    /// Tablet width in device units.
    pub max_x: f32,
    /// Tablet height in device units.
    pub max_y: f32,
    /// Maximum pressure in device units (`0` when the tablet does not report any).
    pub max_pressure: f32,
}

impl TabletSpec {
    pub fn new(name: impl Into<String>, max_x: f32, max_y: f32, max_pressure: f32) -> Self {
        Self {
            name: name.into(),
            max_x,
            max_y,
            max_pressure,
        }
    }

    /// Does this spec have a usable range?  (A plugin that could not read the
    /// tablet leaves the maxima at zero — see `PROTOCOL.md`.)
    pub fn is_known(&self) -> bool {
        self.max_x > 0.0 && self.max_y > 0.0
    }
}

/// A stable mapping from device units to page points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageMap {
    max_x: f32,
    max_y: f32,
    max_pressure: f32,
    left: f32,
    top: f32,
    factor: f32,
}

impl PageMap {
    /// Fits the tablet's active area into `page` with `margin` pt of paper left
    /// on every side.  `None` when the tablet's range is unknown.
    pub fn fit(spec: &TabletSpec, page: Size, margin: f32) -> Option<Self> {
        if !spec.is_known() {
            return None;
        }
        let margin = margin.max(0.0);
        let available_width = page.width - margin * 2.0;
        let available_height = page.height - margin * 2.0;
        if available_width <= 0.0 || available_height <= 0.0 {
            return None;
        }
        let factor = (available_width / spec.max_x).min(available_height / spec.max_y);
        if !factor.is_finite() || factor <= 0.0 {
            return None;
        }
        let ink_width = spec.max_x * factor;
        let ink_height = spec.max_y * factor;
        Some(Self {
            max_x: spec.max_x,
            max_y: spec.max_y,
            max_pressure: spec.max_pressure,
            left: margin + (available_width - ink_width) * 0.5,
            top: margin + (available_height - ink_height) * 0.5,
            factor,
        })
    }

    /// The top-left corner of the ink area (page pt).
    pub fn origin(&self) -> (f32, f32) {
        (self.left, self.top)
    }

    /// The bottom-right corner of the ink area (page pt).
    pub fn far_corner(&self) -> (f32, f32) {
        (
            self.left + self.max_x * self.factor,
            self.top + self.max_y * self.factor,
        )
    }

    /// A device coordinate to a page point, clamped into the ink area.
    pub fn to_page(&self, x: f32, y: f32) -> Pt {
        let (left, top) = self.origin();
        let (right, bottom) = self.far_corner();
        Pt::new(
            (left + x * self.factor).clamp(left, right),
            (top + y * self.factor).clamp(top, bottom),
        )
    }

    /// Raw pressure to `0..=1`.  A tablet that reports no pressure is treated as
    /// full pressure, so the ink still appears.
    pub fn pressure(&self, raw: f32) -> f32 {
        if !raw.is_finite() || raw <= 0.0 {
            return 0.0;
        }
        if self.max_pressure <= 0.0 {
            return 1.0;
        }
        (raw / self.max_pressure).clamp(0.0, 1.0)
    }
}
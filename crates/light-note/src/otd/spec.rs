//! What the tablet says: its range, and the pen's live attributes.
//!
//! OTD reports raw device units, and this module turns them into the two things
//! the app actually uses:
//!
//! * **the range** ([`TabletSpec`]) — needed to scale raw pressure (`0..=8192`)
//!   into `0..=1`.  The range is *not* used to place ink: the position of a
//!   sample comes from the canvas's own pointer event, so the ink lands exactly
//!   where the pointer is and never depends on the window, the zoom or the page
//!   size;
//! * **the attributes** ([`PenState`]) — pressure, tilt, the barrel's rotation
//!   and the eraser flag.  These travel through shared memory because WinUI's
//!   pointer event does not carry them.
//!
//! Both are plain values with no platform type, which is why the whole thing is
//! covered by `tests/`.

use crate::ink::Nib;

use super::Sample;

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

/// The pen's attributes as of one report: everything a stroke needs **except**
/// the position.
///
/// The position deliberately does not live here.  WinUI reports the pointer's
/// position on the canvas, which is the same place the cursor is; taking the
/// tablet's own coordinates instead would mean the ink could land somewhere else
/// than where the pen is, and the mapping would have to know about the window,
/// the zoom and the page (which is exactly the complexity this type removes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PenState {
    /// Pressure in `0..=1` (already normalized by the tablet's range).
    pub pressure: f32,
    /// Tilt and barrel rotation, in degrees.
    pub nib: Nib,
    /// Is the eraser end of the pen on the tablet?
    pub erasing: bool,
    /// Is the tip touching?  (`false` = hovering, which must not draw.)
    pub touching: bool,
    /// Pen buttons, bits 4..=11 of the report's flags (`0` = none pressed).
    pub buttons: u8,
}

impl PenState {
    /// Reads the attributes out of one report.
    ///
    /// `spec` scales the pressure; a report with no pressure at all keeps `0.0`
    /// here and the caller falls back to speed-derived pressure.
    pub fn from_sample(sample: &Sample, spec: Option<&TabletSpec>) -> Self {
        let pressure = match spec {
            Some(spec) => spec.pressure(sample.pressure),
            // No range yet (the daemon has not answered): any pressure at all is
            // full pressure — the same rule the spec applies to a tablet without
            // a pressure sensor, and better than scaling by a number nobody has.
            None if sample.pressure > 0.0 => 1.0,
            None => 0.0,
        };
        Self {
            pressure,
            nib: Nib::new(sample.tilt_x, sample.tilt_y, sample.rotation),
            erasing: sample.is_eraser(),
            touching: sample.touches(),
            buttons: sample.buttons(),
        }
    }
}

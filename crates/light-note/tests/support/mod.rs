//! Shared fixtures for the core test suite.
//!
//! Everything here is plain data: no WinUI, no window, no pdfium requirement.
//! The suite is the specification for the core, so the fixtures stay as close to
//! the real inputs as possible — in particular [`Shm`] writes the shared-memory
//! layout exactly the way the OTD plugin writes it (`OTD.SharedMemoryOutput/
//! PROTOCOL.md`, v1), marker order included.

#![allow(dead_code)]

use light_note::geom::{Pt, Size};
use light_note::ink::{InkPoint, Nib, Stroke, Style, Tool};
use light_note::otd::{CAPACITY, HEADER_LEN, MAGIC, SAMPLE_LEN, VERSION};

/// A4 in points — the default page of a blank note.
pub const A4: Size = Size::new(595.276, 841.89);

/// A pen with the given width in points.
pub fn pen(width_pt: f32) -> Style {
    Style::new(Tool::Pen, width_pt)
}

/// A highlighter with the given width in points.
pub fn highlighter(width_pt: f32) -> Style {
    Style::new(Tool::Highlighter, width_pt)
}

/// A stroke built from `(x, y, pressure)` triples; time advances by 2.5 ms
/// (400 Hz) so speed-derived pressure is exercised.
pub fn stroke(style: Style, points: &[(f32, f32, f32)]) -> Stroke {
    let mut stroke = Stroke::new(style);
    for (index, (x, y, pressure)) in points.iter().enumerate() {
        stroke.push(InkPoint::new(
            Pt::new(*x, *y),
            *pressure,
            index as f64 * 2.5,
        ));
    }
    stroke
}

/// The same, with a tilted and rotated pen: the three angles every point of the
/// stroke carries.
pub fn stroke_with_nib(
    style: Style,
    nib: Nib,
    points: &[(f32, f32, f32)],
) -> Stroke {
    let mut stroke = Stroke::new(style);
    for (index, (x, y, pressure)) in points.iter().enumerate() {
        stroke.push(InkPoint::with_nib(
            Pt::new(*x, *y),
            *pressure,
            nib,
            index as f64 * 2.5,
        ));
    }
    stroke
}

/// A straight horizontal line from `x0` to `x1` at `y`, sampled every 2 pt.
pub fn line(style: Style, x0: f32, x1: f32, y: f32, pressure: f32) -> Stroke {
    let steps = ((x1 - x0).abs() / 2.0).ceil().max(1.0) as usize;
    let points: Vec<(f32, f32, f32)> = (0..=steps)
        .map(|step| {
            let t = step as f32 / steps as f32;
            (x0 + (x1 - x0) * t, y, pressure)
        })
        .collect();
    stroke(style, &points)
}

/// A byte buffer with the shared-memory layout (`128 B header + 4096 * 64 B`).
pub struct Shm {
    pub bytes: Vec<u8>,
}

impl Shm {
    /// A mapping that looks like a live plugin: magic, version and flags set.
    pub fn live(max_x: f32, max_y: f32, max_pressure: f32) -> Self {
        let mut shm = Self {
            bytes: vec![0; HEADER_LEN + CAPACITY * SAMPLE_LEN],
        };
        shm.put_u64(0x00, MAGIC);
        shm.put_u32(0x08, VERSION);
        shm.put_u32(0x0C, SAMPLE_LEN as u32);
        shm.put_u32(0x10, CAPACITY as u32);
        shm.put_u32(0x14, 1); // bit0 = plugin alive
        shm.put_u64(0x18, 0); // write_seq
        shm.put_f32(0x28, max_x);
        shm.put_f32(0x2C, max_y);
        shm.put_f32(0x30, max_pressure);
        shm.put_u64(0x38, 1); // heartbeat
        shm.name("Wacom Intuos Pro M");
        shm
    }

    /// The plugin is gone: the header still reads v1 but bit0 is clear.
    pub fn stale() -> Self {
        let mut shm = Self::live(0.0, 0.0, 0.0);
        shm.put_u32(0x14, 0);
        shm
    }

    /// A mapping written by an older/other plugin version.
    pub fn wrong_version(version: u32) -> Self {
        let mut shm = Self::live(0.0, 0.0, 0.0);
        shm.put_u32(0x08, version);
        shm
    }

    pub fn put_u32(&mut self, offset: usize, value: u32) {
        self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    pub fn put_u64(&mut self, offset: usize, value: u64) {
        self.bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    pub fn put_f32(&mut self, offset: usize, value: f32) {
        self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    pub fn name(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(63);
        self.bytes[0x40..0x40 + 64].fill(0);
        self.bytes[0x40..0x40 + len].copy_from_slice(&bytes[..len]);
    }

    /// Advances `write_seq` and records one sample, following the writer's
    /// marker discipline: `2 * seq + 1` (being written) then `2 * seq` (done).
    pub fn write_sample(&mut self, x: f32, y: f32, pressure: f32, flags: u32) -> u64 {
        let seq = self.u64(0x18) + 1;
        self.put_u64(0x18, seq);
        let slot = HEADER_LEN + (seq as usize % CAPACITY) * SAMPLE_LEN;
        self.put_u64(slot, seq * 2 + 1);
        self.put_f32(slot + 0x08, x);
        self.put_f32(slot + 0x0C, y);
        self.put_f32(slot + 0x10, pressure);
        self.put_f32(slot + 0x14, 0.0); // tilt_x
        self.put_f32(slot + 0x18, 0.0); // tilt_y
        self.put_f32(slot + 0x1C, 0.0); // rotation
        self.put_u32(slot + 0x20, flags);
        self.put_u32(slot + 0x24, 0); // hover distance
        self.put_u64(slot + 0x28, seq * 100); // QPC timestamp
        self.put_u64(slot, seq * 2);
        seq
    }

    /// Leaves a slot half-written (marker `2 * seq + 1`): a reader must not
    /// adopt it, because the fields are still moving.
    pub fn write_torn_sample(&mut self, x: f32, y: f32, pressure: f32, flags: u32) {
        let seq = self.u64(0x18) + 1;
        self.put_u64(0x18, seq);
        let slot = HEADER_LEN + (seq as usize % CAPACITY) * SAMPLE_LEN;
        self.put_u64(slot, seq * 2 + 1);
        self.put_f32(slot + 0x08, x);
        self.put_f32(slot + 0x0C, y);
        self.put_f32(slot + 0x10, pressure);
        self.put_u32(slot + 0x20, flags);
    }

    /// The sample currently stored in slot `index` (0-based within the ring).
    pub fn slot(&self, index: usize) -> usize {
        HEADER_LEN + (index % CAPACITY) * SAMPLE_LEN
    }

    pub fn u64(&self, offset: usize) -> u64 {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&self.bytes[offset..offset + 8]);
        u64::from_le_bytes(raw)
    }
}

/// Flag bits of a sample (`PROTOCOL.md` v1).
pub mod flag {
    pub const TIP: u32 = 1 << 0;
    pub const ERASER: u32 = 1 << 1;
    pub const OUT_OF_RANGE: u32 = 1 << 2;
    pub const PROXIMITY: u32 = 1 << 3;
    pub const BUTTON_1: u32 = 1 << 4;
}

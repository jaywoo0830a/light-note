//! The reader side of the OTD shared-memory contract (v1).
//!
//! `OTD.SharedMemoryOutput/PROTOCOL.md` is the contract; this module is the only
//! place that knows its byte offsets (header 128 B, then 4096 slots of 64 B,
//! `write_seq` at 0x18 as the reader's only cursor).
//!
//! Two rules make this lock-free and safe:
//!
//! 1. a sample is adopted **only** when its slot marker equals `2 * seq` exactly
//!    — a half-written sample, or one whose slot was overwritten meanwhile, is
//!    skipped;
//! 2. the reader keeps its own cursor and reads the window
//!    `max(cursor + 1, latest - capacity + 1) ..= latest`.  Samples that fell out
//!    of the ring are counted in [`ReadStats::skipped`] and shown in the UI — a
//!    real number beats a silently shorter line.
//!
//! This module is pure: bytes in, values out.  The mapping and the thread that
//! polls it are Windows-only and live in [`super::reader`].

use super::spec::TabletSpec;

/// The mapping name the plugin creates (in the session namespace).
pub const MAP_NAME: &str = "light-note.otd.shm";

/// `"LNOTDSM1"` — distinguishes this mapping from any other.
pub const MAGIC: u64 = 0x4C4E_4F54_4453_4D31;
/// The protocol version this reader understands.
pub const VERSION: u32 = 1;
/// Bytes per sample.
pub const SAMPLE_LEN: usize = 64;
/// Samples in the ring (about 10 s at 400 Hz).
pub const CAPACITY: usize = 4096;
/// Bytes before the first sample.
pub const HEADER_LEN: usize = 128;
/// The whole mapping.
pub const TOTAL_LEN: usize = HEADER_LEN + CAPACITY * SAMPLE_LEN;

const OFF_WRITE_SEQ: usize = 0x18;
const OFF_HEARTBEAT: usize = 0x38;
const OFF_TABLET_NAME: usize = 0x40;
const SLOT_SEQ: usize = 0x00;
const SLOT_X: usize = 0x08;
const SLOT_Y: usize = 0x0C;
const SLOT_PRESSURE: usize = 0x10;
const SLOT_TILT_X: usize = 0x14;
const SLOT_TILT_Y: usize = 0x18;
const SLOT_ROTATION: usize = 0x1C;
const SLOT_FLAGS: usize = 0x20;
const SLOT_HOVER: usize = 0x24;
const SLOT_TIME: usize = 0x28;

const FLAG_TIP: u32 = 1 << 0;
const FLAG_ERASER: u32 = 1 << 1;
const FLAG_OUT_OF_RANGE: u32 = 1 << 2;
const BUTTON_SHIFT: u32 = 4;

/// Why a mapping could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ShmError {
    /// The buffer is smaller than one header.
    #[error("the mapping is smaller than one header")]
    TooSmall,
    /// Something else is mapped under this name.
    #[error("the mapping is not a light-note OTD mapping (bad magic)")]
    BadMagic,
    /// The plugin speaks a version this app does not know — treated as "no
    /// plugin", never parsed optimistically.
    #[error("unsupported protocol version {found} (this app reads {VERSION})")]
    BadVersion { found: u32 },
    /// The sample size does not match, so every offset would be wrong.
    #[error("unexpected sample size {found} (this app reads {SAMPLE_LEN})")]
    BadSampleSize { found: u32 },
}

/// The mapping's header.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub magic: u64,
    pub version: u32,
    pub sample_size: u32,
    pub capacity: u32,
    pub flags: u32,
    pub write_seq: u64,
    pub max_x: f32,
    pub max_y: f32,
    pub max_pressure: f32,
    pub heartbeat: u64,
    pub tablet_name: String,
}

impl Header {
    /// Is the plugin still running?  (bit0 of `flags`)
    pub fn is_alive(&self) -> bool {
        self.flags & 1 == 1
    }

    /// A header this app can read.
    pub fn is_readable(&self) -> bool {
        self.magic == MAGIC && self.version == VERSION && self.sample_size == SAMPLE_LEN as u32
    }
}

/// One pen report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub seq: u64,
    /// Tablet X in device units — the pen's position *on the tablet*, which the
    /// app does not use for placement (the canvas's pointer event says where the
    /// pen is on the page).
    pub x: f32,
    pub y: f32,
    /// Raw pressure in device units (`0` = not touching).
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub rotation: f32,
    pub flags: u32,
    pub hover_distance: u32,
    /// `Stopwatch.GetTimestamp()` (QPC, 100 ns units) as the plugin read it.
    pub time: u64,
}

impl Sample {
    /// Is the tip touching the tablet?
    pub fn touches(&self) -> bool {
        self.flags & FLAG_TIP != 0
    }

    /// Is this the pen's eraser end?
    pub fn is_eraser(&self) -> bool {
        self.flags & FLAG_ERASER != 0
    }

    /// Did the pen leave the tablet's range?
    pub fn out_of_range(&self) -> bool {
        self.flags & FLAG_OUT_OF_RANGE != 0
    }

    /// Pen buttons (bits 4..=11).
    pub fn buttons(&self) -> u8 {
        ((self.flags >> BUTTON_SHIFT) & 0xFF) as u8
    }
}

/// Reads the header, refusing anything this app does not understand.
pub fn decode_header(bytes: &[u8]) -> Result<Header, ShmError> {
    if bytes.len() < HEADER_LEN {
        return Err(ShmError::TooSmall);
    }
    let magic = read_u64(bytes, 0x00);
    if magic != MAGIC {
        return Err(ShmError::BadMagic);
    }
    let version = read_u32(bytes, 0x08);
    if version != VERSION {
        return Err(ShmError::BadVersion { found: version });
    }
    let sample_size = read_u32(bytes, 0x0C);
    if sample_size != SAMPLE_LEN as u32 {
        return Err(ShmError::BadSampleSize { found: sample_size });
    }
    Ok(Header {
        magic,
        version,
        sample_size,
        capacity: read_u32(bytes, 0x10),
        flags: read_u32(bytes, 0x14),
        write_seq: read_u64(bytes, OFF_WRITE_SEQ),
        max_x: read_f32(bytes, 0x28),
        max_y: read_f32(bytes, 0x2C),
        max_pressure: read_f32(bytes, 0x30),
        heartbeat: read_u64(bytes, OFF_HEARTBEAT),
        tablet_name: read_name(bytes),
    })
}

/// Writes the tablet's range and name into a header.
///
/// The plugin **cannot** know them: a `PreTransform` filter never sees
/// `IOutputMode.Tablet`, so it leaves `max_*` at zero and the app fills them in
/// from the daemon's `GetTablets` (`PROTOCOL.md`).  Without a range a sample is
/// just a pair of device units — which is what "the tablet is not detected" means
/// from the outside.
///
/// Only the fields the plugin does not own are touched: its magic, version, sizes,
/// cursor, heartbeat and samples are left exactly as they are, because a reader
/// that saw a rewritten `magic` or `write_seq` would drop the whole stream.
///
/// Returns `false` when `bytes` is too short to hold a header (nothing is written).
pub fn fill_tablet(bytes: &mut [u8], spec: &TabletSpec) -> bool {
    if bytes.len() < HEADER_LEN {
        return false;
    }
    write_f32(bytes, 0x28, spec.max_x);
    write_f32(bytes, 0x2C, spec.max_y);
    write_f32(bytes, 0x30, spec.max_pressure);
    bytes[OFF_TABLET_NAME..OFF_TABLET_NAME + 64].fill(0);
    // The name is NUL-terminated UTF-8 in a 64-byte field: truncation must not
    // split a character, or a reader would see a replacement glyph.
    let name = spec.name.as_bytes();
    let mut len = name.len().min(63);
    while len > 0 && !spec.name.is_char_boundary(len) {
        len -= 1;
    }
    bytes[OFF_TABLET_NAME..OFF_TABLET_NAME + len].copy_from_slice(&name[..len]);
    true
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Reads sample `seq`, or `None` when its slot does not currently hold it.
pub fn decode_sample(bytes: &[u8], seq: u64) -> Option<Sample> {
    if seq == 0 {
        return None;
    }
    let slot = HEADER_LEN + (seq as usize % CAPACITY) * SAMPLE_LEN;
    if slot + SAMPLE_LEN > bytes.len() {
        return None;
    }
    // Rule 1: the marker must match exactly.  `2 * seq` means "complete",
    // `2 * seq + 1` means "being written", and anything else means the slot has
    // been reused by a newer sample since the cursor was read.
    if read_u64(bytes, slot + SLOT_SEQ) != seq * 2 {
        return None;
    }
    Some(Sample {
        seq,
        x: read_f32(bytes, slot + SLOT_X),
        y: read_f32(bytes, slot + SLOT_Y),
        pressure: read_f32(bytes, slot + SLOT_PRESSURE),
        tilt_x: read_f32(bytes, slot + SLOT_TILT_X),
        tilt_y: read_f32(bytes, slot + SLOT_TILT_Y),
        rotation: read_f32(bytes, slot + SLOT_ROTATION),
        flags: read_u32(bytes, slot + SLOT_FLAGS),
        hover_distance: read_u32(bytes, slot + SLOT_HOVER),
        time: read_u64(bytes, slot + SLOT_TIME),
    })
}

/// What one [`RingReader::read_new`] call did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadStats {
    /// Samples decoded (the ones a stroke can use).
    pub read: usize,
    /// Samples that were overwritten before this reader saw them.
    pub skipped: u64,
}

/// Follows `write_seq` and hands out the samples that are still in the ring.
#[derive(Debug, Default)]
pub struct RingReader {
    cursor: u64,
    skipped: u64,
}

impl RingReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// The newest sample this reader has consumed.
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// Total samples lost to ring overflow since the app started.
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Decodes every new sample into `out`.
    ///
    /// `latest` is the `write_seq` the caller read from the header.  A cursor
    /// that would move backwards (the plugin restarted) is ignored.
    pub fn read_new(&mut self, bytes: &[u8], latest: u64, out: &mut Vec<Sample>) -> ReadStats {
        if latest <= self.cursor {
            return ReadStats::default();
        }
        let window_start = latest.saturating_sub(CAPACITY as u64 - 1);
        let from = (self.cursor + 1).max(window_start);
        let skipped = from - (self.cursor + 1);
        let before = out.len();
        for seq in from..=latest {
            if let Some(sample) = decode_sample(bytes, seq) {
                out.push(sample);
            }
        }
        self.cursor = latest;
        self.skipped += skipped;
        ReadStats {
            read: out.len() - before,
            skipped,
        }
    }

    /// Forgets the cursor (used when the plugin disappears and comes back).
    pub fn reset(&mut self) {
        self.cursor = 0;
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let mut raw = [0u8; 4];
    raw.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_le_bytes(raw)
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(raw)
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_bits(read_u32(bytes, offset))
}

fn read_name(bytes: &[u8]) -> String {
    let raw = &bytes[OFF_TABLET_NAME..OFF_TABLET_NAME + 64];
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}
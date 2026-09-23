//! OpenTabletDriver input: the shared-memory protocol and the thread that polls
//! it.
//!
//! ```text
//! OTD daemon ──► plugin (C#) ──► shared memory ──► reader thread ──► Inbox ──► UI
//!                 OTD.SharedMemoryOutput/        reader.rs      (no waiting)
//! ```
//!
//! The reader thread does the waiting (a 1 ms poll) so the UI thread does not
//! have to: by the time a batch reaches `Component::update`, everything in it is
//! already decoded.  A batch is `Send` and small — a handful of samples plus the
//! tablet's range, which the plugin only knows after it has seen the device.
//!
//! # What the stream is used for
//!
//! **Attributes, not position.**  The pen's pressure, tilt, barrel rotation and
//! eraser flag are all here (`PROTOCOL.md`), and none of them exist in WinUI's
//! pointer event — so they come from OTD.  The *position*, on the other hand,
//! comes from the canvas's own pointer event: that is the point on the screen
//! the pen is actually touching, so the ink cannot drift away from the cursor
//! when the window is resized, the page is zoomed or the tablet's active area
//! does not match the page's aspect ratio.  The tablet's coordinates are still
//! decoded (they are part of the contract) but nothing draws from them.

pub mod reader;
pub mod rpc;
pub mod shm;
pub mod spec;
pub mod tablets;

pub use spec::{PenState, TabletSpec};
pub use shm::{
    CAPACITY, HEADER_LEN, Header, MAP_NAME, MAGIC, ReadStats, RingReader, SAMPLE_LEN, Sample,
    ShmError, TOTAL_LEN, VERSION, decode_header, decode_sample, fill_tablet,
};

/// How often the reader thread looks at `write_seq`.
///
/// A 400 Hz tablet produces a sample every 2.5 ms, so 1 ms keeps the extra
/// latency under one sample and costs one atomic load per poll.
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1);

/// How old the newest report may be and still describe the pen a pointer sample
/// is drawn with.
///
/// This is the budget for everything between the plugin writing a report and the
/// app drawing with it (the reader's 1 ms poll, the inbox hop, one frame).  Past
/// it the attributes are stale — the plugin stopped, the cable came out — and
/// the app draws with its fallback instead of a frozen pressure.
pub const ATTRIBUTE_FRESH_MS: f32 = 50.0;

/// Where the input is coming from — what the status bar shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// No mapping: the plugin is not installed, or the daemon is not running.
    NoPlugin(String),
    /// The mapping is there but the plugin does not know the tablet yet.
    NoTablet,
    /// Pen reports are flowing.
    Live { tablet: String, samples_per_second: f32 },
}

impl Source {
    /// A short line for the status bar.
    pub fn summary(&self) -> String {
        match self {
            Self::NoPlugin(reason) => format!("no tablet ({reason})"),
            Self::NoTablet => {
                "tablet: the plugin is running, but OTD reports no tablet range".to_string()
            }
            Self::Live {
                tablet,
                samples_per_second,
            } => format!("OTD shared memory — {tablet} ({samples_per_second:.0} Hz)"),
        }
    }
}

/// One delivery from the reader thread.
#[derive(Clone, Debug)]
pub struct Batch {
    /// Samples that arrived since the last batch (in order).
    pub samples: Vec<Sample>,
    /// The tablet's range, when it became known (or changed).
    pub spec: Option<TabletSpec>,
    /// Where the input stands right now.
    pub source: Source,
    /// Counters since the app started.
    pub stats: ReadStats,
    /// The plugin's heartbeat (`Stopwatch` ticks) — proves the plugin is alive.
    pub heartbeat: u64,
    /// Why the input is not usable yet (the range is missing), if that is the
    /// case.  A fact the status bar can show instead of "waiting".
    pub problem: Option<String>,
}

impl Batch {
    /// Nothing to draw and nothing to report.
    pub fn is_quiet(&self) -> bool {
        self.samples.is_empty() && self.spec.is_none()
    }
}

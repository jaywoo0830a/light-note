//! OpenTabletDriver input: the shared-memory protocol, the tablet mapping and
//! the thread that polls them.
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

pub mod map;
pub mod reader;
pub mod shm;

pub use map::{PageMap, TabletSpec};
pub use shm::{
    CAPACITY, HEADER_LEN, Header, MAP_NAME, MAGIC, ReadStats, RingReader, SAMPLE_LEN, Sample,
    ShmError, TOTAL_LEN, VERSION, decode_header, decode_sample,
};

/// How often the reader thread looks at `write_seq`.
///
/// A 400 Hz tablet produces a sample every 2.5 ms, so 1 ms keeps the extra
/// latency under one sample and costs one atomic load per poll.
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1);

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
            Self::NoTablet => "tablet: waiting for the plugin".to_string(),
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
}

impl Batch {
    /// Nothing to draw and nothing to report.
    pub fn is_quiet(&self) -> bool {
        self.samples.is_empty() && self.spec.is_none()
    }
}

/// How long a gap between two samples means "this is a new stroke".
///
/// After a stall the ring still holds old samples; drawing them as one stroke
/// would put a long straight line across the page.  The app compares timestamps
/// (100 ns QPC ticks) and starts a new stroke instead.
pub const STALE_GAP_TICKS: u64 = 50 * 10_000; // 50 ms

//! Windows-only: map the plugin's shared memory and poll it on its own thread.
//!
//! The mapping is created by the C# plugin with `MemoryMappedFile.CreateOrOpen`,
//! which puts the name in the **session namespace**, so `Local\light-note.otd.shm`
//! is tried first and the bare name second (`PROTOCOL.md`).
//!
//! The thread owns the mapping and never touches the UI: it pushes [`Batch`]es
//! into an [`Inbox`] and the UI picks them up.  Polling at [`POLL_INTERVAL`]
//! costs one atomic load per poll; the plugin's writer never waits for us.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Memory::{
    FILE_MAP, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    OpenFileMappingW, UnmapViewOfFile,
};
use windows::core::PCWSTR;

use super::shm::{MAP_NAME, ShmError, TOTAL_LEN};
use super::{Batch, POLL_INTERVAL, RingReader, Source, TabletSpec, fill_tablet};

/// How soon the tablet range is asked for again after a failure, and how far that
/// wait grows.  A daemon that is not running must not be polled every second.
const RANGE_RETRY_MIN: Duration = Duration::from_millis(1000);
const RANGE_RETRY_MAX: Duration = Duration::from_millis(8000);
/// How often a *known* range is refreshed — the user may switch tablets.
const RANGE_REFRESH: Duration = Duration::from_secs(60);

/// A read-only view of the plugin's ring buffer.
pub struct Mapping {
    file: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    len: usize,
}

// A read-only view of shared memory; moving the handle to the reader thread is
// exactly what it is for.
unsafe impl Send for Mapping {}

impl Mapping {
    /// Opens the mapping read-only — the sample path.
    pub fn open() -> Result<Self, ShmError> {
        Self::open_with(FILE_MAP_READ)
    }

    /// Opens the mapping for writing: the tablet-range fields are the app's.
    ///
    /// The plugin cannot know the range (`PROTOCOL.md`), so the app fills it in
    /// from the daemon's `GetTablets`.  This is a second view, used only when the
    /// header still says "range unknown".
    pub fn open_writable() -> Result<Self, ShmError> {
        Self::open_with(FILE_MAP_WRITE)
    }

    fn open_with(access: FILE_MAP) -> Result<Self, ShmError> {
        for name in [format!("Local\\{MAP_NAME}"), MAP_NAME.to_string()] {
            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let Ok(file) =
                (unsafe { OpenFileMappingW(access.0, false, PCWSTR(wide.as_ptr())) })
            else {
                continue;
            };
            let view = unsafe { MapViewOfFile(file, access, 0, 0, TOTAL_LEN) };
            if view.Value.is_null() {
                let _ = unsafe { CloseHandle(file) };
                continue;
            }
            return Ok(Self {
                file,
                view,
                len: TOTAL_LEN,
            });
        }
        Err(ShmError::BadMagic)
    }

    /// The mapping's bytes (header + ring).
    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.view.Value as *const u8, self.len) }
    }

    /// The same bytes, writable — only for the fields the app owns.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.view.Value as *mut u8, self.len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        let _ = unsafe { UnmapViewOfFile(self.view) };
        let _ = unsafe { CloseHandle(self.file) };
    }
}

/// Starts the reader thread.  It runs until the process ends; `rescan` asks it to
/// drop its cursor and look for the mapping again (the OTD daemon may have been
/// started after the app).
///
/// `sink` is how a batch leaves this thread.  The reader does not know what is on
/// the other end — the app hands it the UI's inbox — which keeps the protocol
/// layer free of UI types (and keeps the reader testable with a plain `Vec`).
pub fn spawn(sink: impl Fn(Batch) + Send + 'static, rescan: Arc<AtomicBool>) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("otd-reader".to_string())
        .spawn(move || {
            // The range question lives on its own thread: a pipe read blocks until
            // the daemon answers, and *this* thread is where the pen's latency is.
            let (answers, received) = mpsc::channel();
            let (kick, kicks) = mpsc::channel();
            let _ = std::thread::Builder::new()
                .name("otd-range".to_string())
                .spawn(move || range_worker(answers, kicks));
            run(sink, rescan, received, kick)
        })
        .expect("the reader thread must start")
}

fn run(
    sink: impl Fn(Batch),
    rescan: Arc<AtomicBool>,
    answers: Receiver<Result<TabletSpec, String>>,
    kick: Sender<()>,
) {
    let mut reader = RingReader::new();
    let mut samples: Vec<super::Sample> = Vec::with_capacity(256);
    let mut spec: Option<TabletSpec> = None;
    let mut source = Source::NoPlugin("not started".to_string());
    let mut rate = RateWindow::new();
    // What the daemon said about the tablet, and why it could not be asked.
    let mut range: Option<TabletSpec> = None;
    let mut problem: Option<String> = None;
    let mut writer = HeaderWriter::new();

    loop {
        if rescan.swap(false, Ordering::Relaxed) {
            reader.reset();
            spec = None;
            source = Source::NoPlugin("rescanning".to_string());
            // The user asked to look again: ask the daemon for the range too.
            let _ = kick.send(());
        }

        // The range thread answers when it can; this loop never waits for it.
        collect(&answers, &mut range, &mut problem);
        let Ok(mapping) = Mapping::open() else {
            // No plugin: report it once, then keep looking — the daemon may be
            // started later, and the user should not have to restart the app.
            publish(
                &sink,
                &mut source,
                Source::NoPlugin("OTD plugin not found".to_string()),
                &mut spec,
            );
            reader.reset();
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };

        loop {
            // Both of these must happen *inside* this loop: the mapping stays
            // open for as long as it is readable, so the outer loop may not run
            // again for a long time.
            collect(&answers, &mut range, &mut problem);
            if rescan.swap(false, Ordering::Relaxed) {
                reader.reset();
                spec = None;
                source = Source::NoPlugin("rescanning".to_string());
                let _ = kick.send(());
                break; // look for the mapping again
            }

            let bytes = mapping.bytes();
            match super::decode_header(bytes) {
                Err(error) => {
                    publish(
                        &sink,
                        &mut source,
                        Source::NoPlugin(error.to_string()),
                        &mut spec,
                    );
                    reader.reset();
                    break;
                }
                Ok(header) => {
                    let stats = reader.read_new(bytes, header.write_seq, &mut samples);
                    // The plugin leaves the range at 0 (a `PreTransform` filter
                    // cannot see the tablet), so what the daemon told us *is* the
                    // range — and the header gets it as well, so the mapping
                    // describes itself (`PROTOCOL.md`).
                    let mut live = TabletSpec::new(
                        header.tablet_name.clone(),
                        header.max_x,
                        header.max_y,
                        header.max_pressure,
                    );
                    if !live.is_known()
                        && let Some(known) = &range
                    {
                        live = known.clone();
                        let _ = writer.fill(&live);
                    }
                    let new_spec = live.is_known() && spec.as_ref() != Some(&live);
                    if new_spec {
                        spec = Some(live.clone());
                    }
                    let hz = rate.add(samples.len());
                    let now = if spec.is_some() {
                        Source::Live {
                            tablet: spec
                                .as_ref()
                                .map(|spec| spec.name.clone())
                                .unwrap_or_default(),
                            samples_per_second: hz,
                        }
                    } else {
                        Source::NoTablet
                    };

                    if !samples.is_empty() || new_spec || now != source {
                        sink(Batch {
                            samples: std::mem::take(&mut samples),
                            spec: new_spec.then_some(live),
                            source: now.clone(),
                            stats,
                            heartbeat: header.heartbeat,
                            // Only worth saying while the range is still missing:
                            // that is the one thing that stops the pen drawing.
                            problem: if spec.is_none() { problem.clone() } else { None },
                        });
                        source = now;
                    } else {
                        samples.clear();
                    }
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

/// Takes whatever the range thread has produced, without ever waiting.
///
/// The reader calls this on every poll: the answer is not a message the UI
/// asked for, it is a value that may or may not be there yet.
fn collect(
    answers: &Receiver<Result<TabletSpec, String>>,
    range: &mut Option<TabletSpec>,
    problem: &mut Option<String>,
) {
    while let Ok(answer) = answers.try_recv() {
        match answer {
            Ok(known) => {
                *range = Some(known);
                *problem = None;
            }
            Err(reason) => *problem = Some(reason),
        }
    }
}

/// Writes the app's half of the header: the tablet range the plugin cannot know.
///
/// The writable view is opened **at most once** and only when the header is
/// missing its range, so the sample path stays a read-only mapping.  A mapping
/// that cannot be written (or a plugin restart that zeroes the header) must not
/// turn into an open-per-poll loop — `failed` remembers the first refusal.
struct HeaderWriter {
    mapping: Option<Mapping>,
    failed: bool,
}

impl HeaderWriter {
    fn new() -> Self {
        Self {
            mapping: None,
            failed: false,
        }
    }

    /// Puts `spec` into the header when the plugin left it empty.
    fn fill(&mut self, spec: &TabletSpec) -> bool {
        if self.failed {
            return false;
        }
        if self.mapping.is_none() {
            match Mapping::open_writable() {
                Ok(mapping) => self.mapping = Some(mapping),
                Err(_) => {
                    self.failed = true;
                    return false;
                }
            }
        }
        let Some(mapping) = self.mapping.as_mut() else {
            return false;
        };
        let bytes = mapping.bytes_mut();
        // A plugin restart recreates the mapping and zeroes the range: fill it
        // again, but do not rewrite a header that already carries it.
        let empty = super::decode_header(bytes)
            .map(|header| !header.max_x.is_finite() || header.max_x <= 0.0)
            .unwrap_or(false);
        if !empty {
            return true;
        }
        fill_tablet(bytes, spec)
    }
}

/// Asks the daemon for the tablet range, forever — on its own thread.
///
/// One question at a time: after an answer it waits (a known range is refreshed
/// rarely, because the user may switch tablets), after a failure it waits longer
/// each time (1 → 2 → 4 → 8 s), and a `kick` from the reader cuts the wait short
/// when the user presses "rescan".
fn range_worker(
    answers: Sender<Result<TabletSpec, String>>,
    kicks: Receiver<()>,
) {
    let mut backoff = RANGE_RETRY_MIN;
    loop {
        let result = super::rpc::tablet_spec();
        let wait = match &result {
            Ok(_) => {
                backoff = RANGE_RETRY_MIN;
                RANGE_REFRESH
            }
            Err(_) => {
                let wait = backoff;
                backoff = (backoff * 2).min(RANGE_RETRY_MAX);
                wait
            }
        };
        if answers.send(result).is_err() {
            return; // the app is gone
        }
        match kicks.recv_timeout(wait) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Posts a status change (never twice in a row).
fn publish(
    sink: &impl Fn(Batch),
    source: &mut Source,
    next: Source,
    spec: &mut Option<TabletSpec>,
) {
    if *source == next {
        return;
    }
    sink(Batch {
        samples: Vec::new(),
        spec: None,
        source: next.clone(),
        stats: Default::default(),
        heartbeat: 0,
        problem: None,
    });
    *source = next;
    *spec = None;
}

/// A half-second sample-rate window (the number the status bar shows).
struct RateWindow {
    samples: usize,
    since: Instant,
    rate: f32,
}

impl RateWindow {
    fn new() -> Self {
        Self {
            samples: 0,
            since: Instant::now(),
            rate: 0.0,
        }
    }

    fn add(&mut self, count: usize) -> f32 {
        self.samples += count;
        let elapsed = self.since.elapsed();
        if elapsed >= Duration::from_millis(500) {
            self.rate = self.samples as f32 / elapsed.as_secs_f32();
            self.samples = 0;
            self.since = Instant::now();
        }
        self.rate
    }
}
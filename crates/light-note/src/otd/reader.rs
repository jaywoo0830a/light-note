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
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Memory::{
    FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile,
};
use windows::core::PCWSTR;

use super::shm::{MAP_NAME, ShmError, TOTAL_LEN};
use super::{Batch, POLL_INTERVAL, RingReader, Source, TabletSpec};

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
    /// Opens the mapping, or reports that it is not there.
    pub fn open() -> Result<Self, ShmError> {
        for name in [format!("Local\\{MAP_NAME}"), MAP_NAME.to_string()] {
            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let Ok(file) =
                (unsafe { OpenFileMappingW(FILE_MAP_READ.0, false, PCWSTR(wide.as_ptr())) })
            else {
                continue;
            };
            let view = unsafe { MapViewOfFile(file, FILE_MAP_READ, 0, 0, TOTAL_LEN) };
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
        .spawn(move || run(sink, rescan))
        .expect("the reader thread must start")
}

fn run(sink: impl Fn(Batch), rescan: Arc<AtomicBool>) {
    let mut reader = RingReader::new();
    let mut samples: Vec<super::Sample> = Vec::with_capacity(256);
    let mut spec: Option<TabletSpec> = None;
    let mut source = Source::NoPlugin("not started".to_string());
    let mut rate = RateWindow::new();

    loop {
        if rescan.swap(false, Ordering::Relaxed) {
            reader.reset();
            spec = None;
            source = Source::NoPlugin("rescanning".to_string());
        }
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
                    let live = TabletSpec::new(
                        header.tablet_name.clone(),
                        header.max_x,
                        header.max_y,
                        header.max_pressure,
                    );
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
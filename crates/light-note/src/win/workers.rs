//! Workers: the threads that do everything the UI thread must not.
//!
//! | worker | waits for | posts back |
//! |---|---|---|
//! | bake | a page snapshot (latest wins) | a PNG of the page |
//! | pdf | open / render / export requests | sizes, a page bitmap, saved bytes |
//! | ticker | one frame period (high-resolution timer) | a tick |
//! | dialogs | a dialog request | the chosen path |
//!
//! Every one of them pushes into the shared [`Inbox`]; none of them ever calls
//! the UI directly, and the UI never waits for any of them.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use anyhow::Context;

use crate::display::FrameClock;
use crate::doc::Page;
use crate::geom::Scale;
use crate::inbox::Inbox;
use crate::pdf::{PageInk, bind_pdfium};
use crate::shape;

use super::{HostMessage, OpenedPdf, Purpose};

/// A page to rasterize: everything the bake worker needs, and nothing else.
#[derive(Clone, Debug)]
pub struct BakeRequest {
    pub page: Page,
    pub scale: Scale,
    /// The PDF page bitmap to composite under the ink (already rendered).
    pub background: Option<Arc<vello_cpu::Pixmap>>,
    /// Bumped by the UI so a stale result can be ignored.
    pub generation: u64,
}

/// A baked page.
#[derive(Clone, Debug)]
pub struct Baked {
    pub png: Vec<u8>,
    pub generation: u64,
    pub elapsed_ms: f32,
    pub width: u16,
    pub height: u16,
}

/// A rendered background page (PDF → bitmap).
#[derive(Clone, Debug)]
pub struct Rendered {
    pub index: usize,
    pub scale: Scale,
    pub bitmap: Arc<vello_cpu::Pixmap>,
    pub elapsed_ms: f32,
}

/// What the PDF worker is asked to do.
#[derive(Clone, Debug)]
pub enum PdfJob {
    /// Open a document and measure it.
    Open(PathBuf),
    /// Render one page of the open document.
    Render { index: usize, scale: Scale },
    /// Write the ink into a copy and save it.
    Export { pages: Vec<PageInk>, path: PathBuf },
    /// Rasterize the ink over the background and save a PNG.
    ExportPng {
        page: Page,
        background: Option<Arc<vello_cpu::Pixmap>>,
        scale: Scale,
        path: PathBuf,
    },
}

/// What the dialog worker is asked to show.
#[derive(Clone, Debug)]
pub struct DialogJob {
    pub purpose: Purpose,
    pub suggested: Option<PathBuf>,
}

/// The handles the host keeps.  Dropping this stops every worker.
pub struct Workers {
    bake: Sender<BakeRequest>,
    pdf: Sender<PdfJob>,
    dialogs: Sender<DialogJob>,
    /// Used to retarget the ticker when the refresh rate changes.
    ticker: Sender<u32>,
}

impl Workers {
    /// Starts every worker thread.  `ui_inbox` is how they reach the UI thread.
    pub fn start(ui_inbox: Arc<Inbox<HostMessage>>, refresh_hz: u32) -> Self {
        let (bake_tx, bake_rx) = channel::<BakeRequest>();
        let (pdf_tx, pdf_rx) = channel::<PdfJob>();
        let (dialog_tx, dialog_rx) = channel::<DialogJob>();
        let (tick_tx, tick_rx) = channel::<u32>();

        spawn("light-note-bake", {
            let inbox = Arc::clone(&ui_inbox);
            move || bake_worker(bake_rx, inbox)
        });
        spawn("light-note-pdf", {
            let inbox = Arc::clone(&ui_inbox);
            move || pdf_worker(pdf_rx, inbox)
        });
        spawn("light-note-dialogs", {
            let inbox = Arc::clone(&ui_inbox);
            move || dialog_worker(dialog_rx, inbox)
        });
        spawn("light-note-ticker", {
            let inbox = Arc::clone(&ui_inbox);
            move || ticker_worker(tick_rx, refresh_hz, inbox)
        });

        Self {
            bake: bake_tx,
            pdf: pdf_tx,
            dialogs: dialog_tx,
            ticker: tick_tx,
        }
    }

    /// Queues a bake.  A newer request always wins (the worker drains the queue).
    pub fn bake(&self, request: BakeRequest) {
        let _ = self.bake.send(request);
    }

    pub fn pdf(&self, job: PdfJob) {
        let _ = self.pdf.send(job);
    }

    pub fn dialog(&self, job: DialogJob) {
        let _ = self.dialogs.send(job);
    }

    /// Changes the frame rate (60/120/180/240).
    pub fn set_refresh(&self, hz: u32) {
        let _ = self.ticker.send(hz);
    }
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(work);
}

/// Rasterizes pages: the only place in the app that encodes a PNG.
fn bake_worker(requests: Receiver<BakeRequest>, inbox: Arc<Inbox<HostMessage>>) {
    while let Ok(first) = requests.recv() {
        // Latest wins: whatever piled up while we were working is irrelevant —
        // only the newest page state is worth rasterizing.
        let mut request = first;
        while let Ok(newer) = requests.try_recv() {
            request = newer;
        }

        let started = Instant::now();
        // A page without a PDF background is white paper — and it is painted
        // here, on this thread, not on the UI thread.
        let background = request.background.clone().unwrap_or_else(|| {
            Arc::new(shape::white_page(request.page.size, request.scale))
        });
        let pixmap = shape::rasterize(
            &request.page.strokes,
            request.page.size,
            request.scale,
            Some(&background),
        );
        let (width, height) = (pixmap.width(), pixmap.height());
        match shape::to_png(&pixmap) {
            Ok(png) => inbox.push(HostMessage::Baked(Baked {
                png,
                generation: request.generation,
                elapsed_ms: started.elapsed().as_secs_f32() * 1000.0,
                width,
                height,
            })),
            Err(error) => inbox.push(HostMessage::Failed(error)),
        }
    }
}

/// Owns the Pdfium engine: bound once, on this thread, and used only here.
fn pdf_worker(jobs: Receiver<PdfJob>, inbox: Arc<Inbox<HostMessage>>) {
    let engine = bind_pdfium();
    let mut open: Option<Vec<u8>> = None;

    while let Ok(job) = jobs.recv() {
        let engine = match &engine {
            Ok(engine) => engine,
            Err(error) => {
                inbox.push(HostMessage::Failed(error.to_string()));
                continue;
            }
        };
        match job {
            PdfJob::Open(path) => {
                let result = std::fs::read(&path)
                    .context("could not read the PDF")
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| match engine.page_sizes(&bytes) {
                        Ok(pages) => Ok((bytes, pages)),
                        Err(error) => Err(error.to_string()),
                    });
                match result {
                    Ok((bytes, pages)) => {
                        open = Some(bytes.clone());
                        inbox.push(HostMessage::PdfOpened(Ok(OpenedPdf { path, bytes, pages })));
                    }
                    Err(error) => inbox.push(HostMessage::PdfOpened(Err(error))),
                }
            }
            PdfJob::Render { index, scale } => {
                let Some(bytes) = open.clone() else {
                    continue;
                };
                let started = Instant::now();
                match engine.render_page(&bytes, index, scale) {
                    Ok(bitmap) => inbox.push(HostMessage::PageRendered(Rendered {
                        index,
                        scale,
                        bitmap: Arc::new(bitmap),
                        elapsed_ms: started.elapsed().as_secs_f32() * 1000.0,
                    })),
                    Err(error) => inbox.push(HostMessage::Failed(error.to_string())),
                }
            }
            PdfJob::Export { pages, path } => {
                let Some(bytes) = open.clone() else {
                    inbox.push(HostMessage::Failed("no PDF is open".to_string()));
                    continue;
                };
                let result = engine
                    .annotate(&bytes, &pages)
                    .map_err(|error| error.to_string())
                    .and_then(|annotated| {
                        std::fs::write(&path, annotated)
                            .map(|_| path.clone())
                            .map_err(|error| error.to_string())
                    });
                inbox.push(HostMessage::Exported(result));
            }
            PdfJob::ExportPng {
                page,
                background,
                scale,
                path,
            } => {
                let pixmap = shape::rasterize(&page.strokes, page.size, scale, background.as_deref());
                let result = shape::to_png(&pixmap).and_then(|png| {
                    std::fs::write(&path, png)
                        .map(|_| path.clone())
                        .map_err(|error| error.to_string())
                });
                inbox.push(HostMessage::Exported(result));
            }
        }
    }
}

/// Shows native dialogs on its own thread: `IFileDialog` blocks, and the UI
/// thread must never block.
fn dialog_worker(jobs: Receiver<DialogJob>, inbox: Arc<Inbox<HostMessage>>) {
    while let Ok(job) = jobs.recv() {
        let dialog = rfd::FileDialog::new();
        let purpose = job.purpose;
        let path = match purpose {
            Purpose::OpenPdf => dialog.add_filter("PDF", &["pdf"]).pick_file(),
            Purpose::SavePdf => dialog
                .add_filter("PDF", &["pdf"])
                .set_file_name("light-note.pdf")
                .save_file(),
            Purpose::SavePng => dialog
                .add_filter("PNG", &["png"])
                .set_file_name("light-note.png")
                .save_file(),
        };
        inbox.push(HostMessage::PathChosen { purpose, path });
    }
}

/// The frame ticker: the app's heartbeat at the display's rate.
///
/// It exists so the loop keeps running even when nothing else happens (a note
/// with no pen input still has to show its frame rate), and so a refresh-rate
/// change takes effect without restarting anything.
fn ticker_worker(rates: Receiver<u32>, refresh_hz: u32, inbox: Arc<Inbox<HostMessage>>) {
    let mut clock = FrameClock::new(refresh_hz);
    loop {
        let deadline = clock.next();
        clock.wait_until(deadline);
        inbox.push(HostMessage::Tick);

        // A rate change arrives on this channel; `try_recv` keeps the frame
        // cadence intact (no blocking wait in the loop).
        match rates.try_recv() {
            Ok(hz) => clock.retarget(hz),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
}
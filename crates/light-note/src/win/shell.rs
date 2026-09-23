//! The host: the only place that knows about all four stages.
//!
//! ```text
//! HostMessage::Tablet(batch) → pen samples → Document (+ live stroke) → view
//! HostMessage::Baked(png)    → inactive buffer → (Decoded) → active buffer
//! HostMessage::Intent(..)    → document / tool / zoom / export requests
//! HostMessage::Tick          → frame meters, then re-arm the pump
//! ```
//!
//! ## What this thread never does
//!
//! No file I/O, no Pdfium, no rasterization, no PNG encoding, no dialog, no
//! sleep, no blocking lock.  It applies messages to values and builds a view.
//! The heavy work is on [`Workers`] and reaches this thread through
//! [`Inbox`], which is drained with `try_pop` — a call that cannot wait even in
//! principle.  [`Shell::busy_ms`] measures every message, and the status bar
//! shows the worst one next to the frame budget, so "the UI never blocks" is a
//! number on screen and not a promise.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use elm_magic::Callback;
use elm_magic_windows_reactor::{ElmInput, ElmView};
use windows_reactor::{
    Border, Brush, ChildrenControl, Color, Component, ComponentContext, ContentControl,
    DragDropAction, DragDropOperation, DragDropPolicy, DroppedData, HorizontalAlignment,
    LayoutControl, Orientation, StackPanel, Thickness, View, ViewContext, WindowBackdrop,
    WindowVisuals,
};

use super::screen::{Screen, ScreenProps};
use super::surface::{self, Buffer};
use super::workers::{BakeRequest, Baked, DialogJob, PdfJob, Workers};
use super::{HostMessage, Intent, OpenedPdf, PointerPhase, Purpose, ViewModel};
use crate::display::{FpsMeter, RefreshChoice, frame_period};
use crate::doc::Document;
use crate::geom::{Pt, Scale, Size};
use crate::inbox::Inbox;
use crate::ink::{InkPoint, Stroke, Style, Tool, pressure_from_speed, smooth_pressure};
use crate::otd::{self, Batch, PageMap, STALE_GAP_TICKS, Source, TabletSpec};
use crate::pdf::PageInk;
use crate::settings::Settings;

use std::sync::atomic::{AtomicBool, Ordering};

/// A4 in points — the paper a blank note uses.
const A4: Size = Size::new(595.276, 841.89);

/// How long to wait for WinUI's `ImageOpened` before showing a baked page anyway.
///
/// The decode callback is the *nice* path (it swaps the two buffers with no blank
/// frame); this is the guarantee that the page appears at all.
const DECODE_GRACE: Duration = Duration::from_millis(50);

/// How much of the window the toolbar, the status text and the panels take.
///
/// WinUI measures it, but it does not report the size back to us, so the page's
/// fit is computed against this estimate.  It is deliberately generous: too much
/// leaves a little empty desk, too little would push the toolbar off the window.
const SCREEN_HEIGHT: f64 = 360.0;

/// The desk's padding around the sheet.
const DESK_PADDING: f64 = 12.0;

/// The host component.
pub struct Shell {
    inbox: Arc<Inbox<HostMessage>>,
    workers: Workers,

    // ── model ──────────────────────────────────────────────────────────────
    document: Document,
    settings: Settings,
    tool: Tool,
    scale: Scale,
    zoom: f32,

    // ── input ──────────────────────────────────────────────────────────────
    map: Option<PageMap>,
    tablet: Option<TabletSpec>,
    source: Source,
    live: Option<Stroke>,
    erasing: bool,
    pressure: f32,
    last_sample_time: u64,
    last_pos: Pt,
    stroke_start: u64,
    rescan: Arc<AtomicBool>,
    samples: u64,
    skipped: u64,

    // ── screen ─────────────────────────────────────────────────────────────
    buffers: [Buffer; 2],
    active: usize,
    generation: u64,
    baking: Option<u64>,
    /// When the last bake landed — the deadline for WinUI's decode callback.
    baked_at: Option<Instant>,
    background: Option<Arc<vello_cpu::Pixmap>>,
    background_key: Option<(usize, u32)>,

    // ── pdf ────────────────────────────────────────────────────────────────
    pdf: Option<OpenedPdf>,

    // ── timing ─────────────────────────────────────────────────────────────
    refresh: RefreshChoice,
    detected_hz: u32,
    meter: FpsMeter,
    last_frame: Instant,
    /// The clock the mouse's samples are stamped with (see `Shell::pointer`).
    pointer_base: Instant,
    busy_ms: f32,
    worst_busy_ms: f32,
    bake_ms: f32,
    render_ms: f32,
    frame_count: u64,

    // ── ui state ───────────────────────────────────────────────────────────
    status: String,
    hint: String,
    help: bool,
    dev: bool,
    client: Size,
    pump_armed: bool,
}

impl Component for Shell {
    type Input = ();
    type Message = HostMessage;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        let inbox = Inbox::new();
        let detected_hz = crate::display::detect_refresh_hz();
        let settings = Settings::load();
        let refresh = settings.refresh;
        let hz = refresh.resolve(detected_hz);
        let workers = Workers::start(Arc::clone(&inbox), hz);

        // The reader thread owns the mapping; the UI only ever sees batches.
        // It reaches the UI the same way every worker does — by posting into the
        // inbox — so a pen report wakes the loop instead of waiting for a tick.
        let rescan = Arc::new(AtomicBool::new(false));
        let reader_inbox = Arc::clone(&inbox);
        otd::reader::spawn(
            move |batch: Batch| reader_inbox.push(HostMessage::Tablet(batch)),
            Arc::clone(&rescan),
        );

        let document = Document::blank(A4);
        let tool = settings.tool;
        let margin = settings.margin_pt;
        let mut shell = Self {
            inbox,
            workers,
            document,
            settings,
            tool,
            scale: Scale::DEFAULT,
            zoom: 100.0,
            map: None,
            tablet: None,
            source: Source::NoPlugin("starting".to_string()),
            live: None,
            erasing: false,
            pressure: 0.0,
            last_sample_time: 0,
            last_pos: Pt::new(0.0, 0.0),
            stroke_start: 0,
            rescan,
            samples: 0,
            skipped: 0,
            buffers: [Buffer::default(), Buffer::default()],
            active: 0,
            generation: 0,
            baking: None,
            baked_at: None,
            background: None,
            background_key: None,
            pdf: None,
            refresh,
            detected_hz,
            meter: FpsMeter::new(),
            last_frame: Instant::now(),
            pointer_base: Instant::now(),
            busy_ms: 0.0,
            worst_busy_ms: 0.0,
            bake_ms: 0.0,
            render_ms: 0.0,
            frame_count: 0,
            status: format!("{} Hz display · {} Hz loop", detected_hz.max(60), hz),
            hint: "A digitizer writes through OTD; the mouse writes only while no tablet \
                   is running. Shortcuts live on the buttons, and the tablet's eraser end \
                   erases."
                .to_string(),
            help: false,
            dev: false,
            client: Size::new(1180.0, 880.0),
            pump_armed: false,
        };
        shell.status = shell.startup_status();
        // No scrolling: the page is sized to the window from the start.
        shell.refit();
        shell.request_bake();
        shell.arm_pump(context);
        let _ = margin;
        shell
    }

    fn update(&mut self, message: HostMessage, context: &ComponentContext<Self>) {
        let started = Instant::now();
        match message {
            HostMessage::Wake => {
                self.pump_armed = false;
                self.drain();
            }
            HostMessage::Tick => self.tick(),
            other => self.apply(other),
        }
        self.busy_ms = started.elapsed().as_secs_f32() * 1000.0;
        self.worst_busy_ms = self.worst_busy_ms.max(self.busy_ms);
        if !self.pump_armed {
            self.arm_pump(context);
        }
        self.refresh();
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        let view_model = self.view_model();
        context.window_title("light-note");
        context.window_visuals(
            WindowVisuals::new()
                .client_size(self.client.width as f64, self.client.height as f64)
                .backdrop(WindowBackdrop::Mica),
        );

        let sender = context.sender();
        let on_decoded = {
            let sender = sender.clone();
            Rc::new(move |slot: usize| {
                let _ = sender.send(HostMessage::Decoded(slot));
            })
        };
        // The mouse (or a pen that WinUI sees but OTD does not): the fallback path.
        let on_pointer = {
            let sender = sender.clone();
            Rc::new(
                move |phase: PointerPhase, x: f64, y: f64, pressure: f32| {
                    let _ = sender.send(HostMessage::Pointer {
                        x,
                        y,
                        pressure,
                        phase,
                    });
                },
            )
        };

        let page_px = self.page_pixels();
        let page = surface::build(
            page_px,
            &self.buffers,
            self.active,
            self.live.as_ref(),
            self.scale,
            on_decoded,
            on_pointer,
        );

        // The elm screen: toolbar, status, help and the diagnostics panel.
        let on_intent = Callback::new({
            let sender = sender.clone();
            move |_arena, intent: Intent| {
                let _ = sender.send(HostMessage::Intent(intent));
            }
        });
        let screen = View::component::<ElmView<Screen>>(ElmInput::new(ScreenProps {
            title: Some(view_model.title.clone()),
            subtitle: Some(view_model.subtitle.clone()),
            tool_line: Some(view_model.tool_line.clone()),
            pages: Some(view_model.pages),
            page_index: Some(view_model.page_index + 1),
            strokes: Some(view_model.strokes),
            zoom: Some(view_model.zoom),
            tablet: Some(view_model.tablet.clone()),
            // The two meters the screen shows: the loop's rate and the worst
            // frame's cost, side by side (that is what "never blocks" means).
            frame: Some(format!("{} · {}", view_model.refresh, view_model.worst_frame)),
            hint: Some(view_model.hint.clone()),
            help: Some(view_model.help),
            dev: Some(view_model.dev),
            dev_report: Some(view_model.dev_report.clone()),
            on_intent: Some(on_intent),
            ..Default::default()
        }));

        // The desk: a surface that is clearly *not* the page, so the sheet reads as
        // paper instead of blending into the window.  It hugs the sheet, so the
        // window keeps its own backdrop around it.
        //
        // There is no scrolling.  The page is sized to fit the window at 100% zoom
        // (`Shell::refit`), so it is always fully visible; zooming out shrinks it,
        // and zooming in stops at the fit.  A `ScrollViewer` would let the page
        // move under the pen, which is the one thing a writing surface must not do.
        let desk = Border::new()
            .background(Brush::from(Color::rgb(0x1E, 0x22, 0x28)))
            .padding(Thickness::uniform(DESK_PADDING))
            .width(page_px.0 + DESK_PADDING * 2.0)
            .height(self.desk_height())
            .horizontal_alignment(HorizontalAlignment::Center)
            .content(page);

        let column = StackPanel::new()
            .orientation(Orientation::Vertical)
            .spacing(6.0)
            .children((desk, screen));

        Border::new()
            .padding(Thickness::uniform(8.0))
            .drop_policy(
                DragDropPolicy::new().storage_items(
                    DragDropAction::new(DragDropOperation::Copy).caption("Open this PDF"),
                ),
            )
            .on_drop({
                let sender = sender.clone();
                move |data: DroppedData| {
                    let _ = sender.send(HostMessage::Dropped(data));
                }
            })
            .content(column)
            .into()
    }
}

impl Shell {
    // ── the inbox: the only way in ─────────────────────────────────────────

    /// Parks one background task on the inbox.
    ///
    /// The task blocks on a *worker* thread, so the UI thread is untouched; it
    /// returns the moment something arrives, which keeps the pen's latency at
    /// the reader's poll interval instead of a whole frame.
    fn arm_pump(&mut self, context: &ComponentContext<Self>) {
        if self.pump_armed {
            return;
        }
        let inbox = Arc::clone(&self.inbox);
        let _ = context.spawn_background(move |_cancel| {
            inbox.pop_blocking();
            HostMessage::Wake
        });
        self.pump_armed = true;
    }

    /// Drains everything the workers have produced.  Never waits.
    fn drain(&mut self) {
        while let Some(message) = self.inbox.try_pop() {
            match message {
                // The ticker posts into the same inbox as the workers, so a tick
                // usually arrives *inside* a drain — it still has to count as a
                // frame, or the meter would sit at 0 fps.
                HostMessage::Tick => self.tick(),
                other => self.apply(other),
            }
        }
    }

    /// One message → state.  Every branch is O(1), except the ones that follow a
    /// page change (undo, clear, page move), which touch the page's strokes once.
    fn apply(&mut self, message: HostMessage) {
        match message {
            HostMessage::Wake | HostMessage::Tick => {}
            HostMessage::Tablet(batch) => self.tablet(batch),
            HostMessage::Pointer {
                x,
                y,
                pressure,
                phase,
            } => self.pointer(x, y, pressure, phase),
            HostMessage::Baked(baked) => self.baked(baked),
            HostMessage::Decoded(slot) => {
                if let Some(buffer) = self.buffers.get_mut(slot) {
                    buffer.decoded = true;
                    self.active = slot;
                    self.baked_at = None;
                }
            }
            HostMessage::PdfOpened(result) => self.pdf_opened(result),
            HostMessage::PageRendered(rendered) => {
                if rendered.index == self.document.current_index() && rendered.scale == self.scale {
                    self.render_ms = rendered.elapsed_ms;
                    self.background = Some(rendered.bitmap);
                    self.background_key = Some((rendered.index, self.scale.get().to_bits()));
                    self.request_bake();
                }
            }
            HostMessage::Exported(result) => {
                self.status = match result {
                    Ok(path) => format!("saved {}", path.display()),
                    Err(error) => format!("export failed: {error}"),
                };
            }
            HostMessage::Failed(error) => self.status = error,
            HostMessage::PathChosen { purpose, path } => self.path_chosen(purpose, path),
            HostMessage::Intent(intent) => self.intent(intent),
            HostMessage::Dropped(data) => self.dropped(data),
            HostMessage::Resized(width, height) => {
                self.client = Size::new(width as f32, height as f32);
                // The page fits the window, so a resize means a new scale.
                self.refit();
            }
        }
    }

    /// A frame boundary: the meters move here — and so does the page buffer.
    ///
    /// The second half is a safety net: WinUI decodes a `BitmapImage`
    /// asynchronously and tells us with `ImageOpened`, but that callback only
    /// fires once the element is live.  Depending on it alone meant the page
    /// stayed invisible until some *other* message caused a rebuild.  After
    /// [`DECODE_GRACE`] the buffer is shown regardless: a blank frame is better
    /// than a page that never appears.
    fn tick(&mut self) {
        let now = Instant::now();
        self.meter
            .record(now.saturating_duration_since(self.last_frame));
        self.last_frame = now;
        self.frame_count += 1;

        let inactive = 1 - self.active;
        if let Some(baked_at) = self.baked_at
            && !self.buffers[inactive].decoded
            && baked_at.elapsed() >= DECODE_GRACE
        {
            self.buffers[inactive].decoded = true;
            self.active = inactive;
            self.baked_at = None;
        }
    }

    // ── input ──────────────────────────────────────────────────────────────

    /// One delivery from the OTD reader thread.
    fn tablet(&mut self, batch: Batch) {
        if let Some(spec) = batch.spec {
            self.map = PageMap::fit(&spec, self.document.page().size, self.settings.margin_pt);
            self.tablet = Some(spec);
        }
        self.source = batch.source.clone();
        // A reason is a fact worth showing: "no range" is what stops the pen.
        if let Some(problem) = &batch.problem {
            self.status = problem.clone();
        }
        self.samples += batch.samples.len() as u64;
        self.skipped += batch.stats.skipped;
        for sample in &batch.samples {
            self.pen(sample);
        }
    }

    /// One pen report → ink.  This is the hot path: O(1), no allocation after
    /// the first point of a stroke, no work proportional to the page.
    fn pen(&mut self, sample: &otd::Sample) {
        let Some(map) = self.map else {
            return;
        };
        let at = map.to_page(sample.x, sample.y);
        let stale = sample.time.saturating_sub(self.last_sample_time) > STALE_GAP_TICKS;

        // Pressure: what the hardware reports (smoothed), or speed when it
        // reports none.  A gap in time ends the stroke instead of drawing a line
        // across the page — the samples in the ring are still valid, they are
        // just old.
        let pressure = if sample.touches() && sample.pressure > 0.0 {
            self.pressure = smooth_pressure(self.pressure, map.pressure(sample.pressure));
            self.pressure
        } else if sample.touches() {
            pressure_from_speed(self.speed_pt_per_ms(at, sample.time))
        } else {
            0.0
        };

        if stale && self.live.is_some() {
            self.commit_live();
        }

        let erasing = sample.is_eraser() || self.tool.erases();
        if sample.touches() && !sample.out_of_range() {
            if erasing {
                self.erasing = true;
                self.erase_at(at);
            } else {
                self.ink(at, pressure, sample.time);
            }
        } else {
            self.commit_live();
        }

        self.last_pos = at;
        self.last_sample_time = sample.time;
    }

    /// Adds a point to the stroke under the pen, starting one if needed.
    fn ink(&mut self, at: Pt, pressure: f32, ticks: u64) {
        let time_ms = ticks.saturating_sub(self.stroke_start) as f64 / 10_000.0;
        let point = InkPoint {
            pos: at,
            pressure,
            time_ms,
        };
        match &mut self.live {
            Some(stroke) => stroke.push(point),
            None => {
                let mut stroke = Stroke::new(Style::new(self.tool, self.width_for(self.tool)));
                stroke.push(point);
                self.live = Some(stroke);
                self.stroke_start = ticks;
            }
        }
    }

    /// Ends the stroke under the pen (a tap stays a dot, an empty stroke is
    /// dropped).
    fn commit_live(&mut self) {
        self.erasing = false;
        if let Some(stroke) = self.live.take()
            && !stroke.is_empty()
        {
            self.document.add_stroke(stroke);
            self.invalidate();
        }
    }

    /// Removes the strokes the eraser covers, as one undoable edit per step.
    fn erase_at(&mut self, at: Pt) {
        let radius = self.settings.eraser_radius_pt;
        let hits = self.document.hits(at, radius, Tool::Eraser);
        if !hits.is_empty() {
            self.document.remove_strokes(&hits);
            self.invalidate();
        }
    }

    /// Speed of the pen in pt/ms (used when the tablet reports no pressure).
    fn speed_pt_per_ms(&self, at: Pt, ticks: u64) -> f32 {
        let elapsed_ms = ticks.saturating_sub(self.last_sample_time) as f32 / 10_000.0;
        if elapsed_ms <= 0.0 {
            return 0.0;
        }
        self.last_pos.distance_to(at) / elapsed_ms
    }

    /// The fallback path: a WinUI pointer (a mouse, or a pen without OTD).
    ///
    /// The coordinates are page DIPs (the handler sits on a control the size of
    /// the page), so `scale.pt(..)` turns them into page points — no scroll offset
    /// and no toolbar height to subtract.  A pointer event carries no device clock,
    /// so the samples are stamped from `pointer_base` instead.
    fn pointer(&mut self, x: f64, y: f64, pressure: f32, phase: PointerPhase) {
        // A digitizer that runs through OTD already writes ink, and WinUI reports
        // the same pen as a pointer: taking both would draw two strokes.
        if matches!(self.source, Source::Live { .. }) {
            return;
        }
        let at = Pt::new(self.scale.pt(x as f32), self.scale.pt(y as f32));
        // QPC-style ticks (100 ns), the unit the ink model stores.
        let ticks = Instant::now()
            .saturating_duration_since(self.pointer_base)
            .as_nanos() as u64
            / 100;
        match phase {
            PointerPhase::Pressed => {
                self.pressure = pressure;
                self.ink(at, pressure, ticks);
            }
            PointerPhase::Moved => {
                if self.live.is_some() {
                    self.ink(at, pressure, ticks);
                }
            }
            PointerPhase::Released => self.commit_live(),
        }
        self.last_pos = at;
    }

    // ── screen / document ──────────────────────────────────────────────────

    /// A page bitmap arrived.  It goes into the **inactive** buffer; the swap
    /// happens only when WinUI reports that it has decoded (`Decoded`).
    fn baked(&mut self, baked: Baked) {
        self.bake_ms = baked.elapsed_ms;
        if baked.generation != self.generation {
            return;
        }
        self.baking = None;
        let slot = 1 - self.active;
        self.buffers[slot] = Buffer {
            png: Some(Arc::from(baked.png)),
            decoded: false,
        };
        // Start the clock on WinUI's decode callback (see `tick`).
        self.baked_at = Some(Instant::now());
    }

    /// A PDF was opened and measured.
    fn pdf_opened(&mut self, result: Result<OpenedPdf, String>) {
        match result {
            Ok(opened) => {
                self.status = format!(
                    "{} · {} page(s)",
                    opened.path.display(),
                    opened.pages.len()
                );
                self.document = Document::from_backgrounds(&opened.pages);
                self.pdf = Some(opened);
                self.refit();
                self.background = None;
                self.background_key = None;
                self.buffers = [Buffer::default(), Buffer::default()];
                self.active = 0;
                self.generation += 1;
                self.request_render();
                self.request_bake();
            }
            Err(error) => self.status = format!("could not open the PDF: {error}"),
        }
    }

    /// The dialog worker answered.
    fn path_chosen(&mut self, purpose: Purpose, path: Option<PathBuf>) {
        let Some(path) = path else {
            self.status = "cancelled".to_string();
            return;
        };
        match purpose {
            Purpose::OpenPdf => self.workers.pdf(PdfJob::Open(path)),
            Purpose::SavePdf => {
                let pages: Vec<PageInk> = self
                    .document
                    .pages()
                    .iter()
                    .enumerate()
                    .filter(|(_, page)| !page.strokes.is_empty())
                    .map(|(index, page)| PageInk {
                        index,
                        strokes: page.strokes.clone(),
                    })
                    .collect();
                if pages.is_empty() {
                    self.status = "there is no ink to save yet".to_string();
                    return;
                }
                // The status line is built before the path is handed over (the
                // worker owns it from here).
                self.status = format!("writing {} …", path.display());
                self.workers.pdf(PdfJob::Export { pages, path });
            }
            Purpose::SavePng => {
                self.workers.pdf(PdfJob::ExportPng {
                    page: self.document.page().clone(),
                    background: self.background.clone(),
                    scale: self.scale,
                    path,
                });
                self.status = "writing the PNG …".to_string();
            }
        }
    }

    /// A PDF (or note file) was dropped on the window.
    fn dropped(&mut self, data: DroppedData) {
        let DroppedData::StorageItems(items) = data else {
            return;
        };
        let Some(item) = items.first() else {
            return;
        };
        self.workers.pdf(PdfJob::Open(PathBuf::from(&item.path)));
        self.status = format!("opening {} …", item.name);
    }

    // ── intents from the screen ────────────────────────────────────────────

    /// What the elm screen asked for: the screen never edits the document, it
    /// asks, and this decides.
    fn intent(&mut self, intent: Intent) {
        match intent {
            Intent::Tool(tool) => {
                self.tool = tool;
                self.settings.tool = tool;
            }
            Intent::Thinner => self.nudge_width(-1.0),
            Intent::Thicker => self.nudge_width(1.0),
            Intent::Undo => {
                if self.document.undo() {
                    self.invalidate();
                }
            }
            Intent::Redo => {
                if self.document.redo() {
                    self.invalidate();
                }
            }
            Intent::ClearPage => {
                if self.document.clear_page() {
                    self.invalidate();
                }
            }
            Intent::NextPage => {
                if self.document.next_page() {
                    self.page_changed();
                }
            }
            Intent::PreviousPage => {
                if self.document.previous_page() {
                    self.page_changed();
                }
            }
            Intent::GoToPage(index) => {
                if self.document.go_to(index) {
                    self.page_changed();
                }
            }
            Intent::ZoomIn => self.set_zoom(self.zoom * 1.25),
            Intent::ZoomOut => self.set_zoom(self.zoom / 1.25),
            Intent::ZoomReset => self.set_zoom(100.0),
            Intent::OpenPdf => self.workers.dialog(DialogJob {
                purpose: Purpose::OpenPdf,
                suggested: None,
            }),
            Intent::ExportPdf => self.workers.dialog(DialogJob {
                purpose: Purpose::SavePdf,
                suggested: None,
            }),
            Intent::ExportPng => self.workers.dialog(DialogJob {
                purpose: Purpose::SavePng,
                suggested: None,
            }),
            Intent::SetRefresh(hz) => {
                self.refresh = RefreshChoice::from_hz(hz);
                self.settings.refresh = self.refresh;
                self.meter.reset();
                self.worst_busy_ms = 0.0;
                self.workers
                    .set_refresh(self.refresh.resolve(self.detected_hz));
            }
            Intent::ToggleHelp => self.help = !self.help,
            Intent::ToggleDev => self.dev = !self.dev,
            Intent::RescanTablet => {
                self.rescan.store(true, Ordering::Relaxed);
                self.status = "looking for the tablet again…".to_string();
            }
            Intent::Quit => std::process::exit(0),
        }
        let _ = self.settings.save();
    }

    /// Makes the nib of the current tool thinner or thicker.
    fn nudge_width(&mut self, direction: f32) {
        let step = match self.tool {
            Tool::Pen => 0.5,
            Tool::Highlighter | Tool::Eraser => 2.0,
        };
        let width = self.width_for(self.tool) + step * direction;
        match self.tool {
            Tool::Pen => self.settings.pen_width_pt = width,
            Tool::Highlighter => self.settings.highlighter_width_pt = width,
            Tool::Eraser => self.settings.eraser_radius_pt = width,
        }
        self.settings.normalize();
    }

    fn width_for(&self, tool: Tool) -> f32 {
        self.settings.width_for(tool)
    }

    fn set_zoom(&mut self, zoom: f32) {
        let zoom = zoom.clamp(25.0, 800.0);
        if (zoom - self.zoom).abs() < 0.01 {
            return;
        }
        self.zoom = zoom;
        // 100% is "the whole page fits the window", so the fit is recomputed with
        // the new zoom — and the page is baked again, because the bitmap is drawn
        // at this scale.
        self.refit();
        self.background = None;
        self.background_key = None;
        self.buffers = [Buffer::default(), Buffer::default()];
        self.active = 0;
        self.generation += 1;
        self.request_render();
        self.request_bake();
    }

    /// The current page changed: its background and its ink are different.
    fn page_changed(&mut self) {
        self.commit_live();
        // A PDF page can have its own size, so the fit is recomputed.
        self.refit();
        self.background = None;
        self.background_key = None;
        self.buffers = [Buffer::default(), Buffer::default()];
        self.active = 0;
        self.generation += 1;
        self.request_render();
        self.request_bake();
    }

    // ── the bake / render pipeline ─────────────────────────────────────────

    /// The page changed: the next bitmap must include the change.
    fn invalidate(&mut self) {
        self.generation += 1;
        self.request_bake();
    }

    /// Asks the bake worker for a bitmap, at most once per generation.
    fn request_bake(&mut self) {
        if self.baking == Some(self.generation) {
            return;
        }
        self.baking = Some(self.generation);
        self.workers.bake(BakeRequest {
            page: self.document.page().clone(),
            scale: self.scale,
            background: self.background.clone(),
            generation: self.generation,
        });
    }

    /// Asks the PDF worker for the current page's background, if it has one.
    fn request_render(&mut self) {
        let Some(pdf) = &self.pdf else {
            return;
        };
        let Some(index) = self.document.page().background else {
            return;
        };
        if index >= pdf.pages.len() {
            return;
        }
        if self.background_key == Some((index, self.scale.get().to_bits())) {
            return;
        }
        self.workers.pdf(PdfJob::Render {
            index,
            scale: self.scale,
        });
    }

    /// Called after every message: keeps the pipeline moving without the UI
    /// thread doing any of the work.
    fn refresh(&mut self) {
        self.request_render();
    }

    // ── what the screen shows ──────────────────────────────────────────────

    /// The desk's height: the window minus the toolbar, minus the desk's padding.
    fn desk_height(&self) -> f64 {
        (self.client.height as f64 - SCREEN_HEIGHT - DESK_PADDING * 2.0).max(160.0)
    }

    /// Sets `scale` so the whole page fits the window at the current zoom.
    ///
    /// There is no scrolling, so "100%" has to mean "the page is fully visible":
    /// the fit comes from the window and the zoom scales that fit (never past it).
    /// A new scale means the bitmap is wrong (it is drawn at that scale), so the
    /// page is baked again — on the worker, never on this thread.
    fn refit(&mut self) {
        let page = self.document.page().size;
        if page.width <= 0.0 || page.height <= 0.0 {
            return;
        }
        let available_width = (self.client.width as f64 - DESK_PADDING * 2.0).max(64.0);
        let available_height = (self.desk_height() - DESK_PADDING * 2.0).max(64.0);
        let fit = (available_width / page.width as f64).min(available_height / page.height as f64);
        // There is no scrolling *and* no clip rectangle, so a page bigger than the
        // desk would draw over the toolbar: the fit is the ceiling.  Zooming out
        // works, zooming in stops at "the whole page is visible" — and the zoom the
        // shell reports is the one the page is actually shown at.
        let wanted = fit * self.zoom as f64 / 100.0;
        let scale = wanted.min(fit).clamp(0.02, 20.0) as f32;
        if wanted > fit + 0.001 {
            self.zoom = 100.0;
        }
        if (scale - self.scale.get()).abs() < 0.001 {
            return;
        }
        self.scale = Scale::new(scale);
        self.invalidate();
    }

    /// The page's size in pixels at the current zoom.
    fn page_pixels(&self) -> (f64, f64) {
        let page = self.document.page().size;
        (
            self.scale.px(page.width) as f64,
            self.scale.px(page.height) as f64,
        )
    }

    /// The values the elm screen renders (props down).
    fn view_model(&self) -> ViewModel {
        let page = self.document.page();
        let hz = self.refresh.resolve(self.detected_hz);
        let budget_ms = frame_period(hz).as_secs_f32() * 1000.0;
        let title = match &self.pdf {
            Some(pdf) => pdf
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "light-note".to_string()),
            None => "light-note".to_string(),
        };
        ViewModel {
            title,
            subtitle: if page.background.is_some() {
                format!(
                    "page {} of {} · {}",
                    self.document.current_index() + 1,
                    self.document.page_count(),
                    self.status
                )
            } else {
                format!("blank note · {}", self.status)
            },
            tool: self.tool,
            tool_line: format!(
                "{} · {:.1} pt · zoom {:.0}%",
                tool_name(self.tool),
                self.width_for(self.tool),
                self.zoom
            ),
            pages: self.document.page_count(),
            page_index: self.document.current_index(),
            strokes: page.strokes.len(),
            zoom: self.zoom,
            tablet: self.source.summary(),
            tablet_ok: matches!(self.source, Source::Live { .. }),
            refresh: format!(
                "display {} Hz · loop {} Hz · {:.0} fps",
                self.detected_hz.max(60),
                hz,
                self.meter.fps()
            ),
            fps: format!("{:.0} fps", self.meter.fps()),
            worst_frame: format!(
                "ui busy {:.2} ms (worst {:.2} ms, budget {:.2} ms)",
                self.busy_ms, self.worst_busy_ms, budget_ms
            ),
            hint: self.hint.clone(),
            help: self.help,
            dev: self.dev,
            dev_report: self.dev_report(),
            source: Some(self.source.clone()),
        }
    }

    /// The diagnostics panel: the numbers that explain the writing feel.
    fn dev_report(&self) -> String {
        let hz = self.refresh.resolve(self.detected_hz);
        let page = self.document.page();
        // Where the pen can reach: the tablet is fitted into the page keeping its
        // aspect ratio, so this is the one line that explains why the ink lands
        // where it does.
        let ink_area = match &self.map {
            Some(map) => {
                let (left, top) = map.origin();
                let (right, bottom) = map.far_corner();
                format!("x {left:.0}..{right:.0} · y {top:.0}..{bottom:.0} pt")
            }
            None => "none yet".to_string(),
        };
        format!(
            "loop {} Hz · display {} Hz · frames {}\n\
             ui busy {:.2} ms (worst {:.2} ms) · bake {:.1} ms · pdf render {:.1} ms\n\
             pen samples {} · skipped by the ring {}\n\
             tablet: {}\n\
             ink area {} (tablet fitted into the page, aspect kept)\n\
             page {:.0}×{:.0} pt · scale {:.2} px/pt · margin {:.0} pt\n\
             strokes on this page {} · undo {} · redo {}\n\
             pdf: {}",
            hz,
            self.detected_hz.max(60),
            self.frame_count,
            self.busy_ms,
            self.worst_busy_ms,
            self.bake_ms,
            self.render_ms,
            self.samples,
            self.skipped,
            self.source.summary(),
            ink_area,
            page.size.width,
            page.size.height,
            self.scale.get(),
            self.settings.margin_pt,
            page.strokes.len(),
            self.document.can_undo(),
            self.document.can_redo(),
            match &self.pdf {
                Some(pdf) => format!("{} page(s) open", pdf.pages.len()),
                None => "none open".to_string(),
            },
        )
    }

    /// The line the status bar starts with.
    fn startup_status(&self) -> String {
        let hz = self.refresh.resolve(self.detected_hz);
        format!("ready · {} Hz loop · {}", hz, self.source.summary())
    }
}

/// The name the status bar uses for a tool.
fn tool_name(tool: Tool) -> &'static str {
    match tool {
        Tool::Pen => "Pen",
        Tool::Highlighter => "Highlighter",
        Tool::Eraser => "Eraser",
    }
}

/// Runs the app: one window, one host component, everything else on threads.
///
/// The error is `anyhow`'s: `windows-reactor` reports `windows-core`'s error
/// type, and the app layer already uses `anyhow` for startup failures.
pub fn run() -> anyhow::Result<()> {
    windows_reactor::App::run_component::<Shell>(()).map_err(|error| anyhow::anyhow!("{error}"))
}
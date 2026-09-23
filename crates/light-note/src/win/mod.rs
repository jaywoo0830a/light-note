//! The Windows layer: WinUI 3 through `windows-reactor`, UI text through
//! `elm-magic`, and every slow thing on a worker thread.
//!
//! ```text
//! ┌─ UI thread ────────────────────────────────────────────────────────────┐
//! │ Shell (windows-reactor Component)                                      │
//! │   update(message)  → apply to Document/tool/view model, O(1) per sample│
//! │   view()           → WinUI tree: toolbar + surface + status            │
//! │      └─ ElmView<Screen>  (elm-magic: status, page rail, help, dev)     │
//! │   nothing here opens a file, calls Pdfium, rasterizes or waits         │
//! └────────────────────────────────────────────────────────────────────────┘
//!        ▲ Inbox (never waited on: try_pop)      │ jobs (owned threads)
//!        │                                        ▼
//! ┌─ workers ──────────────────────────────────────────────────────────────┐
//! │ otd reader (1 ms poll) · bake (latest-wins) · pdf (own thread)         │
//! │ frame ticker (60/120/180/240 Hz) · file dialogs (STA)                  │
//! └────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The input arrives in two halves and they meet in the shell: the canvas's own
//! pointer event says *where* the pen is, and the OTD reader says *how* it is
//! pressed and held (pressure, tilt, the barrel's rotation).  `shell` explains
//! why that split is what keeps the ink under the nib.

pub mod screen;
pub mod shell;
pub mod surface;
pub mod workers;

use std::path::PathBuf;

use windows_reactor::DroppedData;

use crate::ink::Tool;
use crate::otd::{Batch, Source};

pub use shell::Shell;

/// What the elm screen asks the host to do.  A screen never touches the
/// document; it sends an [`Intent`] and the host decides (example 11).
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// Select a tool.
    Tool(Tool),
    /// Make the nib thinner / thicker.
    Thinner,
    Thicker,
    Undo,
    Redo,
    /// Clear the current page's ink (one undoable edit).
    ClearPage,
    /// Page navigation.
    NextPage,
    PreviousPage,
    GoToPage(usize),
    /// Zoom.
    ZoomIn,
    ZoomOut,
    ZoomReset,
    /// Ask for a PDF to open (a dialog on a worker thread).
    OpenPdf,
    /// Save the annotated copy as PDF / the current page as PNG.
    ExportPdf,
    ExportPng,
    /// Refresh rate: 0 means "follow the display".
    SetRefresh(u32),
    /// Show/hide the help panel.
    ToggleHelp,
    /// Show/hide the input + frame diagnostics.
    ToggleDev,
    /// Turn the debug trace (see `Settings::debug_path`) on or off.
    ToggleTrace,
    /// Re-scan for the OTD plugin (the daemon may have started later).
    RescanTablet,
    /// Quit.
    Quit,
}

/// Everything that reaches the UI thread.
///
/// `Send` is required: workers produce these.  Nothing in here is a lock, a
/// handle or a platform object — only values the UI can apply immediately.
#[derive(Clone, Debug)]
pub enum HostMessage {
    /// A frame boundary (from the ticker): updates the meters, nothing else.
    Tick,
    /// "The inbox has something" — the UI drains it (never blocks).
    Wake,
    /// Pen **attributes** from the OTD reader thread: pressure, tilt, the
    /// barrel's rotation and the eraser flag.  The position is not in here — that
    /// is the canvas's own pointer event ([`HostMessage::Pointer`]).
    Tablet(Batch),
    /// What the elm screen asked for (it never touches the document itself).
    Intent(Intent),
    /// A pointer sample from the canvas: the position the pen (or the mouse) is
    /// touching, in *page* DIPs.  The pressure and the tilt do not travel with
    /// it — they come from OTD's shared memory ([`crate::otd::PenState`]).
    Pointer { x: f64, y: f64, phase: PointerPhase },
    /// A page bitmap finished baking (the tail can be trimmed).
    Baked(workers::Baked),
    /// WinUI finished decoding a buffered bitmap — it may now be shown.
    Decoded(usize),
    /// A PDF or a note file was dropped on the window.
    Dropped(DroppedData),
    /// A PDF was opened by the dialog worker and parsed by the PDF worker.
    PdfOpened(Result<OpenedPdf, String>),
    /// A page of the background PDF finished rendering.
    PageRendered(workers::Rendered),
    /// An export finished.
    Exported(Result<PathBuf, String>),
    /// Something failed (shown in the status bar, never fatal).
    Failed(String),
    /// A dialog was answered with a path (or cancelled).
    PathChosen { purpose: Purpose, path: Option<PathBuf> },
    /// The window was resized (DIP).
    Resized(f64, f64),
}

/// Pointer phases from WinUI: the canvas's own report of where the pen is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerPhase {
    Pressed,
    Moved,
    Released,
}

/// What a chosen path is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    OpenPdf,
    SavePdf,
    SavePng,
}

/// A PDF the worker opened: the bytes plus what the UI needs to know.
#[derive(Clone, Debug)]
pub struct OpenedPdf {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub pages: Vec<crate::geom::Size>,
}

/// The view model the elm screen renders: every field is a value the host owns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewModel {
    pub title: String,
    pub subtitle: String,
    pub tool: Tool,
    pub tool_line: String,
    pub pages: usize,
    pub page_index: usize,
    pub strokes: usize,
    pub zoom: f32,
    pub tablet: String,
    pub tablet_ok: bool,
    pub refresh: String,
    pub fps: String,
    pub worst_frame: String,
    pub hint: String,
    pub help: bool,
    pub dev: bool,
    pub dev_report: String,
    pub source: Option<Source>,
}

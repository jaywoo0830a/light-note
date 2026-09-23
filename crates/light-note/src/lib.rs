//! light-note — a Windows 11 note-taking app for drawing pads.
//!
//! Write on a PDF with a pen: the tablet talks to
//! [OpenTabletDriver](https://opentabletdriver.net/), a small OTD plugin
//! (`OTD.SharedMemoryOutput/`) publishes the pen's **pressure, tilt, rotation and
//! eraser flag** through shared memory, the canvas's own pointer event says
//! *where* the pen is (so the ink lands under the nib),
//! [Pdfium](https://pdfium.googlesource.com/pdfium/) renders the page (through
//! `pdfium-render`), and [elm-magic](https://crates.io/crates/elm-magic) turns
//! state into WinUI 3 controls (through `elm-magic-windows-reactor`).
//!
//! # The one rule that shapes everything: the UI thread never waits
//!
//! Not "should not" — *cannot*, structurally.  The UI thread (the one that runs
//! `Component::update` and `view`) only ever:
//!
//! 1. **applies a message** to plain values ([`doc::Document`], [`ink`],
//!    `win::surface` state) — O(1) per pen sample;
//! 2. **builds a view** from those values — bounded by [`shape::LIVE_BUDGET`]
//!    live shapes, never by the number of strokes on the page;
//! 3. **posts work** to a worker and moves on.
//!
//! Everything slow lives on another thread and reports back through
//! [`inbox::Inbox`], which the UI drains with `try_pop` — a call that cannot
//! block even in principle:
//!
//! | work | thread | comes back as |
//! |---|---|---|
//! | pen attributes (shared memory) | OTD reader thread | a batch of reports |
//! | page raster + ink bake + PNG | bake worker (latest-wins) | a ready bitmap |
//! | Pdfium open/render/save | PDF worker | page sizes, a page bitmap, saved bytes |
//! | file dialogs | dialog worker | a chosen path |
//! | frame pacing (60/120/180/240 Hz) | frame ticker | a tick with a timestamp |
//!
//! Nothing on the UI thread opens a file, calls Pdfium, rasterizes a pixel,
//! encodes a PNG, sleeps, or waits on a lock.  [`display`] turns that into a
//! measurable number: every frame reports how long the UI thread was busy, and
//! the status bar shows the worst frame next to the frame budget.
//!
//! # Layers
//!
//! | layer | modules | tested |
//! |---|---|---|
//! | core: values, geometry, protocol, clock | [`geom`] [`ink`] [`shape`] [`doc`] [`otd`] [`display`] [`inbox`] [`pdf`] [`settings`] | **yes — `tests/` was written first** |
//! | UI: WinUI 3 translation | `win::*` (Windows only) | no (by design) |
//!
//! The core has no platform dependency, no lock and no thread: it is a pure
//! function of its inputs, which is why the whole suite runs headlessly.

pub mod display;
pub mod doc;
pub mod geom;
pub mod inbox;
pub mod ink;
pub mod otd;
pub mod pdf;
pub mod settings;
pub mod shape;

#[cfg(windows)]
pub mod win;

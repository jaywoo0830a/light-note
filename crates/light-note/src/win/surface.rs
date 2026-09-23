//! The drawing surface: two buffered page bitmaps plus the stroke being written.
//!
//! ```text
//! Canvas(page px)
//!  ├ Image "base-0"  opacity 1 or 0   ← baked page (PDF + committed ink)
//!  ├ Image "base-1"  opacity 0 or 1   ← the other buffer, decoded and ready
//!  └ live shapes (only the stroke in progress): Line per segment, Ellipse per cap
//! ```
//!
//! Why two images: WinUI decodes a new `BitmapImage` asynchronously, so pointing
//! a single `Image` at fresh bytes makes it blink.  Instead the bake goes into
//! the **inactive** buffer, and only when `ImageOpened` has fired (the UI has
//! seen the decode complete) does the host flip the two opacities — a swap that
//! costs nothing and cannot show an empty frame.
//!
//! The live shapes are the same geometry the rasterizer uses
//! ([`crate::shape::live_pieces`]), so committing a stroke does not change how
//! the ink looks.

use windows_reactor::{
    Border, Brush, Canvas, CanvasChildExt, ChildrenControl, Color, ContentControl, Ellipse,
    EncodedImage, HorizontalAlignment, Image, KeyedView, LayoutControl, Line, PointerEventInfo, View,
    VerticalAlignment,
};

use crate::geom::Scale;
use crate::ink::{Rgba, Stroke};
use crate::shape::{LivePiece, live_pieces, stroke_shape};

use super::PointerPhase;

/// One buffered bitmap: PNG bytes plus whether it may be shown.
#[derive(Clone, Debug, Default)]
pub struct Buffer {
    pub png: Option<std::sync::Arc<[u8]>>,
    /// True when WinUI has decoded this buffer (it is safe to show).
    pub decoded: bool,
}

/// What a mouse reports as pressure: half, which is what Windows uses for a
/// device without a pressure sensor.
const MOUSE_PRESSURE: f32 = 0.5;

/// Builds the surface for the current page.
///
/// `buffers` is the double buffer; `active` is the index the user should be
/// looking at.  `live` is the stroke under the pen (or `None`).  `on_decoded`
/// is called (with the buffer index) when WinUI has finished decoding a buffer,
/// which is the only moment it is safe to show it.  `on_pointer` receives the
/// fallback input (a mouse, or a pen without OTD) in page DIPs.
///
/// Returns the view and the number of live shapes in it: the count is what the
/// UI thread pays per frame, so the diagnostics report it instead of guessing.
pub fn build(
    page_px: (f64, f64),
    buffers: &[Buffer; 2],
    active: usize,
    live: Option<&Stroke>,
    scale: Scale,
    on_decoded: std::rc::Rc<dyn Fn(usize)>,
    on_pointer: std::rc::Rc<dyn Fn(PointerPhase, f64, f64, f32)>,
) -> (View, usize) {
    let (width, height) = page_px;

    let slot = |index: usize, buffer: &Buffer| -> View {
        let shown = index == active && buffer.decoded;
        let image = Image::new().width(width).height(height);
        let image = match &buffer.png {
            Some(png) => image
                .source_data(EncodedImage::new(png.clone()))
                .on_opened({
                    let on_decoded = std::rc::Rc::clone(&on_decoded);
                    move || on_decoded(index)
                }),
            None => image,
        };
        image.opacity(if shown { 1.0 } else { 0.0 }).into()
    };

    let mut shapes: Vec<View> = Vec::new();
    if let Some(stroke) = live
        && let Some(shape) = stroke_shape(stroke, scale)
    {
        for piece in live_pieces(&shape) {
            shapes.push(piece_view(piece, stroke.style().color));
        }
    }
    let live_count = shapes.len();

    // Two image layers plus the live shapes.  The children are keyed, so a
    // frame that adds one segment adds one element instead of rebuilding the
    // canvas (the UI thread's cost must not grow with the page).
    let mut children: Vec<KeyedView> = Vec::with_capacity(2 + shapes.len());
    children.push(KeyedView::new("base-0", slot(0, &buffers[0])));
    children.push(KeyedView::new("base-1", slot(1, &buffers[1])));
    for (index, shape) in shapes.into_iter().enumerate() {
        // Position keys are stable while the stroke grows: segment `n` keeps its
        // element for the whole stroke.
        children.push(KeyedView::new(index + 2, shape));
    }

    let canvas = Canvas::new()
        .width(width)
        .height(height)
        .keyed_children(children);

    // The pointer events live on a `Border` (the only control that carries them in
    // windows-reactor), sized exactly like the page — so `info.x`/`info.y` are
    // *page* coordinates in DIPs, not window coordinates, and the mouse lands where
    // it points at any zoom.
    let event = |phase: PointerPhase| {
        let on_pointer = std::rc::Rc::clone(&on_pointer);
        move |info: PointerEventInfo| {
            // A mouse reports 0.5; a pen that reaches us through WinUI reports its
            // own pressure.
            on_pointer(phase, info.x, info.y, MOUSE_PRESSURE)
        }
    };

    (
        Border::new()
            .width(width)
            .height(height)
            // The sheet is centered on the desk (the desk is bigger than the page).
            .horizontal_alignment(HorizontalAlignment::Center)
            .vertical_alignment(VerticalAlignment::Center)
            .on_pointer_pressed(event(PointerPhase::Pressed))
            .on_pointer_moved(event(PointerPhase::Moved))
            .on_pointer_released(event(PointerPhase::Released))
            .content(canvas),
        live_count,
    )
}

/// One live shape as a WinUI control.
fn piece_view(piece: LivePiece, color: Rgba) -> View {
    let brush = Brush::from(Color::rgb(color.r, color.g, color.b));
    match piece {
        LivePiece::Segment {
            from,
            to,
            width,
        } => Line::new()
            .x1(from.0)
            .y1(from.1)
            .x2(to.0)
            .y2(to.1)
            .stroke(brush)
            .stroke_thickness(width)
            .into(),
        LivePiece::Cap { center, radius } => {
            // A `Canvas` child sits at its origin, so the disc is placed with
            // the two attached coordinates WinUI uses inside a `Canvas`.
            let diameter = radius * 2.0;
            Ellipse::new()
                .width(diameter)
                .height(diameter)
                .fill(brush)
                .canvas_left(center.0 - radius)
                .canvas_top(center.1 - radius)
                .into()
        }
    }
}

/// A spacer that keeps the page centered inside its scroll host.
pub fn centered(width: f64) -> View {
    Canvas::new()
        .width(width)
        .height(1.0)
        .vertical_alignment(VerticalAlignment::Center)
        .into()
}

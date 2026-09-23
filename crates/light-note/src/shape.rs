//! One shape decision, shared by the rasterizer and the live WinUI shapes.
//!
//! The contract this module exists to keep: **the ink looks the same the instant
//! a stroke is committed**.  If the bitmap and the live shapes disagreed, the
//! line would visibly jump when it moved from one to the other.  So both read
//! the same [`StrokeShape`]:
//!
//! * `[stroke_shape]` decides *once* whether a stroke is a dot, a constant-width
//!   curve or a sequence of varying-width spans;
//! * [`live_pieces`] turns that into WinUI primitives (a `Line` per segment, an
//!   `Ellipse` per cap) — WinUI's `Line` has no cap property, so each segment is
//!   *extended by its own half-width* and only the two ends get caps;
//! * [`spans_union`] turns the same spans into one filled path, so a stroke is
//!   rasterized with a **single fill** and overlapping pieces never multiply
//!   their alpha (that is what makes a translucent highlighter look even).

use vello_cpu::kurbo::{BezPath, PathEl, Point, flatten};
use vello_cpu::peniko::color::Srgb;
use vello_cpu::{Pixmap, RenderContext, Resources};

use crate::geom::{Scale, Size};
use crate::ink::{InkPoint, Rgba, Stroke, heading, width_at};

/// Tolerance (px) for flattening a curve into line segments.  Both consumers use
/// this value, so a curve cannot be smoother in one of them.
pub const FLATTEN_TOLERANCE_PX: f64 = 0.25;

/// How many live shapes the UI may hand to WinUI for the stroke being written.
/// Past this the tail is truncated rather than dropping frames (R1: the UI
/// thread's cost must not grow with the page).
pub const LIVE_BUDGET: usize = 2000;

/// The shape of one stroke, in pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum StrokeShape {
    /// Nothing to draw.
    #[default]
    Empty,
    /// A single point: a filled disc.
    Dot { center: (f64, f64), radius: f64 },
    /// A constant width: one midpoint-quadratic curve.
    Curve { width: f64, path: BezPath },
    /// A varying width: one span per pair of points.
    Spans(Vec<Span>),
}

/// One segment of a varying-width stroke, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    /// Nib width (px) — the average of the two ends.
    pub width: f64,
    pub from: (f64, f64),
    pub to: (f64, f64),
}

impl Span {
    /// Half the nib width, with a floor so a zero-pressure stroke is visible.
    pub fn half_width(&self) -> f64 {
        (self.width * 0.5).max(0.35)
    }

    /// The segment lengthened by its own half-width at both ends, so the joint
    /// with the next span is covered without a third shape.
    pub fn extended(&self) -> Self {
        let dx = self.to.0 - self.from.0;
        let dy = self.to.1 - self.from.1;
        let length = (dx * dx + dy * dy).sqrt();
        if length <= f64::EPSILON {
            return *self;
        }
        let half = self.half_width();
        let ux = dx / length * half;
        let uy = dy / length * half;
        Self {
            width: self.width,
            from: (self.from.0 - ux, self.from.1 - uy),
            to: (self.to.0 + ux, self.to.1 + uy),
        }
    }
}

impl StrokeShape {
    /// The spans of this shape (a curve is flattened first, a dot is a single
    /// zero-length span).
    pub fn spans(&self) -> Option<Vec<Span>> {
        match self {
            Self::Empty => None,
            Self::Dot { center, radius } => Some(vec![Span {
                width: radius * 2.0,
                from: *center,
                to: *center,
            }]),
            Self::Curve { width, path } => Some(flatten_spans(path, *width)),
            Self::Spans(spans) => Some(spans.clone()),
        }
    }

    /// Is there anything to draw?
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }
}

/// The shape of `stroke` at `scale`, or `None` when there is nothing to draw.
///
/// `None` is not an error: an empty stroke (a pen that was never put down, or a
/// stroke whose points were all dropped) has no shape, and saying so in the type
/// means every consumer has to decide what to do about it instead of drawing an
/// invisible shape.
///
/// Each segment's width is computed **for its own heading**: with the pen held
/// upright that is just the pressure pair, and with a tilted nib it is the
/// chisel effect — thick across the nib's edge, thin along it.
pub fn stroke_shape(stroke: &Stroke, scale: Scale) -> Option<StrokeShape> {
    let points = stroke.points();
    let factor = scale.get() as f64;
    if points.is_empty() {
        return None;
    }
    if points.len() == 1 {
        let point = points[0];
        let radius = (width_at(stroke.style(), point.pressure, point.nib, None) * factor * 0.5)
            .max(0.35);
        return Some(StrokeShape::Dot {
            center: (point.pos.x as f64 * factor, point.pos.y as f64 * factor),
            radius,
        });
    }

    // Two width sets, because they answer two different questions:
    //
    // * per **point**, with the widest footprint — is this stroke uniform enough
    //   to be one smooth curve, or does it taper?
    // * per **segment**, at that segment's own heading — how wide is this piece
    //   on the page?  With a tilted nib the two differ, and that difference *is*
    //   the chisel edge.
    let style = stroke.style();
    let point_widths: Vec<f64> = points
        .iter()
        .map(|point| width_at(style, point.pressure, point.nib, None) * factor)
        .collect();
    let segment_widths: Vec<f64> = points
        .windows(2)
        .map(|pair| {
            let direction = heading(pair[0].pos, pair[1].pos);
            let near = width_at(style, pair[0].pressure, pair[0].nib, direction);
            let far = width_at(style, pair[1].pressure, pair[1].nib, direction);
            (near + far) * 0.5 * factor
        })
        .collect();
    let max = point_widths
        .iter()
        .chain(segment_widths.iter())
        .copied()
        .fold(0.0_f64, f64::max);
    let min = point_widths
        .iter()
        .chain(segment_widths.iter())
        .copied()
        .fold(f64::MAX, f64::min);

    // A highlighter is constant by construction; a pen whose pressure *and* nib
    // angle barely moved is treated as constant too — one smooth curve instead of
    // many spans.
    if (max - min) <= max * 0.05 {
        return Some(StrokeShape::Curve {
            width: max,
            path: midpoint_path(points, factor),
        });
    }

    Some(StrokeShape::Spans(
        points
            .windows(2)
            .enumerate()
            .map(|(index, pair)| Span {
                width: segment_widths[index],
                from: (pair[0].pos.x as f64 * factor, pair[0].pos.y as f64 * factor),
                to: (pair[1].pos.x as f64 * factor, pair[1].pos.y as f64 * factor),
            })
            .collect(),
    ))
}

/// A midpoint quadratic through the samples: each control point is the previous
/// sample and each end point is the midpoint of two samples, so the curve passes
/// through the pen positions and stays smooth without any fitting pass.
fn midpoint_path(points: &[InkPoint], factor: f64) -> BezPath {
    let at = |point: &InkPoint| (point.pos.x as f64 * factor, point.pos.y as f64 * factor);
    let mut path = BezPath::new();
    let first = at(&points[0]);
    path.move_to(first);
    if points.len() == 2 {
        path.line_to(at(&points[1]));
        return path;
    }
    for index in 1..points.len() - 1 {
        let previous = at(&points[index - 1]);
        let current = at(&points[index]);
        let next = at(&points[index + 1]);
        let midpoint = Point::new((current.0 + next.0) * 0.5, (current.1 + next.1) * 0.5);
        path.quad_to(Point::new(previous.0, previous.1), midpoint);
    }
    let last = at(&points[points.len() - 1]);
    path.line_to(last);
    path
}

/// Flattens a curve into spans of constant width.
pub fn flatten_spans(path: &BezPath, width: f64) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut previous: Option<(f64, f64)> = None;
    flatten(path.iter(), FLATTEN_TOLERANCE_PX, |element| match element {
        PathEl::MoveTo(point) => previous = Some((point.x, point.y)),
        PathEl::LineTo(point) => {
            if let Some(from) = previous {
                spans.push(Span {
                    width,
                    from,
                    to: (point.x, point.y),
                });
            }
            previous = Some((point.x, point.y));
        }
        _ => {}
    });
    spans
}

/// The filled outline of a set of spans: each span is a rectangle lengthened by
/// its half-width, plus a disc at both ends of the stroke.
///
/// Filling this in **one** pass is what keeps a translucent stroke even — the
/// pieces overlap, and a nonzero fill does not care.
pub fn spans_union(spans: &[Span]) -> BezPath {
    let mut path = BezPath::new();
    if spans.is_empty() {
        return path;
    }
    for span in spans {
        push_span(&mut path, &span.extended());
    }
    // Round caps at the two ends only — the joints are already covered.
    let first = spans[0];
    let last = spans[spans.len() - 1];
    push_disc(&mut path, first.from, first.half_width());
    push_disc(&mut path, last.to, last.half_width());
    path
}

/// Adds one closed rectangle for `span`.
fn push_span(path: &mut BezPath, span: &Span) {
    let dx = span.to.0 - span.from.0;
    let dy = span.to.1 - span.from.1;
    let length = (dx * dx + dy * dy).sqrt();
    let half = span.half_width();
    let (nx, ny) = if length <= f64::EPSILON {
        (0.0, half)
    } else {
        (-dy / length * half, dx / length * half)
    };
    let corners = [
        (span.from.0 + nx, span.from.1 + ny),
        (span.to.0 + nx, span.to.1 + ny),
        (span.to.0 - nx, span.to.1 - ny),
        (span.from.0 - nx, span.from.1 - ny),
    ];
    path.move_to(Point::new(corners[0].0, corners[0].1));
    for corner in &corners[1..] {
        path.line_to(Point::new(corner.0, corner.1));
    }
    path.close_path();
}

/// Adds a closed disc (four cubic Béziers, the usual 0.5523 circle constant).
pub fn push_disc(path: &mut BezPath, center: (f64, f64), radius: f64) {
    const KAPPA: f64 = 0.552_284_749_830_793_4;
    let (cx, cy) = center;
    let r = radius.max(0.35);
    let k = r * KAPPA;
    path.move_to(Point::new(cx + r, cy));
    path.curve_to(
        Point::new(cx + r, cy + k),
        Point::new(cx + k, cy + r),
        Point::new(cx, cy + r),
    );
    path.curve_to(
        Point::new(cx - k, cy + r),
        Point::new(cx - r, cy + k),
        Point::new(cx - r, cy),
    );
    path.curve_to(
        Point::new(cx - r, cy - k),
        Point::new(cx - k, cy - r),
        Point::new(cx, cy - r),
    );
    path.curve_to(
        Point::new(cx + k, cy - r),
        Point::new(cx + r, cy - k),
        Point::new(cx + r, cy),
    );
    path.close_path();
}

/// One WinUI primitive of a live stroke (pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LivePiece {
    /// A `Line`: `from`/`to` are already extended by `width / 2`.
    Segment {
        from: (f64, f64),
        to: (f64, f64),
        width: f64,
    },
    /// An `Ellipse`: a round cap of `radius` at `center`.
    Cap { center: (f64, f64), radius: f64 },
}

/// The WinUI primitives for `shape`, capped at [`LIVE_BUDGET`].
pub fn live_pieces(shape: &StrokeShape) -> Vec<LivePiece> {
    let mut pieces = Vec::new();
    match shape {
        StrokeShape::Empty => {}
        StrokeShape::Dot { center, radius } => pieces.push(LivePiece::Cap {
            center: *center,
            radius: *radius,
        }),
        StrokeShape::Curve { .. } | StrokeShape::Spans(..) => {
            let Some(spans) = shape.spans() else {
                return pieces;
            };
            if spans.is_empty() {
                return pieces;
            }
            for span in &spans {
                let extended = span.extended();
                pieces.push(LivePiece::Segment {
                    from: extended.from,
                    to: extended.to,
                    width: extended.width,
                });
            }
            let first = spans[0];
            let last = spans[spans.len() - 1];
            pieces.push(LivePiece::Cap {
                center: first.from,
                radius: first.half_width(),
            });
            pieces.push(LivePiece::Cap {
                center: last.to,
                radius: last.half_width(),
            });
        }
    }
    pieces.truncate(LIVE_BUDGET);
    pieces
}

/// Rasterizes `strokes` onto `size` at `scale`, optionally over a background
/// bitmap (the rendered PDF page).
///
/// The ink goes into a transparent bitmap of its own and is then composited over
/// the background, because `vello_cpu` **clears its target** before it draws (a
/// `Replace` composite of the whole layer) — painting the background first would
/// lose it.
///
/// One `fill_path` per stroke with the nonzero rule: a stroke's pieces overlap
/// each other, and this is what keeps a translucent stroke from showing beads at
/// its joints.
pub fn rasterize(
    strokes: &[Stroke],
    size: Size,
    scale: Scale,
    background: Option<&Pixmap>,
) -> Pixmap {
    let (width, height) = size.to_pixels(scale);
    let mut ink = Pixmap::new(width, height);
    if !strokes.is_empty() {
        let mut context = RenderContext::new(width, height);
        context.set_fill_rule(vello_cpu::peniko::Fill::NonZero);
        for stroke in strokes {
            let Some(shape) = stroke_shape(stroke, scale) else {
                continue;
            };
            let Some(spans) = shape.spans() else {
                continue;
            };
            let path = spans_union(&spans);
            // `AlphaColor<Srgb>` converts into the context's brush, and a solid
            // brush keeps the stroke's own alpha (a highlighter stays translucent).
            context.set_paint(to_alpha_color(stroke.style().color));
            context.fill_path(&path);
        }
        // One `flush` before the render is the documented shape of a vello_cpu
        // frame (it is required when the context renders multi-threaded).
        context.flush();
        let mut resources = Resources::new();
        context.render(&mut ink, &mut resources);
    }

    match background {
        Some(background) => over(background, &ink),
        None => ink,
    }
}

/// `background` with `ink` composited over it (source-over, premultiplied).
///
/// Both bitmaps are premultiplied 8-bit RGBA, so the blend is `ink + paper *
/// (1 - ink.a)` per channel — the same arithmetic a GPU would do, and the reason
/// a translucent highlighter looks identical on the page and in the exported PNG.
fn over(background: &Pixmap, ink: &Pixmap) -> Pixmap {
    if background.width() != ink.width() || background.height() != ink.height() {
        // A background of another size cannot be composited; the ink alone is
        // still the right answer (and is what a caller would expect to see).
        return ink.clone();
    }
    let mut page = background.clone();
    for (target, source) in page.data_mut().iter_mut().zip(ink.data().iter()) {
        let alpha = source.a as u16;
        if alpha == 0 {
            continue;
        }
        let inverse = 255 - alpha;
        let blend = |ink_channel: u8, paper_channel: u8| -> u8 {
            (ink_channel as u16 + (paper_channel as u16 * inverse + 127) / 255).min(255) as u8
        };
        target.r = blend(source.r, target.r);
        target.g = blend(source.g, target.g);
        target.b = blend(source.b, target.b);
        target.a = blend(alpha as u8, target.a);
    }
    page
}

/// The rasterized page as PNG bytes (for WinUI's `Image` and for export).
pub fn to_png(pixmap: &Pixmap) -> Result<Vec<u8>, String> {
    pixmap
        .clone()
        .into_png()
        .map_err(|error| format!("PNG encode failed: {error}"))
}

/// Decodes PNG bytes back into a bitmap (tests, and reading an exported page).
pub fn from_png(bytes: &[u8]) -> Result<Pixmap, String> {
    Pixmap::from_png(std::io::Cursor::new(bytes))
        .map_err(|error| format!("PNG decode failed: {error}"))
}

/// One pixel, unpremultiplied, for tests and for the dev panel.
pub fn pixel_at(pixmap: &Pixmap, x: u16, y: u16) -> Rgba {
    let pixel = pixmap.sample(x, y);
    Rgba::rgba(
        unpremultiply(pixel.r, pixel.a),
        unpremultiply(pixel.g, pixel.a),
        unpremultiply(pixel.b, pixel.a),
        pixel.a,
    )
}

fn unpremultiply(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    let value = (channel as u32 * 255 + alpha as u32 / 2) / alpha as u32;
    value.min(255) as u8
}

/// How many pixels carry ink (a cheap "did anything change" check).
///
/// "Ink" is a pixel that is neither transparent nor white paper.  A rendered PDF
/// page is opaque white *everywhere*, so counting every non-transparent pixel
/// would report a blank page as fully inked (and the export tests could not tell
/// "this page has a stroke" from "this page is empty").
pub fn ink_coverage(pixmap: &Pixmap) -> usize {
    pixmap
        .data()
        .iter()
        .filter(|pixel| pixel.a > 0 && !(pixel.r > 250 && pixel.g > 250 && pixel.b > 250))
        .count()
}

/// An ink color in the rasterizer's color type.
pub fn to_alpha_color(color: Rgba) -> vello_cpu::peniko::color::AlphaColor<Srgb> {
    vello_cpu::peniko::color::AlphaColor::from_rgba8(color.r, color.g, color.b, color.a)
}

/// A blank white page bitmap (the background of a note that has no PDF).
pub fn white_page(size: Size, scale: Scale) -> Pixmap {
    let (width, height) = size.to_pixels(scale);
    let mut pixmap = Pixmap::new(width, height);
    let white = vello_cpu::peniko::color::PremulRgba8 {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
    for y in 0..height {
        for x in 0..width {
            pixmap.set_pixel(x, y, white);
        }
    }
    pixmap
}
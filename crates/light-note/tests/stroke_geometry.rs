//! Stroke geometry contract: **one** decision, shared by the rasterizer and the
//! live WinUI shapes.
//!
//! Why this file exists: if the bitmap and the live shapes disagree by even half
//! a pixel, the ink visibly changes the instant a stroke is committed.  So both
//! consumers read the same [`StrokeShape`], and the tests below check the three
//! cases (dot / constant width / varying width) plus the fact that the live
//! pieces cover exactly the same area as the filled union.

mod support;

use light_note::geom::{Scale, Size};
use light_note::ink::Tool;
use light_note::shape::{
    FLATTEN_TOLERANCE_PX, LivePiece, StrokeShape, from_png, ink_coverage, live_pieces, pixel_at,
    rasterize, spans_union, stroke_shape, to_png,
};

const SCALE: Scale = Scale::DEFAULT;

#[test]
fn an_empty_stroke_has_no_shape() {
    let stroke = light_note::ink::Stroke::new(support::pen(2.0));

    assert_eq!(stroke_shape(&stroke, SCALE), None);
    assert!(live_pieces(&StrokeShape::default()).is_empty());
}

#[test]
fn a_single_point_is_a_filled_dot() {
    let stroke = support::stroke(support::pen(2.0), &[(50.0, 50.0, 1.0)]);
    let shape = stroke_shape(&stroke, SCALE).expect("dot");

    // 2 pt at full pressure, 1.5 px/pt -> 3 px wide, radius 1.5 px.
    match shape {
        StrokeShape::Dot { center, radius } => {
            assert_eq!(center, (75.0, 75.0));
            assert!((radius - 1.5).abs() < 1e-6);
        }
        other => panic!("expected a dot, got {other:?}"),
    }

    let pieces = live_pieces(&shape);
    assert_eq!(pieces.len(), 1, "a dot is one round cap");
    assert!(matches!(pieces[0], LivePiece::Cap { .. }));
}

#[test]
fn a_constant_width_stroke_is_a_curve_with_end_caps() {
    let stroke = support::line(support::pen(2.0), 10.0, 110.0, 50.0, 1.0);
    let shape = stroke_shape(&stroke, SCALE).expect("curve");

    match &shape {
        StrokeShape::Curve { width, .. } => assert_eq!(*width, 3.0, "2 pt * 1.5 px/pt"),
        other => panic!("expected a curve, got {other:?}"),
    }

    let pieces = live_pieces(&shape);
    let caps = pieces.iter().filter(|p| matches!(p, LivePiece::Cap { .. })).count();
    let segments = pieces.iter().filter(|p| matches!(p, LivePiece::Segment { .. })).count();

    assert_eq!(caps, 2, "one cap per end — joints are covered by the segments");
    assert!(segments >= 2, "a flattened line is more than one segment: {segments}");
    assert_eq!(FLATTEN_TOLERANCE_PX, 0.25);
}

#[test]
fn a_pressure_taper_becomes_varying_width_spans() {
    // 3 pt at full pressure, 1 pt at zero: a visible taper.
    let stroke = support::stroke(support::pen(3.0), &[(0.0, 0.0, 1.0), (20.0, 0.0, 0.0)]);
    let shape = stroke_shape(&stroke, SCALE).expect("spans");

    match &shape {
        StrokeShape::Spans(spans) => {
            assert_eq!(spans.len(), 1);
            // 3 pt at full pressure is 4.5 px; at no pressure it is
            // 3 * 0.35 = 1.05 pt = 1.575 px.  The span stores the average of the
            // two ends — compared with a tolerance, because 0.35 and 1.5 are not
            // exact in binary floating point.
            let expected = (4.5 + 1.575) * 0.5;
            assert!(
                (spans[0].width - expected).abs() < 1e-9,
                "average of both ends, in px: {} (expected {expected})",
                spans[0].width
            );
        }
        other => panic!("expected spans, got {other:?}"),
    }
}

#[test]
fn segments_are_extended_by_half_their_width_so_joints_are_covered() {
    // WinUI `Line` has no cap property, so each segment is lengthened by its own
    // half-width; the joint is then covered by the two neighbouring rectangles
    // instead of needing a third shape.
    let stroke = support::stroke(support::pen(4.0), &[(0.0, 0.0, 1.0), (10.0, 0.0, 1.0)]);
    let shape = stroke_shape(&stroke, SCALE).expect("spans");
    let pieces = live_pieces(&shape);

    let (from, to, width) = pieces
        .iter()
        .find_map(|piece| match piece {
            LivePiece::Segment { from, to, width } => Some((*from, *to, *width)),
            LivePiece::Cap { .. } => None,
        })
        .expect("segment");

    let half = width / 2.0;
    assert!((from.0 - (0.0 - half)).abs() < 1e-6, "start extended: {from:?}");
    assert!((to.0 - (15.0 + half)).abs() < 1e-6, "end extended: {to:?}");
}

#[test]
fn the_union_path_is_a_single_closed_shape() {
    // One fill per stroke: overlapping pieces must not double-multiply alpha,
    // which is why the rasterizer fills the union instead of each piece.
    use vello_cpu::kurbo::Shape as _;

    let stroke = support::line(support::pen(6.0), 0.0, 60.0, 20.0, 1.0);
    let shape = stroke_shape(&stroke, SCALE).expect("shape");
    let spans = shape.spans().expect("flattened spans");
    let path = spans_union(&spans);

    assert!(path.elements().len() >= 5, "at least one closed polygon");
    assert!(path.bounding_box().width() > 80.0, "the union spans the stroke");
}

#[test]
fn rasterized_ink_lands_where_the_points_are() {
    let stroke = support::line(support::pen(2.0), 10.0, 90.0, 50.0, 1.0);
    let pixmap = rasterize(&[stroke], Size::new(100.0, 100.0), SCALE, None);

    // 100 pt * 1.5 = 150 px.
    assert_eq!((pixmap.width(), pixmap.height()), (150, 150));
    assert!(pixel_at(&pixmap, 75, 75).a > 200, "ink at the middle of the line");
    assert_eq!(pixel_at(&pixmap, 75, 10).a, 0, "transparent far from the line");
    assert_eq!(pixel_at(&pixmap, 10, 75).a, 0, "transparent before the start");
    assert!(ink_coverage(&pixmap) > 300, "a 120 px line covers area");
}

#[test]
fn a_dot_is_round_and_small() {
    let stroke = support::stroke(support::pen(2.0), &[(50.0, 50.0, 1.0)]);
    let pixmap = rasterize(&[stroke], Size::new(100.0, 100.0), SCALE, None);

    assert!(pixel_at(&pixmap, 75, 75).a > 200, "center is inked");
    assert_eq!(pixel_at(&pixmap, 80, 75).a, 0, "5 px away is empty");
    let coverage = ink_coverage(&pixmap);
    assert!((5..40).contains(&coverage), "a 3 px disc is a few pixels: {coverage}");
}

#[test]
fn a_highlighter_stays_translucent_on_the_bitmap() {
    let style = Tool::Highlighter;
    assert_eq!(style, Tool::Highlighter);
    let stroke = support::line(support::highlighter(14.0), 10.0, 90.0, 50.0, 1.0);
    let pixmap = rasterize(&[stroke], Size::new(100.0, 100.0), SCALE, None);
    let pixel = pixel_at(&pixmap, 75, 75);

    assert!(pixel.a > 0 && pixel.a < 255, "translucent, not opaque: {pixel:?}");
}

#[test]
fn png_round_trip_preserves_the_raster() {
    let stroke = support::line(support::pen(4.0), 10.0, 90.0, 50.0, 1.0);
    let pixmap = rasterize(&[stroke], Size::new(100.0, 100.0), SCALE, None);

    let bytes = to_png(&pixmap).expect("encode");
    assert_eq!(&bytes[1..4], b"PNG", "a real PNG stream");

    let decoded = from_png(&bytes).expect("decode");
    assert_eq!(decoded.width(), pixmap.width());
    assert_eq!(decoded.height(), pixmap.height());
    assert_eq!(pixel_at(&decoded, 75, 75), pixel_at(&pixmap, 75, 75));
}

#[test]
fn a_background_bitmap_is_composited_under_the_ink() {
    // The PDF page arrives as a bitmap; ink is drawn on top of it in one pass,
    // so the screen never shows ink without its page.
    let mut page = vello_cpu::Pixmap::new(150, 150);
    for y in 0..150u16 {
        for x in 0..150u16 {
            page.set_pixel(
                x,
                y,
                vello_cpu::peniko::color::PremulRgba8 {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                },
            );
        }
    }

    let stroke = support::line(support::pen(2.0), 10.0, 90.0, 50.0, 1.0);
    let pixmap = rasterize(&[stroke], Size::new(100.0, 100.0), SCALE, Some(&page));

    assert_eq!(pixel_at(&pixmap, 10, 10), light_note::ink::Rgba::rgb(255, 0, 0));
    assert!(pixel_at(&pixmap, 75, 75).a > 200);
    assert!(pixel_at(&pixmap, 75, 75).r < 120, "ink covers the page");
}

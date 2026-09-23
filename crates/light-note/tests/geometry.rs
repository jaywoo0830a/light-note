//! Geometry contract: page points in, pixels out.
//!
//! Two units exist in the whole app — **pt** (model: pages, ink) and **px**
//! (drawing: WinUI shapes, rasterized bitmaps).  Exactly one value connects
//! them: `scale = DEFAULT_SCALE * zoom`, where `DEFAULT_SCALE` is 1.5 (a
//! 1.5x screen for a 96 dpi display).  These tests pin that conversion down,
//! because every other layer assumes it.

mod support;

use light_note::geom::{Pt, Rect, Scale, Size, DEFAULT_SCALE};

#[test]
fn default_scale_is_one_and_a_half_pixels_per_point() {
    assert_eq!(DEFAULT_SCALE, 1.5);
    assert_eq!(Scale::DEFAULT.get(), 1.5);
}

#[test]
fn scale_converts_between_points_and_pixels() {
    let scale = Scale::new(2.0);

    assert_eq!(scale.px(10.0), 20.0);
    assert_eq!(scale.pt(20.0), 10.0);
    assert_eq!(scale.px_point(Pt::new(3.0, 4.0)), (6.0, 8.0));
}

#[test]
fn scale_rejects_non_positive_and_non_finite_values() {
    // A zoom of 0 or NaN would make every downstream division meaningless, so
    // the type clamps instead of propagating garbage.
    assert_eq!(Scale::new(0.0).get(), Scale::MIN.get());
    assert_eq!(Scale::new(-3.0).get(), Scale::MIN.get());
    assert_eq!(Scale::new(f32::NAN).get(), Scale::MIN.get());
    assert_eq!(Scale::new(f32::INFINITY).get(), Scale::MAX.get());
    assert_eq!(Scale::new(1e9).get(), Scale::MAX.get());
}

#[test]
fn scale_from_zoom_is_percent_based() {
    assert_eq!(Scale::from_zoom(100.0).get(), DEFAULT_SCALE);
    assert_eq!(Scale::from_zoom(200.0).get(), DEFAULT_SCALE * 2.0);
    assert_eq!(Scale::from_zoom(50.0).get(), DEFAULT_SCALE * 0.5);
}

#[test]
fn size_rounds_to_pixels_for_bitmaps() {
    let size = Size::new(595.276, 841.89);

    // 595.276 * 1.5 = 892.914 -> 893, 841.89 * 1.5 = 1262.835 -> 1263.
    assert_eq!(size.to_pixels(Scale::DEFAULT), (893, 1263));

    // Never zero: a bitmap of width 0 is not a bitmap.
    assert_eq!(Size::new(0.1, 0.1).to_pixels(Scale::new(1.0)), (1, 1));
}

#[test]
fn size_keeps_the_page_aspect_ratio_when_fitted() {
    let size = Size::new(200.0, 100.0);
    let fitted = size.fit_into(Size::new(100.0, 100.0));

    assert_eq!(fitted, Size::new(100.0, 50.0), "width-bound fit");
    let fitted = size.fit_into(Size::new(1000.0, 100.0));
    assert_eq!(fitted, Size::new(200.0, 100.0), "height-bound fit");
    // Degenerate targets never produce a negative or NaN size.
    let fitted = size.fit_into(Size::new(0.0, 0.0));
    assert!(fitted.width >= 0.0 && fitted.height >= 0.0);
}

#[test]
fn point_distance_is_euclidean() {
    let a = Pt::new(0.0, 0.0);
    let b = Pt::new(3.0, 4.0);

    assert_eq!(a.distance_to(b), 5.0);
    assert_eq!(b.distance_to(a), 5.0);
    assert_eq!(a.distance_to(a), 0.0);
}

#[test]
fn rect_contains_expands_and_unions() {
    let rect = Rect::new(10.0, 10.0, 20.0, 20.0);

    assert!(rect.contains(Pt::new(10.0, 10.0)), "top-left is inside");
    assert!(rect.contains(Pt::new(30.0, 30.0)), "bottom-right is inside");
    assert!(!rect.contains(Pt::new(30.1, 20.0)));
    assert!(!rect.contains(Pt::new(5.0, 20.0)));

    let padded = rect.expand(5.0);
    assert_eq!(padded, Rect::new(5.0, 5.0, 30.0, 30.0));
    assert!(padded.contains(Pt::new(5.0, 5.0)));

    let union = rect.union(Rect::new(50.0, 50.0, 10.0, 10.0));
    assert_eq!(union, Rect::new(10.0, 10.0, 50.0, 50.0));
}

#[test]
fn rect_intersects_reports_overlap_only() {
    let rect = Rect::new(0.0, 0.0, 10.0, 10.0);

    assert!(rect.intersects(Rect::new(5.0, 5.0, 10.0, 10.0)));
    assert!(rect.intersects(Rect::new(9.9, 0.0, 1.0, 1.0)));
    assert!(!rect.intersects(Rect::new(10.0, 0.0, 1.0, 1.0)), "touching is not overlap");
    assert!(!rect.intersects(Rect::new(-20.0, 0.0, 10.0, 10.0)));
}

#[test]
fn rect_from_points_covers_every_point() {
    let rect = Rect::from_points(&[Pt::new(5.0, 2.0), Pt::new(-1.0, 8.0), Pt::new(3.0, 3.0)]);

    assert_eq!(rect, Rect::new(-1.0, 2.0, 6.0, 6.0));
    assert!(rect.contains(Pt::new(5.0, 2.0)));
    assert!(rect.contains(Pt::new(-1.0, 8.0)));

    // An empty set has an empty rect (no NaN, no panic).
    let empty = Rect::from_points(&[]);
    assert_eq!(empty.width, 0.0);
    assert_eq!(empty.height, 0.0);
}

#[test]
fn rect_center_is_the_middle() {
    assert_eq!(Rect::new(0.0, 0.0, 10.0, 4.0).center(), Pt::new(5.0, 2.0));
}

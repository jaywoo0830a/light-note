//! Tablet → page mapping contract.
//!
//! OTD reports **device units** (raw tablet coordinates) because the plugin
//! cannot know the window size or the zoom level.  Turning those into page
//! points is the app's job, and it has to be stable: the mapping must not move
//! when the window is resized, or the ink would land somewhere else than where
//! it was written.

mod support;

use light_note::geom::{Pt, Size};
use light_note::otd::map::{PageMap, TabletSpec};

fn intuos() -> TabletSpec {
    TabletSpec::new("Wacom Intuos Pro M", 63_460.0, 39_660.0, 8192.0)
}

#[test]
fn a_tablet_without_a_known_range_cannot_be_mapped() {
    let unknown = TabletSpec::new("Unknown", 0.0, 0.0, 0.0);

    assert!(!unknown.is_known());
    assert_eq!(PageMap::fit(&unknown, support::A4, 24.0), None);
    assert_eq!(PageMap::fit(&intuos(), Size::new(0.0, 0.0), 24.0), None);
}

#[test]
fn the_mapping_is_a_centered_fit_that_keeps_the_aspect_ratio() {
    let map = PageMap::fit(&intuos(), support::A4, 24.0).expect("fit");
    let (left, top) = map.origin();
    let (right, bottom) = map.far_corner();

    // The tablet is much wider than A4, so the fit is width-bound.
    assert!((left - 24.0).abs() < 0.01, "the margin is respected on the left");
    assert!((right - (support::A4.width - 24.0)).abs() < 0.01);
    assert!(top > 24.0, "the ink area is centered vertically: {top}");
    assert!(bottom < support::A4.height - 24.0);
    assert!(
        ((bottom - top) - (right - left) * (39_660.0 / 63_460.0)).abs() < 0.5,
        "the tablet aspect ratio is preserved"
    );
}

#[test]
fn the_corners_of_the_tablet_map_to_the_corners_of_the_ink_area() {
    let map = PageMap::fit(&intuos(), support::A4, 24.0).expect("fit");
    let (left, top) = map.origin();
    let (right, bottom) = map.far_corner();

    assert_eq!(map.to_page(0.0, 0.0), Pt::new(left, top));
    assert_eq!(map.to_page(63_460.0, 39_660.0), Pt::new(right, bottom));

    // The middle of the tablet is the middle of the ink area.
    let middle = map.to_page(31_730.0, 19_830.0);
    assert!((middle.x - (left + right) / 2.0).abs() < 0.01);
    assert!((middle.y - (top + bottom) / 2.0).abs() < 0.01);
}

#[test]
fn a_mapping_does_not_depend_on_the_window_size() {
    // Same tablet, same page, same margin -> same mapping, no matter when it is
    // computed.  This is what keeps the ink where the pen was.
    let first = PageMap::fit(&intuos(), support::A4, 24.0).expect("fit");
    let second = PageMap::fit(&intuos(), support::A4, 24.0).expect("fit");

    assert_eq!(first, second);
    assert_eq!(first.to_page(1234.0, 567.0), second.to_page(1234.0, 567.0));
}

#[test]
fn coordinates_outside_the_tablet_are_clamped_into_the_page() {
    let map = PageMap::fit(&intuos(), support::A4, 24.0).expect("fit");
    let (left, top) = map.origin();
    let (right, bottom) = map.far_corner();

    let beyond = map.to_page(999_999.0, -999_999.0);
    assert_eq!(beyond.x, right);
    assert_eq!(beyond.y, top);

    let behind = map.to_page(-1.0, 999_999.0);
    assert_eq!(behind.x, left);
    assert_eq!(behind.y, bottom);
}

#[test]
fn raw_pressure_becomes_a_unit_range() {
    let map = PageMap::fit(&intuos(), support::A4, 24.0).expect("fit");

    assert_eq!(map.pressure(0.0), 0.0);
    assert_eq!(map.pressure(4096.0), 0.5);
    assert_eq!(map.pressure(8192.0), 1.0);
    assert_eq!(map.pressure(20_000.0), 1.0, "clamped");
    assert_eq!(map.pressure(-5.0), 0.0, "clamped");
    assert_eq!(map.pressure(f32::NAN), 0.0);
}

#[test]
fn a_tablet_that_reports_no_pressure_falls_back_to_full_pressure() {
    // No pressure hardware at all: the ink still has to appear, so the caller
    // gets 1.0 and the pen width is whatever the nib says.
    let spec = TabletSpec::new("No pressure", 63_460.0, 39_660.0, 0.0);
    let map = PageMap::fit(&spec, support::A4, 24.0).expect("fit");

    assert_eq!(map.pressure(1234.0), 1.0);
}

#[test]
fn the_spec_keeps_the_tablet_name_for_the_status_bar() {
    let spec = intuos();

    assert_eq!(spec.name, "Wacom Intuos Pro M");
    assert!(spec.is_known());
}

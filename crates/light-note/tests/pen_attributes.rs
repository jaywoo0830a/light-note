//! Pen attributes: what OTD's shared memory says about the pen, and what a
//! stroke does with it.
//!
//! Two things are pinned here:
//!
//! 1. **the position is not part of this** — the ink is placed at the canvas
//!    pointer's coordinates, so nothing in this file maps a tablet coordinate to
//!    a page point.  What the stream contributes is pressure, tilt, the barrel's
//!    rotation and the eraser flag;
//! 2. **tilt changes the mark** — a pen lying flat draws wider, and a flat nib
//!    turned by the barrel draws a thin line along its edge and a thick one
//!    across it.

mod support;

use light_note::geom::Pt;
use light_note::ink::{MAX_TILT_DEG, Nib, TILT_WIDTH_GAIN, heading, width_at};
use light_note::otd::{PenState, Sample, TabletSpec};

/// An XP-Pen Deco 01 V3, the tablet this project was developed against.
fn deco() -> TabletSpec {
    TabletSpec::new("XP-Pen Deco 01 V3 (Variant 2)", 51_196.0, 31_826.0, 16_383.0)
}

/// A report with the given raw values.
fn report(pressure: f32, tilt_x: f32, tilt_y: f32, rotation: f32, flags: u32) -> Sample {
    Sample {
        seq: 1,
        x: 1234.0,
        y: 567.0,
        pressure,
        tilt_x,
        tilt_y,
        rotation,
        flags,
        hover_distance: 0,
        time: 100,
    }
}

#[test]
fn a_tablet_without_a_known_range_cannot_scale_pressure() {
    let unknown = TabletSpec::new("Unknown", 0.0, 0.0, 0.0);
    let down = report(4096.0, 0.0, 0.0, 0.0, support::flag::TIP);

    assert!(!unknown.is_known());
    assert_eq!(
        PenState::from_sample(&down, Some(&unknown)).pressure,
        1.0,
        "no range means no pressure sensor: the ink still has to appear"
    );
}

#[test]
fn raw_pressure_becomes_a_unit_range() {
    let spec = deco();

    assert_eq!(spec.pressure(0.0), 0.0);
    assert_eq!(spec.pressure(8191.5), 0.5);
    assert_eq!(spec.pressure(16_383.0), 1.0);
    assert_eq!(spec.pressure(99_999.0), 1.0, "clamped");
    assert_eq!(spec.pressure(-5.0), 0.0, "clamped");
    assert_eq!(spec.pressure(f32::NAN), 0.0);
}

#[test]
fn the_spec_keeps_the_tablet_name_for_the_status_bar() {
    let spec = deco();

    assert_eq!(spec.name, "XP-Pen Deco 01 V3 (Variant 2)");
    assert!(spec.is_known());
}

#[test]
fn a_report_becomes_pressure_tilt_and_the_eraser_flag() {
    let sample = report(
        8191.5,
        12.0,
        -4.0,
        30.0,
        support::flag::TIP | support::flag::ERASER,
    );
    let pen = PenState::from_sample(&sample, Some(&deco()));

    assert_eq!(pen.pressure, 0.5);
    assert_eq!(pen.nib.tilt_x, 12.0);
    assert_eq!(pen.nib.tilt_y, -4.0);
    assert_eq!(pen.nib.rotation, 30.0);
    assert!(pen.touching);
    assert!(pen.erasing);
    assert_eq!(pen.buttons, 0, "nothing on the barrel is pressed");

    // A barrel button is a fact the app reports, so it has to survive the trip.
    let pressed = report(
        8191.5,
        0.0,
        0.0,
        0.0,
        support::flag::TIP | support::flag::BUTTON_1,
    );
    assert_eq!(PenState::from_sample(&pressed, Some(&deco())).buttons, 1);

    // A hover report touches nothing, so it must not ink.
    let hover = report(8191.5, 0.0, 0.0, 0.0, 0);
    let pen = PenState::from_sample(&hover, Some(&deco()));
    assert!(!pen.touching);
    assert!(!pen.erasing);
}

#[test]
fn an_upright_pen_has_no_tilt_effect() {
    let upright = Nib::UPRIGHT;

    assert!(upright.is_upright());
    assert_eq!(upright.flatness(), 0.0);
    assert_eq!(upright.axis(), None, "a round nib has no edge");
    assert_eq!(upright.width_ratio(None), 1.0);
    assert_eq!(upright.width_ratio(Some(0.75)), 1.0);
}

#[test]
fn a_flat_nib_is_a_chisel() {
    // Lying flat towards +x, no barrel rotation: the edge is along x, so a stroke
    // along x is thin and a stroke across it is thick.
    let nib = Nib::new(MAX_TILT_DEG, 0.0, 0.0);
    let style = support::pen(4.0);

    assert_eq!(nib.flatness(), 1.0, "90° of tilt is as flat as a pen gets");
    assert_eq!(nib.axis(), Some(0.0));
    assert!(
        (width_at(&style, 1.0, nib, None) - 4.0 * (1.0 + TILT_WIDTH_GAIN)).abs() < 1e-9,
        "with no heading the widest footprint is used"
    );

    let along = width_at(&style, 1.0, nib, Some(0.0));
    let across = width_at(&style, 1.0, nib, Some(std::f32::consts::FRAC_PI_2));

    assert!((along - 4.0).abs() < 1e-6, "along the edge: the nib's thickness");
    assert!((across - 4.0 * 1.6).abs() < 1e-6, "across it: the full width");
    assert!(across > along);
}

#[test]
fn the_barrel_rotation_turns_the_nib() {
    let straight = Nib::new(MAX_TILT_DEG, 0.0, 0.0);
    let turned = Nib::new(MAX_TILT_DEG, 0.0, 90.0);
    let style = support::pen(4.0);

    assert_eq!(turned.axis(), Some(std::f32::consts::FRAC_PI_2));
    // What was thin for one grip is thick for the other.
    assert!(width_at(&style, 1.0, straight, Some(0.0)) < width_at(&style, 1.0, turned, Some(0.0)));
    // Half a turn is the same nib: a flat edge has no front and back.
    let half_turn = Nib::new(MAX_TILT_DEG, 0.0, 180.0).width_ratio(Some(0.3));
    assert!(
        (half_turn - straight.width_ratio(Some(0.3))).abs() < 1e-6,
        "180° of barrel rotation is the same nib, got {half_turn}"
    );
}

#[test]
fn tilt_is_bounded_even_when_the_report_is_not() {
    assert_eq!(Nib::new(200.0, 0.0, 0.0).flatness(), 1.0, "clamped to flat");
    assert_eq!(
        Nib::new(f32::NAN, 0.0, 0.0).flatness(),
        0.0,
        "a wild report is ignored, never propagated"
    );
    assert_eq!(
        Nib::new(0.0, 0.0, f32::NAN).width_ratio(None),
        1.0,
        "an upright pen is upright whatever its rotation says"
    );
}

#[test]
fn a_highlighter_ignores_tilt() {
    // A felt marker's line does not change with the hand: only the tool's width
    // does.
    let nib = Nib::new(MAX_TILT_DEG, 0.0, 0.0);
    let style = support::highlighter(14.0);

    assert_eq!(width_at(&style, 0.2, nib, Some(0.0)), 14.0);
    assert_eq!(width_at(&style, 0.2, nib, None), 14.0);
}

#[test]
fn a_heading_exists_only_when_the_pen_moved() {
    assert_eq!(heading(Pt::new(5.0, 5.0), Pt::new(5.0, 5.0)), None);
    assert_eq!(heading(Pt::new(0.0, 0.0), Pt::new(2.0, 0.0)), Some(0.0));
    assert_eq!(
        heading(Pt::new(0.0, 0.0), Pt::new(0.0, 2.0)),
        Some(std::f32::consts::FRAC_PI_2),
        "y grows downwards, the page's own direction"
    );
}

#[test]
fn a_report_before_the_tablet_is_known_is_full_pressure() {
    // The daemon may not have answered yet: a raw pressure with no range to
    // divide it by is read as full pressure, never as a number invented out of
    // thin air.
    let down = PenState::from_sample(&report(4096.0, 0.0, 0.0, 0.0, support::flag::TIP), None);
    assert_eq!(down.pressure, 1.0);

    let hover = PenState::from_sample(&report(0.0, 0.0, 0.0, 0.0, 0), None);
    assert_eq!(hover.pressure, 0.0);
    assert!(!hover.touching);
}

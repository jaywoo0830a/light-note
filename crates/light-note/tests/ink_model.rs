//! Ink contract: how a pen sample becomes a point of a stroke.
//!
//! The writing feel lives here.  Three decisions matter and all three are
//! pinned by numbers below:
//!
//! 1. **Width follows pressure** — `width_pt * (0.35 + 0.65 * pressure)`.
//!    A pen that reports no pressure still produces a visible, thinner line.
//! 2. **Pressure is smoothed** — an exponential filter, so a noisy digitizer
//!    does not make the line wobble.
//! 3. **Samples are decimated** — points closer than 0.4 pt are dropped unless
//!    the pressure actually moved.  A 400 Hz stream stays cheap without
//!    flattening the pressure curve.

mod support;

use light_note::geom::Pt;
use light_note::ink::{
    InkPoint, MIN_SAMPLE_DISTANCE_PT, MIN_WIDTH_RATIO, PRESSURE_SMOOTHING, Stroke, Style, Tool,
    keep_sample, pressure_from_speed, smooth_pressure, width_at,
};

/// The nib a plain pressure test uses: upright (no tilt, no rotation) and a
/// heading nobody knows — which is the one case where the width is exactly the
/// pressure formula.
fn width(style: &Style, pressure: f32) -> f64 {
    width_at(style, pressure, light_note::ink::Nib::UPRIGHT, None)
}

#[test]
fn tools_that_write_ink_are_pen_and_highlighter() {
    assert!(Tool::Pen.writes_ink());
    assert!(Tool::Highlighter.writes_ink());
    assert!(!Tool::Eraser.writes_ink());
}

#[test]
fn eraser_removes_instead_of_drawing() {
    assert!(Tool::Eraser.erases());
    assert!(!Tool::Pen.erases());
    assert!(!Tool::Highlighter.erases());
}

#[test]
fn pen_width_scales_with_pressure_from_a_visible_floor() {
    let style = support::pen(2.0);

    assert_eq!(MIN_WIDTH_RATIO, 0.35);
    assert_eq!(width(&style, 0.0), 0.7, "no pressure is still visible");
    assert_eq!(width(&style, 1.0), 2.0, "full pressure is the full width");
    assert_eq!(width(&style, 0.5), 2.0 * 0.675);
}

#[test]
fn width_clamps_out_of_range_pressure() {
    let style = support::pen(2.0);

    assert_eq!(width(&style, -1.0), width(&style, 0.0));
    assert_eq!(width(&style, 4.0), width(&style, 1.0));
    assert_eq!(width(&style, f32::NAN), width(&style, 0.0));
}

#[test]
fn highlighter_keeps_a_constant_width() {
    // A highlighter is a marker: the nib is felt, so the line is not pressure
    // sensitive.  Pressure is still recorded, it just does not change width.
    let style = support::highlighter(14.0);

    assert_eq!(width(&style, 0.0), 14.0);
    assert_eq!(width(&style, 0.5), 14.0);
    assert_eq!(width(&style, 1.0), 14.0);
}

#[test]
fn pressure_from_speed_is_an_inverse_curve() {
    // No pressure hardware?  Then speed is the signal: slow = wide, fast = thin.
    assert_eq!(pressure_from_speed(0.0), 1.0);
    assert_eq!(pressure_from_speed(1.25), 0.5);
    assert_eq!(pressure_from_speed(2.5), 0.15, "at the floor");
    assert_eq!(pressure_from_speed(50.0), 0.15, "never below the floor");
    assert_eq!(pressure_from_speed(-1.0), 1.0, "never above 1");
}

#[test]
fn pressure_smoothing_moves_a_fraction_of_the_way() {
    assert_eq!(PRESSURE_SMOOTHING, 0.35);
    assert_eq!(smooth_pressure(0.2, 0.8), 0.2 + 0.6 * 0.35);
    assert_eq!(smooth_pressure(0.5, 0.5), 0.5);
    assert!(smooth_pressure(0.0, 1.0) < 1.0, "one sample never jumps the whole way");
    assert_eq!(smooth_pressure(0.5, 2.0), smooth_pressure(0.5, 1.0), "clamped first");
}

#[test]
fn samples_closer_than_the_minimum_distance_are_dropped() {
    assert_eq!(MIN_SAMPLE_DISTANCE_PT, 0.4);

    let last = InkPoint::new(Pt::new(10.0, 10.0), 0.5, 0.0);
    let too_close = InkPoint::new(Pt::new(10.2, 10.0), 0.5, 2.5);
    assert!(!keep_sample(&last, &too_close), "0.2 pt is below the floor");

    let far_enough = InkPoint::new(Pt::new(10.5, 10.0), 0.5, 5.0);
    assert!(keep_sample(&last, &far_enough));
}

#[test]
fn a_pressure_change_survives_the_distance_filter() {
    // "Close but different pressure" is exactly the sample that makes a stroke
    // look alive, so it is kept even though it sits near the previous point.
    let last = InkPoint::new(Pt::new(10.0, 10.0), 0.2, 0.0);
    let same_place_new_pressure = InkPoint::new(Pt::new(10.1, 10.0), 0.6, 2.5);
    assert!(keep_sample(&last, &same_place_new_pressure));

    // A tiny pressure wobble (below the delta) is still dropped.
    let wobble = InkPoint::new(Pt::new(10.1, 10.0), 0.205, 2.5);
    assert!(!keep_sample(&last, &wobble));
}

#[test]
fn a_stroke_keeps_its_first_sample_no_matter_what() {
    let mut stroke = Stroke::new(support::pen(2.0));

    assert!(stroke.is_empty());
    assert_eq!(stroke.len(), 0);

    stroke.push(InkPoint::new(Pt::new(5.0, 5.0), 0.0, 0.0));
    assert_eq!(stroke.len(), 1);

    // The same position and pressure again: dropped — a tap must stay a dot.
    stroke.push(InkPoint::new(Pt::new(5.0, 5.0), 0.0, 2.5));
    assert_eq!(stroke.len(), 1);
}

#[test]
fn a_stroke_records_its_own_style_and_points() {
    let stroke = support::stroke(support::pen(3.0), &[(0.0, 0.0, 1.0), (10.0, 0.0, 0.5)]);

    assert_eq!(stroke.style().tool, Tool::Pen);
    assert_eq!(stroke.style().width_pt, 3.0);
    assert_eq!(stroke.len(), 2);
    assert_eq!(stroke.points()[1].pos, Pt::new(10.0, 0.0));
    assert_eq!(stroke.points()[1].pressure, 0.5);
}

#[test]
fn stroke_bounds_cover_every_point_with_the_nib_width() {
    let stroke = support::stroke(support::pen(2.0), &[(10.0, 10.0, 1.0), (30.0, 20.0, 1.0)]);
    let bounds = stroke.bounds();

    // A 2 pt nib at full pressure leaves 1 pt of ink on each side.
    assert_eq!(bounds, light_note::geom::Rect::new(9.0, 9.0, 22.0, 12.0));
    assert!(bounds.contains(Pt::new(30.0, 20.0)));

    let empty = Stroke::new(support::pen(2.0)).bounds();
    assert_eq!(empty.width, 0.0, "an empty stroke has an empty box");
}

#[test]
fn style_is_per_stroke_and_the_highlighter_is_translucent() {
    let pen = support::pen(2.0);
    let marker = support::highlighter(14.0);

    assert_ne!(pen.color, marker.color, "a highlighter is not ink-black");
    assert!(marker.color.a < pen.color.a, "a highlighter is translucent");
    assert_eq!(Style::new(Tool::Pen, 2.0).width_pt, 2.0);
}

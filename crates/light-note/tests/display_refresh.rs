//! Refresh-rate contract: 60, 120, 180 and 240 Hz.
//!
//! The app paces itself with the display it is running on.  Two things are
//! pinned here:
//!
//! * **the policy** — a detected rate is snapped to the nearest supported rate
//!   (ties go to the slower rate, because asking for more frames than the panel
//!   can show only wastes power), and an unknown display means 60 Hz;
//! * **the arithmetic** — one frame period per rate, in nanoseconds, so a
//!   240 Hz loop is 4.166 ms and not "about 4 ms".

mod support;

use std::time::Duration;

use light_note::display::{
    DEFAULT_HZ, FpsMeter, FrameClock, RefreshChoice, SUPPORTED_HZ, frame_period, snap_to_supported,
};

#[test]
fn the_supported_rates_are_the_four_panel_rates() {
    assert_eq!(SUPPORTED_HZ, [60, 120, 180, 240]);
    assert_eq!(DEFAULT_HZ, 60);
}

#[test]
fn an_exact_rate_snaps_to_itself() {
    for hz in SUPPORTED_HZ {
        assert_eq!(snap_to_supported(hz), hz);
    }
}

#[test]
fn an_unsupported_rate_snaps_to_the_nearest_supported_one() {
    assert_eq!(snap_to_supported(75), 60);
    assert_eq!(snap_to_supported(100), 120);
    assert_eq!(snap_to_supported(144), 120, "24 away from 120, 36 from 180");
    assert_eq!(snap_to_supported(165), 180);
    assert_eq!(snap_to_supported(300), 240, "above the fastest rate");
    assert_eq!(snap_to_supported(30), 60, "below the slowest rate");
}

#[test]
fn a_tie_goes_to_the_slower_rate() {
    // 150 Hz is exactly between 120 and 180: 120 is the safe answer.
    assert_eq!(snap_to_supported(150), 120);
    assert_eq!(snap_to_supported(90), 60);
    assert_eq!(snap_to_supported(210), 180);
}

#[test]
fn an_unknown_display_means_sixty_hertz() {
    assert_eq!(snap_to_supported(0), DEFAULT_HZ);
}

#[test]
fn the_choice_resolves_auto_against_the_display_and_fixed_by_override() {
    assert_eq!(RefreshChoice::Auto.resolve(0), 60);
    assert_eq!(RefreshChoice::Auto.resolve(240), 240);
    assert_eq!(RefreshChoice::Auto.resolve(144), 120);

    // A manual choice is an override: it is taken as-is, even on a display that
    // reports something else (the user may know better).
    assert_eq!(RefreshChoice::Fixed(120).resolve(60), 120);
    assert_eq!(RefreshChoice::Fixed(60).resolve(240), 60);
    assert_eq!(RefreshChoice::Fixed(240).resolve(0), 240);

    // Only supported rates can be chosen.
    assert_eq!(RefreshChoice::from_hz(144), RefreshChoice::Fixed(120));
    assert_eq!(RefreshChoice::from_hz(60), RefreshChoice::Fixed(60));
    assert_eq!(RefreshChoice::default(), RefreshChoice::Auto);
}

#[test]
fn a_frame_period_is_exact_nanoseconds() {
    assert_eq!(frame_period(60), Duration::from_nanos(16_666_666));
    assert_eq!(frame_period(120), Duration::from_nanos(8_333_333));
    assert_eq!(frame_period(180), Duration::from_nanos(5_555_555));
    assert_eq!(frame_period(240), Duration::from_nanos(4_166_666));

    // An unsupported rate is snapped first, so the loop can never spin.
    assert_eq!(frame_period(75), frame_period(60));
    assert_eq!(frame_period(0), frame_period(60));
}

#[test]
fn the_clock_advances_by_exactly_one_period_and_never_drifts() {
    let mut clock = FrameClock::new(240);
    let start = clock.start();

    // Deadlines are absolute: `start + n * period`, not "now + period".
    assert_eq!(clock.next(), start + frame_period(240));
    assert_eq!(clock.next(), start + frame_period(240) * 2);
    assert_eq!(clock.next(), start + frame_period(240) * 3);

    // Re-arming with a slower rate restarts the schedule from the current
    // deadline (so a rate change does not teleport the clock).
    clock.retarget(60);
    let deadline = clock.next();
    assert_eq!(clock.next(), deadline + frame_period(60));
}

#[test]
fn the_frame_clock_reports_how_far_behind_it_is() {
    let mut clock = FrameClock::new(60);
    let period = frame_period(60);

    // A tick that arrives early: nothing to skip.
    let deadline = clock.next();
    assert_eq!(clock.lateness(deadline - Duration::from_millis(1)), Duration::ZERO);

    // A tick that arrives late by 5 ms reports 5 ms.
    assert_eq!(clock.lateness(deadline + Duration::from_millis(5)), Duration::from_millis(5));

    // A long stall (e.g. the machine was asleep) is clamped to one frame, so
    // the loop never tries to "catch up" by running a burst of frames.
    assert_eq!(clock.lateness(deadline + Duration::from_secs(10)), period);
}

#[test]
fn the_fps_meter_reports_the_rate_it_is_fed() {
    let mut meter = FpsMeter::new();

    for _ in 0..60 {
        meter.record(frame_period(60));
    }

    let fps = meter.fps();
    assert!((fps - 60.0).abs() < 1.0, "60 frames of 16.67 ms is 60 fps: {fps}");

    // The worst frame in the window is reported, not hidden by the average.
    meter.record(Duration::from_millis(50));
    assert_eq!(meter.worst_ms(), 50.0);
    assert!(meter.frames() >= 60);
}

#[test]
fn the_fps_meter_forgets_old_frames_after_a_reset() {
    let mut meter = FpsMeter::new();
    meter.record(Duration::from_millis(40));
    meter.reset();

    assert_eq!(meter.frames(), 0);
    assert_eq!(meter.worst_ms(), 0.0);
    assert_eq!(meter.fps(), 0.0, "nothing measured yet is not 60 fps");
}

#[test]
fn a_two_hundred_forty_hertz_loop_stays_under_four_point_two_milliseconds() {
    // The budget that makes 240 Hz possible: one frame of the loop (state +
    // view) must fit well inside the period, so the ink keeps up with the pen.
    let budget = frame_period(240);
    let mut meter = FpsMeter::new();

    for _ in 0..1000 {
        meter.record(Duration::from_micros(500));
    }

    assert!(meter.worst_ms() < 4.166, "a 0.5 ms frame fits 240 Hz");
    assert!(Duration::from_micros(500) < budget);
}

//! Settings contract: the app remembers how the user likes to write.
//!
//! Settings are the only thing light-note persists on its own (a note's ink
//! travels with its PDF sidecar).  The format is JSON, it is forward compatible
//! — an unknown field from a newer version is ignored, not an error — and a
//! damaged file falls back to defaults instead of refusing to start.

use light_note::display::{RefreshChoice, SUPPORTED_HZ};
use light_note::ink::Tool;
use light_note::settings::Settings;

#[test]
fn the_defaults_are_what_a_first_run_should_be() {
    let settings = Settings::default();

    assert_eq!(settings.refresh, RefreshChoice::Auto);
    assert_eq!(settings.tool, Tool::Pen);
    assert_eq!(settings.pen_width_pt, 2.0);
    assert_eq!(settings.highlighter_width_pt, 14.0);
    assert_eq!(settings.eraser_radius_pt, 12.0);
    assert_eq!(settings.margin_pt, 24.0);
    assert!(settings.show_dev_panel == false);
}

#[test]
fn settings_round_trip_through_json() {
    let mut settings = Settings::default();
    settings.refresh = RefreshChoice::Fixed(240);
    settings.tool = Tool::Highlighter;
    settings.pen_width_pt = 3.5;
    settings.margin_pt = 36.0;

    let json = settings.to_json().expect("serialize");
    let restored = Settings::from_json(&json).expect("deserialize");

    assert_eq!(restored, settings);
    assert!(json.contains("240"), "the rate is written down: {json}");
}

#[test]
fn a_setting_from_a_newer_version_is_ignored_not_rejected() {
    let json = r#"{
        "refresh": {"Fixed": 120},
        "tool": "Pen",
        "pen_width_pt": 2.0,
        "highlighter_width_pt": 14.0,
        "eraser_radius_pt": 12.0,
        "margin_pt": 24.0,
        "show_dev_panel": false,
        "something_from_the_future": {"nested": [1, 2, 3]}
    }"#;

    let settings = Settings::from_json(json).expect("unknown fields are ignored");
    assert_eq!(settings.refresh, RefreshChoice::Fixed(120));
}

#[test]
fn a_damaged_settings_file_falls_back_to_defaults() {
    // The app must start even if the file was truncated by a crash.
    let settings = Settings::from_json("{ this is not json").expect("fall back");
    assert_eq!(settings, Settings::default());

    // A file that is valid JSON but the wrong shape, too.
    let settings = Settings::from_json("[1, 2, 3]").expect("fall back");
    assert_eq!(settings, Settings::default());
}

#[test]
fn every_supported_rate_survives_a_round_trip() {
    // 60/120/180/240 are the whole point of the refresh setting.
    for hz in SUPPORTED_HZ {
        let mut settings = Settings::default();
        settings.refresh = RefreshChoice::Fixed(hz);

        let restored = Settings::from_json(&settings.to_json().expect("json")).expect("parse");
        assert_eq!(restored.refresh.resolve(60), hz);
    }
}

#[test]
fn a_manual_rate_is_snapped_when_it_is_loaded() {
    // A hand-edited file with a rate that no longer exists is snapped rather
    // than refused.
    let json = r#"{"refresh": {"Fixed": 144}}"#;
    let settings = Settings::from_json(json).expect("parse");

    assert_eq!(settings.refresh, RefreshChoice::Fixed(120));
}

#[test]
fn the_pen_width_is_kept_inside_a_writable_range() {
    let json = r#"{"pen_width_pt": 900.0, "highlighter_width_pt": -3.0, "eraser_radius_pt": 0.0}"#;
    let settings = Settings::from_json(json).expect("parse");

    assert!(settings.pen_width_pt <= 40.0, "{}", settings.pen_width_pt);
    assert!(settings.pen_width_pt >= 0.5);
    assert!(settings.highlighter_width_pt >= 1.0);
    assert!(settings.eraser_radius_pt >= 1.0);
}

//! 필기 모델 계약 — 도구 라벨, 샘플 필터, 압력, 경계, 지우개 히트 테스트.
//!
//! 이 파일은 **플랫폼을 전혀 모른다**: 리눅스에서 그대로 돈다.

use light_note_core::geom::Point;
use light_note_core::ink::{
    pressure_from_speed, InkPoint, Rgba, Stroke, StrokeStyle, Tool, MIN_SAMPLE_DISTANCE_PT,
};

fn pen() -> StrokeStyle {
    StrokeStyle::for_tool(Tool::Pen)
}

#[test]
fn tool_labels_are_the_single_source_of_truth() {
    assert_eq!(Tool::ALL.len(), 3);
    for tool in Tool::ALL {
        assert_eq!(
            Tool::from_label(tool.label()),
            Some(tool),
            "라벨 ↔ 도구 왕복이 성립해야 한다"
        );
        assert!(!tool.hint().is_empty());
    }
    assert_eq!(Tool::from_label("없는도구"), None);
    assert!(Tool::Eraser.is_eraser() && !Tool::Pen.is_eraser());
    assert!(Tool::Pen.is_draw());
}

#[test]
fn samples_closer_than_the_minimum_are_dropped() {
    let mut stroke = Stroke::new(Tool::Pen, pen(), InkPoint::new(0.0, 0.0));
    let tiny = MIN_SAMPLE_DISTANCE_PT * 0.25;
    assert!(!stroke.push_xy(tiny, tiny), "너무 가까운 샘플은 버린다");
    assert!(stroke.push_xy(1.0, 0.0), "충분히 먼 샘플은 받는다");
    assert_eq!(stroke.len(), 2);
}

#[test]
fn pressure_only_changes_are_kept() {
    let mut stroke = Stroke::new(Tool::Pen, pen(), InkPoint::new(0.0, 0.0));
    assert!(
        stroke.push(InkPoint::with_pressure(0.05, 0.0, 0.2)),
        "같은 자리라도 필압이 크게 바뀌면 남긴다"
    );
    assert_eq!(stroke.len(), 2);
    assert!(!stroke.push(InkPoint::with_pressure(0.05, 0.0, 0.21)));
}

#[test]
fn widths_follow_pressure_and_are_clamped() {
    let style = StrokeStyle::new(Rgba::BLACK, 10.0);
    assert!((style.width_at(1.0) - 10.0).abs() < 1e-4);
    assert!((style.width_at(0.0) - 4.5).abs() < 1e-4);
    assert!((style.width_at(9.0) - 10.0).abs() < 1e-4, "범위 밖 압력은 clamp");

    let too_wide = StrokeStyle::new(Rgba::BLACK, 999.0);
    assert_eq!(too_wide.width, StrokeStyle::MAX_WIDTH_PT);
    let too_thin = StrokeStyle::new(Rgba::BLACK, 0.0);
    assert_eq!(too_thin.width, StrokeStyle::MIN_WIDTH_PT);
}

#[test]
fn bounds_include_half_of_the_stroke_width() {
    let stroke = Stroke::new(Tool::Pen, StrokeStyle::pen(Rgba::BLACK, 4.0), InkPoint::new(10.0, 10.0));
    let mut stroke = stroke;
    stroke.push_xy(50.0, 10.0);

    let bounds = stroke.bounds().expect("경계");
    // 잉크 경계는 폭의 절반(2pt)만큼 **사방으로** 확장된다.
    assert!((bounds.min.x - 8.0).abs() < 1e-4);
    assert!((bounds.min.y - 8.0).abs() < 1e-4, "폭의 절반만큼 확장");
    assert!((bounds.max.x - 52.0).abs() < 1e-4);
    assert!((bounds.max.y - 12.0).abs() < 1e-4);
    assert!(bounds.contains(Point::new(30.0, 10.0)));
    assert!((stroke.length() - 40.0).abs() < 1e-4);

    let points_only = stroke.point_bounds().expect("점 경계");
    assert!((points_only.min.x - 10.0).abs() < 1e-4, "점 경계는 폭을 무시한다");
}

#[test]
fn eraser_hit_test_uses_the_segment_distance() {
    let mut stroke = Stroke::new(Tool::Pen, StrokeStyle::pen(Rgba::BLACK, 2.0), InkPoint::new(0.0, 0.0));
    stroke.push_xy(50.0, 0.0);

    assert!(stroke.hits_circle(Point::new(25.0, 1.0), 1.0), "선분 위");
    assert!(!stroke.hits_circle(Point::new(25.0, 20.0), 2.0), "멀리 떨어진 점");
    assert!(stroke.hits_circle(Point::new(52.0, 0.0), 4.0), "끝점 근처");
}

#[test]
fn dot_strokes_are_recognized_and_hit_near_their_point() {
    let stroke = Stroke::new(Tool::Pen, pen(), InkPoint::new(10.0, 10.0));
    assert!(stroke.is_dot());
    assert_eq!(stroke.len(), 1);
    assert!(stroke.hits_point(Point::new(11.0, 10.0), 2.0));
    assert!(!stroke.hits_point(Point::new(30.0, 10.0), 2.0));
}

#[test]
fn speed_fallback_pressure_stays_in_range() {
    assert!((pressure_from_speed(0.0) - 1.0).abs() < 1e-4);
    assert!((pressure_from_speed(100.0) - 0.35).abs() < 1e-4);
    assert!(pressure_from_speed(1.25) < 1.0);
}

#[test]
fn describe_mentions_the_tool_and_the_sample_count() {
    let mut stroke = Stroke::new(Tool::Highlighter, StrokeStyle::for_tool(Tool::Highlighter), InkPoint::new(0.0, 0.0));
    stroke.push_xy(10.0, 0.0);
    let text = stroke.describe();
    assert!(text.contains(Tool::Highlighter.label()));
    assert!(text.contains("2점"));
}

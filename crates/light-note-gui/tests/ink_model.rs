//! 필기 모델 계약 — 도구 라벨, 샘플 필터, 압력, 경계, 지우개 히트 테스트.
//!
//! 이 파일은 **플랫폼을 전혀 모른다**: WinUI 없이 그대로 돈다(②와 그 재료만 검사한다).

use light_note_gui::geom::{distance_to_segment, Pt};
use light_note_gui::ink::{
    pressure_from_speed, InkPoint, Rgba, Stroke, Style, Tool, MIN_SAMPLE_DISTANCE_PT,
};

/// 펜 스타일.
fn pen() -> Style {
    Style::for_tool(Tool::Pen)
}

/// `(x, y)`에 압력 `pressure`인 표본.
fn at(x: f32, y: f32, pressure: f32) -> InkPoint {
    InkPoint::new(Pt::new(x, y), pressure)
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
        assert!(!Style::hint(tool).is_empty(), "도구마다 안내 문구가 있다");
    }
    assert_eq!(Tool::from_label("없는도구"), None);
    assert!(Tool::Eraser.is_eraser() && !Tool::Pen.is_eraser());
    assert!(!Tool::Highlighter.is_eraser());
}

#[test]
fn samples_closer_than_the_minimum_are_dropped() {
    let mut stroke = Stroke::new(Tool::Pen, pen(), at(0.0, 0.0, 1.0));
    let tiny = MIN_SAMPLE_DISTANCE_PT * 0.25;
    assert!(
        !stroke.push(at(tiny, tiny, 1.0)),
        "너무 가까운 표본은 버린다(모델을 가볍게)"
    );
    assert!(stroke.push(at(1.0, 0.0, 1.0)), "충분히 먼 표본은 받는다");
    assert_eq!(stroke.len(), 2);
    assert_eq!(stroke.points()[1].pos, Pt::new(1.0, 0.0));
}

#[test]
fn pressure_only_changes_are_kept() {
    let mut stroke = Stroke::new(Tool::Pen, pen(), at(0.0, 0.0, 1.0));
    assert!(
        stroke.push(at(0.05, 0.0, 0.2)),
        "같은 자리라도 필압이 크게 바뀌면 남긴다(폭이 변하는 획)"
    );
    assert_eq!(stroke.len(), 2);
    assert!(
        !stroke.push(at(0.05, 0.0, 0.21)),
        "표본도 가깝고 필압도 비슷하면 버린다"
    );
}

#[test]
fn widths_follow_pressure_and_are_clamped() {
    let style = Style::new(Rgba::INK_BLUE, 10.0);
    assert!(
        (style.width_at(1.0) - 10.0).abs() < 1e-4,
        "압력 1.0 = 온전한 폭"
    );
    assert!((style.width_at(0.0) - 4.5).abs() < 1e-4, "압력 0.0 = 45%");
    assert!(
        (style.width_at(9.0) - 10.0).abs() < 1e-4,
        "범위 밖 압력은 clamp"
    );

    assert_eq!(
        Style::new(Rgba::INK_BLUE, 999.0).width_pt,
        Style::MAX_WIDTH_PT
    );
    assert_eq!(
        Style::new(Rgba::INK_BLUE, 0.0).width_pt,
        Style::MIN_WIDTH_PT
    );
}

#[test]
fn eraser_hit_test_uses_the_segment_distance() {
    let mut stroke = Stroke::new(
        Tool::Pen,
        Style::new(Rgba::INK_BLUE, 2.0),
        at(0.0, 0.0, 1.0),
    );
    stroke.push(at(50.0, 0.0, 1.0));

    // 판정은 "선분까지의 거리 ≤ 반지름 + 획 반폭(1.0)"이다.
    assert!(stroke.hits_circle(Pt::new(25.0, 1.0), 1.0), "선분 위");
    assert!(
        !stroke.hits_circle(Pt::new(25.0, 20.0), 2.0),
        "멀리 떨어진 점"
    );
    assert!(stroke.hits_circle(Pt::new(52.0, 0.0), 4.0), "끝점 근처");
    assert_eq!(
        distance_to_segment(Pt::new(25.0, 20.0), Pt::new(0.0, 0.0), Pt::new(50.0, 0.0)),
        20.0
    );
}

#[test]
fn a_dot_is_recognized_and_hit_near_its_point() {
    let stroke = Stroke::new(Tool::Pen, pen(), at(10.0, 10.0, 1.0));
    assert!(stroke.is_dot(), "점 하나짜리 획은 탭이다");
    assert_eq!(stroke.len(), 1);
    assert!(stroke.hits_circle(Pt::new(11.0, 10.0), 2.0));
    assert!(!stroke.hits_circle(Pt::new(30.0, 10.0), 2.0));
    assert_eq!(stroke.first().map(|p| p.pos), Some(Pt::new(10.0, 10.0)));
    assert_eq!(stroke.last().map(|p| p.pos), Some(Pt::new(10.0, 10.0)));
}

#[test]
fn an_empty_page_has_no_ink_to_bake() {
    // 빈 페이지에 1.13Mpx PNG를 만들 이유가 없다 — 워커를 부르기 전에 싸게 답한다.
    assert!(!light_note_gui::shape::has_ink(&[]));
    assert!(
        !light_note_gui::shape::has_ink(&[Stroke::new(
            Tool::Eraser,
            Style::for_tool(Tool::Eraser),
            at(0.0, 0.0, 1.0)
        )]),
        "지우개는 모델에 획을 남기지 않는다"
    );

    let pen = Stroke::new(Tool::Pen, pen(), at(0.0, 0.0, 1.0));
    assert!(!pen.is_empty());
    assert!(light_note_gui::shape::has_ink(&[pen]));
}

#[test]
fn speed_fallback_pressure_stays_in_range() {
    assert!(
        (pressure_from_speed(0.0) - 1.0).abs() < 1e-4,
        "멈춰 있으면 굵다"
    );
    assert!(
        (pressure_from_speed(100.0) - 0.35).abs() < 1e-4,
        "아주 빠르면 가늘다"
    );
    assert!(pressure_from_speed(1.25) < 1.0);
    assert!(pressure_from_speed(1.25) > 0.35);
}

#[test]
fn rgba_helpers_keep_the_channels() {
    assert!(Rgba::INK_BLUE.is_opaque());
    assert!(!Rgba::HIGHLIGHT_YELLOW.is_opaque(), "형광펜은 반투명이다");
    assert_eq!(Rgba::INK_BLUE.to_rgb(), (23, 78, 166));

    let opaque = Rgba::HIGHLIGHT_YELLOW.with_alpha(255);
    assert_eq!(opaque.a, 255);
    assert_eq!(
        opaque.to_rgb(),
        Rgba::HIGHLIGHT_YELLOW.to_rgb(),
        "색은 그대로, 알파만"
    );
    assert!(Rgba::new(0, 0, 0, 0) != opaque);
}

#[test]
fn ink_points_default_to_full_pressure() {
    let point = InkPoint::at(Pt::new(3.0, 4.0));
    assert_eq!(point.pressure, InkPoint::DEFAULT_PRESSURE);
    assert!((point.pos.distance(Pt::new(0.0, 0.0)) - 5.0).abs() < 1e-4);
}

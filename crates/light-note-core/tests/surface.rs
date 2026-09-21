//! 표면 계약 — 정적 PNG와 라이브 선분이 **언제** 바뀌는가.
//!
//! 정적 레이어는 백그라운드에서 만들어진다(필기감: UI 스레드는 페이지 래스터를
//! 기다리지 않는다 — `surface_cost` 예제가 그 비용을 잰다). 그래서 **PNG에 아직 없는
//! 확정 획**은 라이브 선분으로 계속 그려야 한다. 이 파일이 그 규칙을 고정한다.

mod common;

use light_note_core::geom::{Point, Size};
use light_note_core::ink::{InkPoint, Stroke, StrokeStyle, Tool};
use light_note_core::surface::{has_ink, live_lines, pending_lines, static_layer};

/// 직선 스트로크 하나.
fn pen(x: f32) -> Stroke {
    common::line_stroke(
        Tool::Pen,
        Point::new(x, 50.0),
        Point::new(x + 40.0, 50.0),
        8,
        4.0,
    )
}

#[test]
fn a_page_without_ink_makes_no_layer() {
    assert!(!has_ink(&[]), "빈 페이지에 잉크는 없다");
    assert!(static_layer(&[], Size::A4, 1.5).is_none());
}

#[test]
fn eraser_strokes_alone_are_not_ink() {
    // 지우개 드래그는 모델에 스트로크를 남기지 않지만, 남더라도 그릴 것이 아니다.
    let eraser = common::line_stroke(
        Tool::Eraser,
        Point::new(10.0, 10.0),
        Point::new(60.0, 10.0),
        4,
        2.0,
    );
    assert!(!has_ink(std::slice::from_ref(&eraser)));
    assert!(static_layer(&[eraser], Size::A4, 1.5).is_none());
}

#[test]
fn pen_strokes_become_a_static_png() {
    let stroke = pen(20.0);
    assert!(has_ink(std::slice::from_ref(&stroke)));
    let png = static_layer(&[stroke], Size::new(200.0, 100.0), 1.0).expect("정적 PNG");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "PNG 시그니처");
}

#[test]
fn strokes_committed_after_the_snapshot_stay_live() {
    let first = pen(20.0);
    let second = pen(120.0);

    // 첫 획은 PNG에 들어갔다 — 라이브는 아직 안 들어간 두 번째 획만 그린다.
    let lines = pending_lines(&[first.clone(), second.clone()], 1, None, 1.0);
    assert_eq!(lines, live_lines(&second, 1.0));
}

#[test]
fn a_snapshot_of_everything_leaves_no_tail() {
    let first = pen(20.0);
    assert!(
        pending_lines(std::slice::from_ref(&first), 1, None, 1.0).is_empty(),
        "전부 PNG에 있으면 라이브 선분은 없다"
    );
    // 화면의 PNG를 믿지 않는 상태(되돌리기/페이지 이동/줌 직후)도 꼬리는 없다.
    assert!(pending_lines(&[first], 5, None, 1.0).is_empty());
}

#[test]
fn the_in_progress_stroke_is_drawn_on_top_of_the_tail() {
    let committed = pen(20.0);
    let mut drawing = Stroke::new(
        Tool::Pen,
        StrokeStyle::for_tool(Tool::Pen),
        InkPoint::new(60.0, 200.0),
    );
    for step in 1..12 {
        drawing.push(InkPoint::new(60.0 + step as f32 * 3.0, 200.0));
    }

    let lines = pending_lines(std::slice::from_ref(&committed), 0, Some(&drawing), 1.0);
    let mut expected = live_lines(&committed, 1.0);
    expected.extend(live_lines(&drawing, 1.0));
    assert_eq!(lines, expected, "꼬리 뒤에 진행 중인 획이 온다");
}

//! 표면 계약 — 정적 PNG와 라이브 도형이 **언제**, 그리고 **무슨 모양으로** 바뀌는가.
//!
//! 정적 레이어는 백그라운드에서 만들어진다(필기감: UI 스레드는 페이지 래스터를
//! 기다리지 않는다 — `surface_cost` 예제가 그 비용을 잰다). 그래서 **PNG에 아직 없는
//! 확정 획**은 라이브 도형으로 계속 그려야 한다. 이 파일이 그 규칙을 고정한다.
//!
//! 가장 중요한 계약은 [`the_live_geometry_covers_the_raster_ink`]다 — 라이브 도형이
//! 래스터 잉크와 **같은 영역**을 덮지 않으면, 획을 확정하는 순간 잉크가 "딱" 바뀐다.

mod common;

use std::sync::Arc;

use hayro::vello_cpu::kurbo::BezPath;
use hayro::vello_cpu::peniko::color::{AlphaColor, Srgb};
use hayro::vello_cpu::{Pixmap, RenderContext, Resources};
use light_note_core::geom::{Point, Size};
use light_note_core::ink::{InkPoint, Rgba, Stroke, StrokeStyle, Tool};
use light_note_core::raster::{self, InkSpan};
use light_note_core::surface::{
    blank_png, has_ink, live_ink, pending_ink, static_layer, LiveStroke, StaticLayers,
};

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
    let ink = pending_ink(&[first.clone(), second.clone()], 1, None, 1.0);
    assert_eq!(ink, vec![live_ink(&second, 1.0)]);
}

#[test]
fn a_snapshot_of_everything_leaves_no_tail() {
    let first = pen(20.0);
    assert!(
        pending_ink(std::slice::from_ref(&first), 1, None, 1.0).is_empty(),
        "전부 PNG에 있으면 라이브 도형은 없다"
    );
    // 화면의 PNG를 믿지 않는 상태(되돌리기/페이지 이동/줌 직후)도 꼬리는 없다.
    assert!(pending_ink(&[first], 5, None, 1.0).is_empty());
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

    let ink = pending_ink(std::slice::from_ref(&committed), 0, Some(&drawing), 1.0);
    assert_eq!(
        ink,
        vec![live_ink(&committed, 1.0), live_ink(&drawing, 1.0)],
        "꼬리 뒤에 진행 중인 획이 온다"
    );
}

/// 200×100pt 페이지에 직선 `count`개를 그린 PNG.
fn layer(count: usize) -> Arc<[u8]> {
    let strokes: Vec<Stroke> = (0..count)
        .map(|index| pen(20.0 + index as f32 * 60.0))
        .collect();
    static_layer(&strokes, Size::new(200.0, 100.0), 1.0).expect("정적 PNG")
}

#[test]
fn a_new_picture_never_touches_the_visible_layer() {
    let mut layers = StaticLayers::default();
    let first = layer(1);
    layers.show_now(Some(Arc::clone(&first)), 1);

    let second = layer(2);
    let same = layers.stage(Some(Arc::clone(&second)), 2);

    // 스테이징은 **보이지 않는 뒤 레이어**에만 한다 — 화면은 그대로다(깜빡이지 않는다).
    assert!(!same, "다른 그림이면 디코드를 기다려야 한다");
    assert_eq!(layers.front(), 0);
    assert_eq!(layers.visible_count(), 1, "보이는 레이어의 획 수는 그대로");
    assert_eq!(layers.png(1), Some(&second), "새 그림은 뒤 레이어에 있다");
    assert_eq!(layers.png(0), Some(&first), "보이는 레이어는 손대지 않는다");
}

#[test]
fn promotion_happens_only_after_the_decode_signal() {
    let mut layers = StaticLayers::default();
    layers.show_now(None, 0);
    layers.stage(Some(layer(3)), 3);

    // 스테이징되지 않은 레이어의 신호는 무시한다(낡은 디코드).
    assert!(!layers.promote(0), "스테이징된 레이어가 아니다");
    assert_eq!(layers.front(), 0, "신호 없이는 화면이 바뀌지 않는다");

    // 뒤 레이어(1)의 디코드 완료 → 이제 바꾼다.
    assert!(layers.promote(1));
    assert_eq!(layers.front(), 1);
    assert_eq!(layers.visible_count(), 3);
    assert!(!layers.promote(1), "이미 승격했다 — 두 번 바뀌지 않는다");
}

#[test]
fn staging_the_same_picture_needs_no_wait() {
    let mut layers = StaticLayers::default();
    let one = layer(1);
    layers.show_now(Some(Arc::clone(&one)), 1);

    // 새 그림은 뒤 레이어로 → 디코드 필요.
    assert!(!layers.stage(Some(layer(2)), 2));
    assert!(layers.promote(1));
    assert_eq!(layers.front(), 1);

    // 되돌리기 → 뒤 레이어(0)가 **이미 그 그림**을 디코드해 두었다 → 기다릴 필요가 없다.
    assert!(
        layers.stage(Some(Arc::clone(&one)), 1),
        "같은 그림이면 즉시"
    );
    assert!(layers.promote_staged());
    assert_eq!(layers.front(), 0);
    assert_eq!(layers.visible_count(), 1);
}

#[test]
fn a_blank_layer_is_a_transparent_picture() {
    let png = blank_png();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "PNG 시그니처");

    let pixmap = light_note_core::raster::from_png(&png).expect("빈 레이어 PNG 디코드");
    assert_eq!((pixmap.width(), pixmap.height()), (1, 1));
    assert_eq!(
        light_note_core::raster::ink_coverage(&pixmap),
        0,
        "투명하다 — 화면에 아무것도 보이지 않는다"
    );
    assert!(
        Arc::ptr_eq(&png, &blank_png()),
        "한 번만 만들어 재사용한다(복사/재인코딩 없음)"
    );
}

#[test]
fn the_safety_net_promotes_only_after_enough_time() {
    use std::time::Duration;

    let mut layers = StaticLayers::default();
    layers.show_now(None, 0);
    layers.stage(Some(layer(2)), 2);

    // 방금 스테이징했다 — 아직 디코드 중일 수 있으므로 성급히 바꾸지 않는다.
    assert!(
        !layers.promote_if_settled(Duration::from_millis(120)),
        "신호도 없고 시간도 안 지났다"
    );
    assert_eq!(layers.front(), 0);

    // 충분히 기다렸다면(디코드가 끝났을 시점) 바꾼다 — 화면이 영원히 멈추지 않는다.
    std::thread::sleep(Duration::from_millis(10));
    assert!(layers.promote_if_settled(Duration::from_millis(5)));
    assert_eq!(layers.front(), 1);
    assert_eq!(layers.visible_count(), 2);
}

#[test]
fn the_tail_follows_the_visible_layer() {
    let first = pen(20.0);
    let second = pen(120.0);
    let committed = [first.clone(), second.clone()];

    // 화면에 아무것도 없으면 두 획 다 라이브로 그린다(빈 틈이 없다).
    let mut layers = StaticLayers::default();
    let ink = pending_ink(&committed, layers.visible_count(), None, 1.0);
    assert_eq!(
        ink.iter().map(LiveStroke::element_count).sum::<usize>(),
        live_ink(&first, 1.0).element_count() + live_ink(&second, 1.0).element_count()
    );

    // 첫 획이 화면에 올라오면(승격) 꼬리에는 두 번째만 남는다.
    layers.stage(None, 1);
    assert!(layers.promote_staged());
    let ink = pending_ink(&committed, layers.visible_count(), None, 1.0);
    assert_eq!(ink, vec![live_ink(&second, 1.0)]);
}

// ─────────────────────────────────────────────────────────────────────────────
// 라이브 도형 = 래스터 잉크 (승격 순간에 잉크가 바뀌지 않는 근거)
// ─────────────────────────────────────────────────────────────────────────────

/// 곡선 스트로크 — `varying`이면 폭이 변한다(래스터의 구간 경로).
fn curved(tool: Tool, width: f32, varying: bool) -> Stroke {
    let style = match tool {
        Tool::Highlighter => StrokeStyle::highlighter(Rgba::HIGHLIGHT_YELLOW, width),
        _ => StrokeStyle::pen(Rgba::INK_BLUE, width),
    };
    let at = |step: f32| {
        let t = step / 48.0;
        let pressure = if varying {
            0.65 + 0.35 * (step * 0.4).sin()
        } else {
            InkPoint::DEFAULT_PRESSURE
        };
        InkPoint::with_pressure(20.0 + t * 150.0, 60.0 + (t * 6.0).sin() * 24.0, pressure)
    };
    let mut stroke = Stroke::new(tool, style, at(0.0));
    for step in 1..=48 {
        stroke.push(at(step as f32));
    }
    stroke
}

/// 라이브 도형을 **래스터와 같은 방식**(같은 색, 한 번의 채움)으로 그린다 — 커버리지 비교용.
fn render_live(stroke: &LiveStroke, size: Size, scale: f32) -> Pixmap {
    let (width, height) = raster::page_pixel_size(size, scale);
    let mut path = BezPath::new();
    for line in &stroke.lines {
        raster::push_span_rect(
            &mut path,
            &InkSpan {
                width: line.width as f32,
                from: (line.x1, line.y1),
                to: (line.x2, line.y2),
            },
        );
    }
    for cap in &stroke.caps {
        raster::push_disc(&mut path, (cap.x, cap.y), cap.radius);
    }
    let color = stroke.color;
    let mut context = RenderContext::new(width, height);
    context.set_paint(AlphaColor::<Srgb>::from_rgba8(
        color.r, color.g, color.b, color.a,
    ));
    context.fill_path(&path);
    let mut pixmap = Pixmap::new(width, height);
    context.render_to_pixmap(&mut Resources::new(), &mut pixmap);
    pixmap
}

/// 덮인 픽셀(알파 > 40) — 형광펜(알파 90)도 잉크로 센다.
fn coverage(pixmap: &Pixmap) -> Vec<bool> {
    pixmap
        .data_as_u8_slice()
        .chunks_exact(4)
        .map(|pixel| pixel[3] > 40)
        .collect()
}

/// 두 마스크가 다른 픽셀 비율(합집합 기준).
fn difference(left: &[bool], right: &[bool]) -> f64 {
    let mut union = 0usize;
    let mut different = 0usize;
    for (left, right) in left.iter().zip(right) {
        if *left || *right {
            union += 1;
        }
        if left != right {
            different += 1;
        }
    }
    if union == 0 {
        0.0
    } else {
        different as f64 / union as f64
    }
}

#[test]
fn the_live_geometry_covers_the_raster_ink() {
    let size = Size::new(200.0, 120.0);
    let cases: Vec<(&str, Stroke)> = vec![
        ("펜 직선", pen(20.0)),
        ("펜 곡선(폭 일정)", curved(Tool::Pen, 4.0, false)),
        ("펜 곡선(폭 변화)", curved(Tool::Pen, 4.0, true)),
        (
            "형광펜 곡선(폭 일정)",
            curved(Tool::Highlighter, 14.0, false),
        ),
        (
            "형광펜 곡선(폭 변화)",
            curved(Tool::Highlighter, 14.0, true),
        ),
        (
            "점 하나",
            Stroke::new(
                Tool::Pen,
                StrokeStyle::for_tool(Tool::Pen),
                InkPoint::new(60.0, 60.0),
            ),
        ),
    ];

    for (name, stroke) in cases {
        let raster_ink = raster::render_ink(std::slice::from_ref(&stroke), size, 1.0);
        let live = live_ink(&stroke, 1.0);
        let live_ink_map = render_live(&live, size, 1.0);
        let ratio = difference(&coverage(&raster_ink), &coverage(&live_ink_map));
        // 같은 도형을 같은 방식으로 그리므로 사실상 0%다 — 0.5%는 반올림/AA 여유일 뿐이다.
        assert!(
            ratio < 0.005,
            "{name}: 라이브 도형과 래스터 잉크가 {:.2}% 다르다(승격 순간에 잉크가 바뀐다)",
            ratio * 100.0
        );
    }
}

#[test]
fn a_translucent_joint_is_not_darker() {
    // 형광펜 + 폭 변화 = 구간 스트로크. 예전에는 구간마다 따로 스트로크해서 관절마다
    // 알파가 두 번 곱해졌다(실측 90 → 149) — 지금은 한 번의 채움이라 어디서나 같다.
    let stroke = curved(Tool::Highlighter, 14.0, true);
    let pixmap = raster::render_ink(std::slice::from_ref(&stroke), Size::new(200.0, 120.0), 1.0);
    let alpha = Rgba::HIGHLIGHT_YELLOW.a;
    let strongest = (0..120)
        .map(|y| raster::pixel_at(&pixmap, 95, y).a)
        .max()
        .unwrap_or(0);
    assert_eq!(strongest, alpha, "가장 진한 자리도 한 번만 곱해진 알파");

    for x in 20..170 {
        for y in 30..90 {
            let pixel = raster::pixel_at(&pixmap, x, y).a;
            assert!(
                pixel <= alpha,
                "({x},{y}) 알파 {pixel} — 겹쳐서 진해졌다(관절 얼룩)"
            );
        }
    }
}

#[test]
fn the_live_layer_ends_with_round_caps() {
    let stroke = pen(20.0);
    let live = live_ink(&stroke, 1.0);
    assert_eq!(live.caps.len(), 2, "양 끝의 둥근 캡");
    let width = stroke.style.width_at(InkPoint::DEFAULT_PRESSURE) as f64;
    assert!(
        (live.caps[0].radius - width * 0.5).abs() < 1e-9,
        "래스터의 둥근 캡과 같은 반지름 — 승격 순간에 잉크 끝이 자라지 않는다"
    );
    assert!((live.caps[0].x - 20.0).abs() < 1e-6, "첫 점 위");
    assert!((live.caps[1].x - 60.0).abs() < 1e-6, "마지막 점 위");
    assert!(!live.lines.is_empty(), "몸통은 선분이 그린다");
}

#[test]
fn a_tap_is_one_round_cap() {
    let dot = Stroke::new(
        Tool::Pen,
        StrokeStyle::for_tool(Tool::Pen),
        InkPoint::new(30.0, 40.0),
    );
    let live = live_ink(&dot, 1.0);
    assert!(live.lines.is_empty(), "점은 선분이 아니다");
    assert_eq!(live.caps.len(), 1);
    assert!((live.caps[0].x - 30.0).abs() < 1e-6);
    assert!((live.caps[0].y - 40.0).abs() < 1e-6);
}

#[test]
fn live_elements_stay_close_to_the_sample_count() {
    // 600점 스트로크 — WinUI 도형 수가 점 수 규모를 넘지 않는다(트리가 무거워지지 않는다).
    let mut stroke = Stroke::new(
        Tool::Pen,
        StrokeStyle::for_tool(Tool::Pen),
        InkPoint::new(10.0, 60.0),
    );
    for step in 1..600 {
        stroke.push(InkPoint::new(
            10.0 + step as f32 * 0.8,
            60.0 + (step as f32 * 0.05).sin() * 12.0,
        ));
    }
    let live = live_ink(&stroke, 1.0);
    let count = live.element_count();
    // 실측 601개(점 599개) — 곡선이 완만하면 점마다 선분 하나로 충분하다.
    assert!(
        count <= 700,
        "도형 {count}개 — 점 수(599) 규모를 넘지 않는다"
    );
    assert_eq!(live.caps.len(), 2);
}

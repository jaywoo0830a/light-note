//! 도형 계약 — **④-워커(래스터)와 ④-UI(라이브 도형)가 같은 기하를 쓴다.**
//!
//! 이 파일이 지키는 한 문장: *"획을 확정해도 잉크의 모양이 바뀌지 않는다."*
//! 화면의 라이브 도형은 WinUI `Line`/`Ellipse`로 그려지고 워커는 곡선+합집합을 래스터한다.
//! 둘이 다른 결정을 쓰면 확정하는 순간 선이 얇아지거나 굵어진다 — 그래서 여기서 **픽셀로**
//! 두 결정이 같은지 확인한다.
//!
//! 한계(정직하게): 여기서 검사하는 것은 **라이브 도형의 자료**(늘린 구간 + 반지름)가 래스터와
//! 같다는 것이지, WinUI가 그 선을 실제로 그리는 방식은 아니다(그건 창을 띄워야 안다).

mod common;

use hayro::vello_cpu::kurbo::{BezPath, Point, Shape};
use hayro::vello_cpu::peniko::color::{AlphaColor, Srgb};
use hayro::vello_cpu::{Pixmap, RenderContext, Resources};

use light_note_gui::geom::{Pt, Scale, Size};
use light_note_gui::ink::{Rgba, Stroke, Tool};
use light_note_gui::shape::{self, InkShape, InkSpan, LiveInk};

/// 라이브 도형을 래스터와 **같은 규칙**으로 채운다: 늘린 구간 사각형 + 양 끝 캡을 한 번에.
fn render_live(ink: &LiveInk, size: Size, scale: Scale, color: Rgba) -> Pixmap {
    let (width, height) = scale.pixels(size);
    let mut path = BezPath::new();
    for line in &ink.lines {
        shape::push_span_rect(
            &mut path,
            &InkSpan {
                width: line.width as f32,
                from: (line.x1, line.y1),
                to: (line.x2, line.y2),
            },
        );
    }
    for cap in &ink.caps {
        shape::push_disc(&mut path, (cap.x, cap.y), cap.radius);
    }

    let mut context = RenderContext::new(width, height);
    context.set_paint(AlphaColor::<Srgb>::from_rgba8(
        color.r, color.g, color.b, color.a,
    ));
    context.fill_path(&path);
    let mut pixmap = Pixmap::new(width, height);
    let mut resources = Resources::new();
    context.render_to_pixmap(&mut resources, &mut pixmap);
    pixmap
}

/// 도형 종류 이름 — 실패 메시지가 어느 결정에서 어긋났는지 알려준다.
fn shape_kind(stroke: &Stroke, scale: Scale) -> &'static str {
    match shape::ink_shape(stroke, scale) {
        Some(InkShape::Curve { .. }) => "Curve",
        Some(InkShape::Spans { .. }) => "Spans",
        Some(InkShape::Dot { .. }) => "Dot",
        None => "None",
    }
}

/// 획 하나를 래스터와 라이브로 각각 그려 **같은 픽셀**이 나오는지 본다.
fn assert_live_matches_raster(stroke: &Stroke, size: Size, scale: Scale) {
    let color = stroke.style.color;
    let raster = shape::render_ink(std::slice::from_ref(stroke), size, scale);
    let ink = shape::live_ink(stroke, scale);
    let live = render_live(&ink, size, scale, color);

    assert_eq!(
        (live.width(), live.height()),
        (raster.width(), raster.height()),
        "같은 크기여야 비교가 성립한다"
    );
    assert_eq!(
        live.data_as_u8_slice(),
        raster.data_as_u8_slice(),
        "라이브 도형과 래스터가 다르다(도형: {}) — 확정하는 순간 잉크가 바뀐다",
        shape_kind(stroke, scale)
    );
}

#[test]
fn a_dot_matches_between_live_and_raster() {
    let stroke = common::stroke_of(Tool::Pen, 4.0, &[(50.0, 50.0, 1.0)]);
    assert_eq!(shape_kind(&stroke, Scale::new(2.0)), "Dot");
    assert_live_matches_raster(&stroke, Size::new(100.0, 100.0), Scale::new(2.0));
}

#[test]
fn a_curve_matches_between_live_and_raster() {
    // 압력이 일정하다 → 곡선 하나(`Curve`): 직선 조각으로 펴는 **같은 오차**를 써야 한다.
    let stroke = common::line_stroke(
        Tool::Pen,
        Pt::new(10.0, 20.0),
        Pt::new(190.0, 60.0),
        24,
        3.0,
    );
    assert_eq!(shape_kind(&stroke, Scale::new(2.0)), "Curve");
    assert_live_matches_raster(&stroke, Size::new(200.0, 100.0), Scale::new(2.0));
}

#[test]
fn varying_pressure_matches_between_live_and_raster() {
    // 필압이 변한다 → 구간(`Spans`): 관절을 덮는 규칙까지 똑같아야 한다.
    let samples: Vec<(f32, f32, f32)> = (0..=20)
        .map(|step| {
            let t = step as f32 / 20.0;
            (
                10.0 + t * 180.0,
                50.0 + (t * 6.0).sin() * 20.0,
                0.2 + 0.8 * (1.0 - t),
            )
        })
        .collect();
    let stroke = common::stroke_of(Tool::Pen, 12.0, &samples);
    assert_eq!(shape_kind(&stroke, Scale::new(2.0)), "Spans");
    assert_live_matches_raster(&stroke, Size::new(200.0, 100.0), Scale::new(2.0));
}

#[test]
fn a_highlighter_matches_between_live_and_raster() {
    // 반투명은 "한 획 = 한 번 채우기" 규칙까지 같아야 한다(관절에서 알파가 겹치면 얼룩진다).
    let stroke = common::line_stroke(
        Tool::Highlighter,
        Pt::new(20.0, 40.0),
        Pt::new(180.0, 40.0),
        12,
        14.0,
    );
    assert_live_matches_raster(&stroke, Size::new(200.0, 100.0), Scale::new(2.0));
}

#[test]
fn the_joint_is_covered_by_the_extended_span() {
    // 관절 원은 "변의 절반이 r인 정사각형"에 내접한다 → 구간을 r만큼 늘리면 관절이 전부 덮인다.
    let span = InkSpan {
        width: 10.0,
        from: (0.0, 0.0),
        to: (20.0, 0.0),
    };
    let next = InkSpan {
        width: 10.0,
        from: (20.0, 0.0),
        to: (20.0, 20.0),
    };

    // 늘리지 않은 사각형은 관절 쪽 모서리를 **비워 둔다**(그래서 확장이 필요하다).
    let mut bare = BezPath::new();
    shape::push_span_rect(&mut bare, &span);
    assert!(
        !bare.contains(Point::new(23.0, 3.0)),
        "확장 없는 사각형이 관절을 덮었다 — 이 테스트의 전제가 틀렸다"
    );

    // 늘린 사각형은 덮는다.
    let mut extended = BezPath::new();
    shape::push_span_rect(&mut extended, &shape::extended_span(&span, false, true));
    assert!(extended.contains(Point::new(23.0, 3.0)));

    // 합집합은 관절 원의 **둘레 전체**를 덮는다 — 도형을 더 그릴 필요가 없다.
    let union = shape::spans_union(&[span, next]);
    for step in 0..32 {
        let angle = std::f32::consts::TAU * step as f32 / 32.0;
        let point = Point::new(20.0 + 4.9 * angle.cos() as f64, 4.9 * angle.sin() as f64);
        assert!(
            union.contains(point),
            "관절 둘레 {step}번 방향이 비어 있다: {point:?}"
        );
    }
}

#[test]
fn a_translucent_joint_does_not_double_blend() {
    // 형광펜을 꺾어 그으면 관절에서 잉크가 두 번 겹치기 쉽다 — 알파가 같아야 한다.
    let samples = [
        (20.0_f32, 50.0_f32, 1.0_f32),
        (60.0, 50.0, 1.0),
        (60.0, 90.0, 1.0),
        (100.0, 90.0, 1.0),
    ];
    let stroke = common::stroke_of(Tool::Highlighter, 20.0, &samples);
    let pixmap = shape::render_ink(&[stroke], Size::new(200.0, 200.0), Scale::new(2.0));

    let middle = shape::pixel_at(&pixmap, 80, 100); // 첫 구간 한가운데(px)
    let joint = shape::pixel_at(&pixmap, 120, 100); // 꺾이는 자리(px)
    assert!(middle.a > 0 && joint.a > 0, "둘 다 잉크여야 한다");
    assert_eq!(
        middle.a, joint.a,
        "관절에서 알파가 두 번 곱해졌다 — 한 획은 한 번에 채워야 한다"
    );
    assert!(middle.a < 255, "형광펜은 반투명이다: {}", middle.a);
}

#[test]
fn ink_lands_where_the_model_says() {
    // 좌표 규약 자체(pt → px, 좌상단 원점)를 픽셀로 못박는다.
    let size = Size::new(200.0, 100.0);
    let stroke = common::line_stroke(
        Tool::Pen,
        Pt::new(10.0, 50.0),
        Pt::new(190.0, 50.0),
        16,
        4.0,
    );
    let pixmap = shape::render_ink(&[stroke], size, Scale::new(1.0));

    assert_eq!((pixmap.width(), pixmap.height()), (200, 100));
    assert!(
        shape::pixel_at(&pixmap, 100, 50).a > 200,
        "선 위에는 잉크가 있다"
    );
    assert_eq!(shape::pixel_at(&pixmap, 100, 5).a, 0, "선에서 먼 곳은 투명");
    assert_eq!(shape::pixel_at(&pixmap, 5, 50).a, 0, "선분 밖은 투명");
    assert!(
        shape::ink_coverage(&pixmap) > 400,
        "선 길이만큼 픽셀이 칠해진다"
    );
}

#[test]
fn rendering_is_deterministic() {
    // 테스트가 픽셀을 비교하려면 같은 입력이 같은 출력이어야 한다(전제 검사).
    let stroke = common::line_stroke(
        Tool::Pen,
        Pt::new(10.0, 20.0),
        Pt::new(180.0, 80.0),
        40,
        6.0,
    );
    let size = Size::new(200.0, 100.0);
    let first = shape::render_ink(std::slice::from_ref(&stroke), size, Scale::new(1.5));
    let second = shape::render_ink(std::slice::from_ref(&stroke), size, Scale::new(1.5));
    assert_eq!(first.data_as_u8_slice(), second.data_as_u8_slice());
}

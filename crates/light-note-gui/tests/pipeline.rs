//! 4단계 파이프라인 계약 — **WinUI 없이** 돈다(WinUI는 창을 띄우는 일만 한다).
//!
//! 여기서 고정하는 것:
//! - ① 표본(태블릿 → 페이지 pt)과 위상 — 호버·접촉·이탈·스트림 끊김
//! - ② 드래그 하나 = 편집 하나, 취소는 아무것도 남기지 않는다, 지우개는 되돌릴 수 있다
//! - ③ 꼬리는 **구운 접두사 뒤에서 시작**하고, 베이크는 예산이 정하며, 낡은 응답은 버려진다 (R1·R2·R3)

use std::time::{Duration, Instant};

use light_note_gui::canvas::{Canvas, LIVE_SHAPE_BUDGET};
use light_note_gui::doc::Edit;
use light_note_gui::geom::{Pt, Size};
use light_note_gui::ink::{InkPoint, Rgba, Style, Tool};
use light_note_gui::input::{self, Device, Phase};
use light_note_gui::otd::shm::{PenSample, FLAG_ERASER, FLAG_OUT_OF_RANGE, FLAG_TIP};
use light_note_gui::otd::{Feed, TabletSpec};
use light_note_gui::render;
use light_note_gui::tool::CanvasTool;

/// 제스처 안의 시각 — 표본마다 조금씩 흐른다(속도 → 압력).
fn at(step: u64) -> Instant {
    Instant::now() + Duration::from_millis(step)
}

/// ②로 획 하나를 긋는다 — press → drag… → lift.
fn draw(tool: &mut CanvasTool, canvas: &mut Canvas, points: &[Pt]) -> Option<Edit> {
    assert!(points.len() >= 2, "획은 점이 둘 이상이어야 한다");
    tool.press(canvas.doc_mut(), points[0], at(0));
    for (index, point) in points.iter().enumerate().skip(1) {
        tool.drag(canvas.doc_mut(), *point, at(index as u64 * 8));
    }
    tool.lift(canvas.doc_mut())
}

/// 일직선 위의 점들(간격 1pt — 최소 간격보다 크다).
fn line(from: Pt, to: Pt, count: usize) -> Vec<Pt> {
    (0..count)
        .map(|index| {
            let t = index as f32 / (count - 1) as f32;
            Pt::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t)
        })
        .collect()
}

/// ④-워커를 **테스트 안에서** 대신 돌린다(워커 본체는 순수 함수다).
fn bake_now(canvas: &mut Canvas) -> bool {
    let request = canvas.bake_request();
    let result = if request.is_empty() {
        request.empty_result()
    } else {
        render::bake(&request)
    };
    canvas.accept(result).unwrap_or(false)
}

// ── ① HardwareInput (OTD) ───────────────────────────────────────────

/// 실측한 태블릿 — 이 PC의 XP-Pen Deco 01 V3 (Variant 2).
fn deco() -> TabletSpec {
    TabletSpec {
        name: "XP-Pen Deco 01 V3 (Variant 2)".to_string(),
        max_x: 51196.0,
        max_y: 31826.0,
        max_pressure: 16383.0,
    }
}

/// 공유 메모리에서 온 표본 하나 — 계약(`otd::shm`)대로 만든다.
fn pen_sample(x: f32, y: f32, pressure: f32, flags: u32) -> PenSample {
    PenSample {
        seq: 1,
        x,
        y,
        pressure,
        tilt: None,
        eraser: flags & FLAG_ERASER != 0,
        buttons: 0,
        contact: flags & FLAG_TIP != 0,
        out_of_range: flags & FLAG_OUT_OF_RANGE != 0,
    }
}

/// 표본 흐름 하나 — 태블릿 범위와 페이지 크기로.
fn feed() -> Feed {
    Feed::new(&deco(), Size::A4)
}

#[test]
fn stage1_contact_is_what_makes_a_stroke() {
    // 표본 → 위상: 호버는 아무것도 아니고, 닿으면 시작, 떼면 끝이다.
    let mut feed = feed();
    let now = Instant::now();
    let mut out = Vec::new();
    feed.push(&pen_sample(1000.0, 1000.0, 0.0, 0), now, &mut out);
    assert!(out.is_empty(), "호버는 표본이 아니다");

    feed.push(&pen_sample(1000.0, 1000.0, 4000.0, FLAG_TIP), now, &mut out);
    feed.push(&pen_sample(2000.0, 1000.0, 5000.0, FLAG_TIP), now, &mut out);
    feed.push(&pen_sample(2000.0, 1000.0, 0.0, 0), now, &mut out);
    assert_eq!(
        out.iter().map(|sample| sample.phase).collect::<Vec<_>>(),
        vec![Phase::Pressed, Phase::Moved, Phase::Released]
    );
    assert!(
        out.iter().all(|sample| sample.is_ink()),
        "표본은 전부 펜이다"
    );
    // 좌표는 **이미 페이지 pt**다 — 창 크기(DIP)는 여기에 관여하지 않는다.
    assert!(out
        .iter()
        .all(|sample| sample.at.x <= Size::A4.width + 0.01));
}

#[test]
fn stage1_pressure_and_tilt_come_from_the_tablet() {
    // 필압은 **장치가 보고한 값**을 장치의 최대값으로 나눈 것이다(1024가 아니다).
    let mut feed = feed();
    let now = Instant::now();
    let mut out = Vec::new();
    let mut pressed = pen_sample(1000.0, 1000.0, 16383.0, FLAG_TIP);
    pressed.tilt = Some((12.5, -30.0));
    feed.push(&pressed, now, &mut out);
    assert_eq!(out[0].pressure(), Some(1.0));
    assert_eq!(out[0].tilt(), Some((12.5, -30.0)));
    assert_eq!(out[0].device(), Device::Pen);

    // 범위 밖의 필압도 잉크 폭은 0~1이다(장치가 이상한 값을 보내도).
    let mut clamped = Vec::new();
    feed.push(
        &pen_sample(1100.0, 1000.0, 999_999.0, FLAG_TIP),
        now,
        &mut clamped,
    );
    assert_eq!(clamped[0].pressure(), Some(1.0));
}

#[test]
fn stage1_leaving_the_tablet_ends_the_stroke() {
    // 범위 이탈은 좌표가 무효다 — 진행 중 획을 **커밋**하는 신호다(버리지 않는다).
    let mut feed = feed();
    let now = Instant::now();
    let mut out = Vec::new();
    feed.push(&pen_sample(1000.0, 1000.0, 4000.0, FLAG_TIP), now, &mut out);
    out.clear();
    feed.push(&pen_sample(0.0, 0.0, 0.0, FLAG_OUT_OF_RANGE), now, &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].phase, Phase::Released);
    assert_eq!(out[0].pressure(), Some(0.0), "떨어진 펜은 필압이 없다");
}

#[test]
fn stage1_a_flipped_pen_says_so() {
    // 지우개 비트는 프레임의 **자세**로 온다 — ②가 그 자세를 정책으로 바꾼다(지우기).
    let mut feed = feed();
    let now = Instant::now();
    let mut out = Vec::new();
    feed.push(
        &pen_sample(1000.0, 1000.0, 4000.0, FLAG_TIP | FLAG_ERASER),
        now,
        &mut out,
    );
    assert!(out[0].inverted(), "뒤집힘은 프레임이 들고 온다");
    assert!(out[0].is_ink(), "뒤집힌 펜도 **펜**이다(자격은 그대로)");
    assert_eq!(out[0].pressure(), Some(4000.0 / 16383.0));

    // 뒤집히지 않은 펜은 뒤집힘이 아니다. 회전은 0.6.7에 리포트가 없어 언제나 `None`이다.
    let mut straight = Vec::new();
    feed.push(
        &pen_sample(1000.0, 1000.0, 4000.0, FLAG_TIP),
        now,
        &mut straight,
    );
    assert!(!straight[0].inverted());
    assert_eq!(straight[0].frame.expect("프레임").rotation, None);
}

#[test]
fn stage1_only_a_pen_makes_ink() {
    // 손가락·마우스는 잉크가 아니다(정책은 ②에 있다). 그리고 표본을 만드는 곳이 태블릿
    // 하나뿐이라 **그런 표본은 아예 생기지 않는다** — 그래도 판정은 남는다.
    assert!(CanvasTool::accepts(Device::Pen));
    assert!(!CanvasTool::accepts(Device::Touch));
    assert!(!CanvasTool::accepts(Device::Mouse));
    let bare = input::Sample {
        phase: Phase::Pressed,
        at: Pt::new(10.0, 10.0),
        now: Instant::now(),
        frame: None,
    };
    assert_eq!(
        bare.device(),
        Device::Mouse,
        "프레임이 없으면 장치를 모른다"
    );
    assert!(!bare.is_ink());
}

#[test]
fn stage1_a_broken_stream_cancels() {
    // 플러그인이 사라졌다 — 반쪽 획을 문서에 남기지 않는다(커밋하지 않는다).
    let mut feed = feed();
    let now = Instant::now();
    let mut out = Vec::new();
    feed.push(&pen_sample(1000.0, 1000.0, 4000.0, FLAG_TIP), now, &mut out);
    out.clear();
    feed.finish(now, &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].phase, Phase::Canceled);
    assert!(Phase::Canceled.is_end());
    assert!(Phase::Released.is_end());
    assert!(!Phase::Pressed.is_end() && !Phase::Moved.is_end());
}

// ── ② CanvasTool ────────────────────────────────────────────────────

#[test]
fn stage2_a_flipped_pen_erases_without_changing_the_tool() {
    // `PEN_FLAG_INVERTED` — 펜을 뒤집으면 **그 제스처만** 지운다: 도구 선택은 그대로라
    // 뒤집기를 풀면 원래 도구로 계속 그린다. 지우는 규칙은 지우개 도구와 **같은 길**이다.
    let mut canvas = Canvas::new(Size::A4);
    let mut tool = CanvasTool::new();
    draw(
        &mut tool,
        &mut canvas,
        &line(Pt::new(10.0, 10.0), Pt::new(60.0, 10.0), 8),
    );
    assert_eq!(canvas.doc().strokes().len(), 1);

    // 뒤집힌 펜으로 문지른다 — 획이 **안 생기고** 있던 획이 사라진다.
    tool.press_inverted(canvas.doc_mut(), Pt::new(10.0, 10.0), at(0), Some(0.5));
    tool.drag_with(canvas.doc_mut(), Pt::new(60.0, 10.0), at(8), Some(0.5));
    let edit = tool.lift(canvas.doc_mut());
    assert!(
        matches!(edit, Some(Edit::RemoveStrokes { .. })),
        "뒤집힌 펜은 지운다"
    );
    assert_eq!(canvas.doc().strokes().len(), 0);
    assert_eq!(tool.tool(), Tool::Pen, "도구 선택은 바뀌지 않는다");

    // 되돌리기 **한 번**으로 돌아온다(지우개와 같은 편집 하나).
    canvas.edit(|doc| doc.undo());
    assert_eq!(canvas.doc().strokes().len(), 1);
}

#[test]
fn stage2_only_the_pen_is_accepted() {
    // 필기 정책은 ②에 있다 — 장치 판정은 ①이 하고(WM_POINTER), ②가 그 판정을 적용한다.
    assert!(CanvasTool::accepts(Device::Pen));
    assert!(!CanvasTool::accepts(Device::Touch));
    assert!(!CanvasTool::accepts(Device::Mouse));
}

#[test]
fn stage2_hardware_pressure_wins_over_speed() {
    // 압력의 출처는 하나다: 하드웨어가 보고했으면 그것, 아니면 속도 추정.
    let mut canvas = Canvas::new(Size::A4);
    let mut tool = CanvasTool::new();
    tool.press_with(canvas.doc_mut(), Pt::new(10.0, 10.0), at(0), Some(0.2));
    tool.drag_with(canvas.doc_mut(), Pt::new(30.0, 10.0), at(8), Some(0.9));
    let Some(Edit::AddStroke { stroke, .. }) = tool.lift(canvas.doc_mut()) else {
        panic!("펜 획은 커밋돼야 한다");
    };
    let pressures: Vec<f32> = stroke.points().iter().map(|point| point.pressure).collect();
    assert_eq!(pressures, vec![0.2, 0.9], "필압이 그대로 점에 들어간다");

    let style = Style::for_tool(Tool::Pen);
    assert!(
        style.width_at(0.2) < style.width_at(0.9),
        "필압이 굵기를 만든다"
    );
}

#[test]
fn stage2_without_hardware_pressure_speed_still_drives_width() {
    // 압력을 안 보내는 디지타이저여도 필기는 된다 — 그때는 속도가 굵기를 만든다.
    let mut canvas = Canvas::new(Size::A4);
    let mut tool = CanvasTool::new();
    let edit = draw(
        &mut tool,
        &mut canvas,
        &line(Pt::new(10.0, 10.0), Pt::new(90.0, 40.0), 20),
    );
    let Some(Edit::AddStroke { stroke, .. }) = edit else {
        panic!("획이 커밋돼야 한다");
    };
    assert!(
        stroke
            .points()
            .iter()
            .any(|point| point.pressure < InkPoint::DEFAULT_PRESSURE),
        "하드웨어 필압이 없으면 속도가 굵기를 만든다"
    );
}

#[test]
fn stage2_a_drag_becomes_exactly_one_edit() {
    let mut canvas = Canvas::new(Size::A4);
    let mut tool = CanvasTool::new();

    let edit = draw(
        &mut tool,
        &mut canvas,
        &line(Pt::new(10.0, 10.0), Pt::new(90.0, 40.0), 20),
    );
    assert!(matches!(edit, Some(Edit::AddStroke { index: 0, .. })));
    assert_eq!(canvas.doc().strokes().len(), 1);

    // 되돌리기 한 번이면 드래그 전체가 사라진다.
    assert!(canvas.edit(|doc| doc.undo()).is_some());
    assert_eq!(canvas.doc().strokes().len(), 0);
    assert!(canvas.doc().can_redo());
}

#[test]
fn stage2_committing_a_stroke_does_not_invalidate_the_base() {
    // 추가는 접두사를 낡게 하지 않는다 — 그 획은 꼬리가 그린다(R3).
    let mut canvas = Canvas::new(Size::A4);
    let mut tool = CanvasTool::new();
    assert!(
        bake_now(&mut canvas),
        "빈 페이지도 빈 그림으로 접두사를 세운다"
    );
    canvas.promote();
    assert!(!canvas.needs_bake(false), "빈 페이지는 구울 것이 없다");

    draw(
        &mut tool,
        &mut canvas,
        &line(Pt::new(10.0, 10.0), Pt::new(60.0, 10.0), 8),
    );
    assert!(
        !canvas.needs_bake(false),
        "예산 안의 추가는 베이크를 부르지 않는다"
    );
    assert!(
        !canvas.needs_bake(true),
        "요청이 가 있으면 더 만들지 않는다"
    );
}

#[test]
fn stage2_cancel_leaves_nothing_behind() {
    let mut canvas = Canvas::new(Size::A4);
    let mut tool = CanvasTool::new();

    tool.press(canvas.doc_mut(), Pt::new(10.0, 10.0), at(0));
    tool.drag(canvas.doc_mut(), Pt::new(40.0, 30.0), at(8));
    assert!(tool.is_active());
    assert!(tool.drawing().is_some());

    assert!(tool.cancel(canvas.doc_mut()));
    assert!(!tool.is_active());
    assert_eq!(
        canvas.doc().strokes().len(),
        0,
        "취소한 획은 어디에도 남지 않는다"
    );
    assert!(!canvas.doc().can_undo(), "취소는 히스토리에도 남지 않는다");
}

#[test]
fn stage2_eraser_drag_is_one_edit_and_cancel_restores() {
    let mut canvas = Canvas::new(Size::A4);
    let mut tool = CanvasTool::new();
    for y in [10.0_f32, 30.0, 50.0] {
        draw(
            &mut tool,
            &mut canvas,
            &line(Pt::new(10.0, y), Pt::new(60.0, y), 8),
        );
    }
    assert_eq!(canvas.doc().strokes().len(), 3);

    // 지우개로 문지른다 — 드래그 하나가 편집 하나다.
    tool.select(Tool::Eraser);
    tool.press(canvas.doc_mut(), Pt::new(10.0, 10.0), at(0));
    tool.drag(canvas.doc_mut(), Pt::new(30.0, 30.0), at(8));
    tool.drag(canvas.doc_mut(), Pt::new(60.0, 50.0), at(16));
    let edit = tool.lift(canvas.doc_mut());
    assert_eq!(canvas.doc().strokes().len(), 0);
    assert!(matches!(edit, Some(Edit::RemoveStrokes { .. })));

    // 되돌리기 **한 번**으로 셋이 모두 돌아온다.
    canvas.edit(|doc| doc.undo());
    assert_eq!(canvas.doc().strokes().len(), 3);

    // 이번에는 문지르다가 취소한다 — 즉시 사라지고, 취소하면 돌아오고, 히스토리는 그대로다.
    tool.press(canvas.doc_mut(), Pt::new(10.0, 10.0), at(0));
    assert!(
        canvas.doc().strokes().len() < 3,
        "문지르는 동안 즉시 사라진다"
    );
    tool.drag(canvas.doc_mut(), Pt::new(60.0, 50.0), at(8));
    let left = canvas.doc().strokes().len();
    assert!(tool.cancel(canvas.doc_mut()));
    assert_eq!(
        canvas.doc().strokes().len(),
        3,
        "취소는 되돌린다(문지르는 동안 {}개였다)",
        left
    );
    assert!(
        canvas.doc().can_undo(),
        "취소는 히스토리에 영향을 주지 않는다"
    );
}

#[test]
fn stage2_width_steps_are_clamped() {
    let mut tool = CanvasTool::new();
    let start = tool.state().width_pt;
    tool.resize(true);
    assert!(tool.state().width_pt > start);
    for _ in 0..40 {
        tool.resize(true);
    }
    assert!(tool.state().width_pt <= Style::MAX_WIDTH_PT);
    for _ in 0..80 {
        tool.resize(false);
    }
    assert!(tool.state().width_pt >= Style::MIN_WIDTH_PT);

    // 도구를 바꾸면 그 도구의 기본 스타일로 돌아간다.
    tool.select(Tool::Highlighter);
    assert_eq!(tool.style().color, Rgba::HIGHLIGHT_YELLOW);
}

// ── ③ Canvas ────────────────────────────────────────────────────────

#[test]
fn stage3_the_tail_starts_where_the_base_ends() {
    let mut canvas = Canvas::new(Size::A4);
    let mut tool = CanvasTool::new();

    // 접두사도 꼬리도 없다.
    canvas.refresh(None);
    assert_eq!(canvas.baked_count(), 0);
    assert!(canvas.tail().is_empty());

    // 획 하나를 확정한다 — 아직 안 구웠으므로 **꼬리가 그린다**.
    draw(
        &mut tool,
        &mut canvas,
        &line(Pt::new(10.0, 10.0), Pt::new(60.0, 10.0), 8),
    );
    canvas.refresh(None);
    assert_eq!(canvas.baked_count(), 0);
    assert_eq!(canvas.tail().len(), 1, "확정 획은 꼬리에 있다");
    assert!(canvas.live_shapes() > 0, "화면은 꼬리로 완전하다(R3)");

    // ④-워커를 돌려 승격하면 꼬리에서 빠지고 접두사가 된다.
    assert!(bake_now(&mut canvas));
    assert!(canvas.promote());
    canvas.refresh(None);
    assert_eq!(canvas.baked_count(), 1);
    assert!(canvas.tail().is_empty(), "구운 획은 꼬리에서 빠진다");

    // 승격은 **디코드 신호에 한 번만** — 신호가 더 와도 두 번 올리지 않는다.
    assert!(!canvas.promote());
}

#[test]
fn stage3_a_late_answer_is_dropped() {
    let mut canvas = Canvas::new(Size::A4);
    let mut tool = CanvasTool::new();
    draw(
        &mut tool,
        &mut canvas,
        &line(Pt::new(10.0, 10.0), Pt::new(60.0, 10.0), 8),
    );

    // 요청을 만들고, 그 사이 문서가 바뀌면(지우개) 그 응답은 쓸 수 없다.
    let request = canvas.bake_request();
    canvas.invalidate();
    let result = render::bake(&request);
    assert_eq!(canvas.accept(result), None, "낡은 응답은 버린다");
    assert!(canvas.needs_bake(false), "그리고 다시 구우라고 알린다");
}

#[test]
fn stage3_erasing_invalidates_the_base() {
    let mut canvas = Canvas::new(Size::A4);
    let mut tool = CanvasTool::new();
    draw(
        &mut tool,
        &mut canvas,
        &line(Pt::new(10.0, 10.0), Pt::new(60.0, 10.0), 8),
    );
    bake_now(&mut canvas);
    canvas.promote();
    canvas.refresh(None);
    assert_eq!(canvas.baked_count(), 1);
    assert!(!canvas.needs_bake(false));

    // 지운 획은 접두사에 반영돼야 한다 — 픽셀에는 뺄셈이 없다.
    tool.select(Tool::Eraser);
    tool.press(canvas.doc_mut(), Pt::new(10.0, 10.0), at(0));
    tool.lift(canvas.doc_mut());
    canvas.invalidate();
    canvas.refresh(None);
    assert!(canvas.needs_bake(false), "지우개는 베이크를 강제한다");
    assert!(canvas.tail().is_empty(), "남은 획이 없으면 꼬리도 없다");
}

#[test]
fn stage3_page_move_drops_the_staged_picture() {
    let mut canvas = Canvas::new(Size::A4);
    let mut tool = CanvasTool::new();
    draw(
        &mut tool,
        &mut canvas,
        &line(Pt::new(10.0, 10.0), Pt::new(60.0, 10.0), 8),
    );
    canvas.edit(|doc| {
        doc.add_page_after_active(Size::A4);
    });
    assert_eq!(canvas.doc().page_count(), 2);
    assert_eq!(canvas.doc().active_index(), 1);

    // 2페이지 그림을 구워 대기 자리에 넣었다가 1페이지로 돌아오면, 승격하지 않는다.
    let request = canvas.bake_request();
    let result = render::bake(&request);
    assert_eq!(canvas.accept(result), Some(true), "대기 자리에 들어간다");
    canvas.go_to_page(0);
    assert!(!canvas.promote(), "다른 페이지의 그림은 올리지 않는다");
    assert!(canvas.needs_bake(false), "지금 페이지를 다시 구워야 한다");
}

#[test]
fn stage3_budget_caps_the_ui_shape_count() {
    // 예산 안이면 베이크를 부르지 않고, 넘으면 부른다 — UI 도형 수의 상한이 곧 규칙이다(R1).
    let mut canvas = Canvas::new(Size::A4);
    let mut tool = CanvasTool::new();
    bake_now(&mut canvas);
    canvas.promote();

    // 점 1000개짜리 긴 획 = 구간 999개 → 예산을 넘는다(획 하나가 아니라 **합**으로 판단).
    draw(
        &mut tool,
        &mut canvas,
        &line(Pt::new(5.0, 5.0), Pt::new(590.0, 400.0), 1000),
    );
    canvas.refresh(None);
    assert!(
        canvas.live_shapes() > LIVE_SHAPE_BUDGET,
        "긴 획은 예산을 넘는다: {}",
        canvas.live_shapes()
    );
    assert!(canvas.needs_bake(false), "예산을 넘으면 베이크를 요청한다");

    // 구우면 꼬리가 비고, 화면에 남는 도형은 없다(진행 중 획이 없으므로).
    bake_now(&mut canvas);
    canvas.promote();
    canvas.refresh(None);
    assert_eq!(canvas.baked_count(), 1);
    assert_eq!(canvas.live_shapes(), 0);
    assert!(!canvas.needs_bake(false));
}

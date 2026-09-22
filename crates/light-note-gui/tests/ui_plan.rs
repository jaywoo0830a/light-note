//! 화면 계약 — elm 계획(plan)과 `<Raw>` 경계를 **헤드리스로** 검증한다.
//!
//! 화면은 플랫폼을 모르므로 여기서 잡히는 회귀는 WinUI에서도 그대로 회귀다(계획 → 컨트롤
//! 번역은 기계적이다). 그리고 `<Raw>`가 유일한 WinUI 접점이라는 규칙도 여기서 못박는다.

use std::cell::RefCell;
use std::rc::Rc;

use elm_magic::prelude::*;
use elm_magic_windows_reactor::{plan, Pass, PlanNode};

use light_note_gui::geom::{Pt, Scale, Size};
use light_note_gui::ink::{InkPoint, Stroke, Style, Tool};
use light_note_gui::shape::live_ink;
use light_note_gui::ui::{
    clear_frame, clear_surface_builder, page_label, set_surface_builder, stage_frame, Frame,
    Intent, Screen, ScreenProps, Stage, SurfaceSlot, ViewModel,
};

/// 툴바 버튼 18개 — **라벨은 사용자가 읽는 문자열**이므로 테스트가 그대로 못박는다.
const TOOLBAR: [(Intent, &str); 18] = [
    (Intent::Pen, "펜"),
    (Intent::Highlighter, "형광펜"),
    (Intent::Eraser, "지우개"),
    (Intent::Thinner, "가늘게"),
    (Intent::Thicker, "굵게"),
    (Intent::Undo, "되돌리기"),
    (Intent::Redo, "다시하기"),
    (Intent::Clear, "페이지 비우기"),
    (Intent::Open, "PDF 열기"),
    (Intent::ExportPng, "PNG 저장"),
    (Intent::ExportPdf, "PDF 저장"),
    (Intent::PageAdd, "페이지 추가"),
    (Intent::PageRemove, "페이지 삭제"),
    (Intent::PagePrev, "이전 페이지"),
    (Intent::PageNext, "다음 페이지"),
    (Intent::ZoomIn, "확대"),
    (Intent::ZoomOut, "축소"),
    (Intent::Retry, "다시 시도"),
];

/// 화면에 내려가는 값 하나 — 호스트가 진실을 소유하므로 테스트가 그 값을 정한다.
fn view(tool: Tool, stage: Stage) -> ViewModel {
    ViewModel {
        tool,
        width_pt: Style::for_tool(tool).width_pt,
        zoom: 100.0,
        page: 0,
        page_count: 1,
        stroke_count: 0,
        live_shapes: 0,
        baked: 0,
        can_undo: false,
        can_redo: false,
        dirty: false,
        title: "무제".to_string(),
        pdf_name: String::new(),
        stage,
        status: "무제 · 1 / 1페이지 · 획 0개".to_string(),
        page_labels: vec![page_label(0, 0)],
    }
}

/// 의도를 기록하는 콜백 하나 — 화면에는 이것 하나만 있다.
fn props(view: ViewModel, log: &Rc<RefCell<Vec<Intent>>>) -> ScreenProps {
    let log = Rc::clone(log);
    ScreenProps {
        view: Some(view),
        on_intent: Some(Callback::new(move |_arena: &mut Arena, intent: Intent| {
            log.borrow_mut().push(intent);
        })),
        ..Default::default()
    }
}

fn plan_of(view: ViewModel) -> (PlanNode, Pass) {
    let mut ctx = Ctx::new();
    let tree =
        elm_magic::frame::<Screen>(&mut ctx, &props(view, &Rc::new(RefCell::new(Vec::new()))));
    plan(&tree)
}

#[test]
fn the_toolbar_has_one_button_per_intent() {
    let (node, pass) = plan_of(view(Tool::Pen, Stage::Ready));
    assert_eq!(node.control(), "StackPanel");
    assert_eq!(pass.count("Button"), 18, "의도 하나 = 버튼 하나");

    for (intent, label) in TOOLBAR {
        if intent == Intent::Retry {
            assert!(
                !pass.has_label(label),
                "복구 버튼은 **실패 화면에만** 있다 — 평소 화면을 어지럽히지 않는다"
            );
            continue;
        }
        assert!(
            pass.has_label(label),
            "`{}` 버튼이 계획에 없다(의도 {intent:?})",
            label
        );
    }
    assert!(pass.has_label("단축키"), "안내 패널은 버튼으로도 열린다");
}

#[test]
fn every_button_reports_its_intent() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut app = elm_magic::mount_with::<Screen>(props(view(Tool::Pen, Stage::Ready), &log));

    // 실패 화면 전용 버튼을 뺀 나머지를 차례로 누른다.
    for (intent, label) in TOOLBAR {
        if intent == Intent::Retry {
            continue;
        }
        app.click(label);
    }

    let expected: Vec<Intent> = TOOLBAR
        .iter()
        .filter(|(intent, _)| *intent != Intent::Retry)
        .map(|(intent, _)| *intent)
        .collect();
    assert_eq!(
        log.borrow().as_slice(),
        expected.as_slice(),
        "버튼 → 의도 매핑이 정확해야 한다"
    );
}

#[test]
fn retry_is_reachable_where_the_failure_is_shown() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let failed = view(Tool::Pen, Stage::Failed("암호화된 PDF".to_string()));
    let mut app = elm_magic::mount_with::<Screen>(props(failed, &log));

    app.click("다시 시도");
    assert_eq!(log.borrow().as_slice(), [Intent::Retry]);
}

#[test]
fn the_stage_decides_what_the_surface_area_shows() {
    // 빈 페이지: 안내 + 표면. 표면이 없으면 그릴 수가 없다.
    let (_, pass) = plan_of(view(Tool::Pen, Stage::Empty));
    assert!(
        pass.has_text("여기에 필기하세요 — 펜으로 그리면 됩니다"),
        "빈 상태는 무엇을 하면 되는지 말한다"
    );
    assert_eq!(pass.count("Raw"), 1, "빈 페이지에도 표면은 있어야 한다");

    // 준비됨: 안내는 사라지고 표면만 남는다.
    let (_, pass) = plan_of(view(Tool::Pen, Stage::Ready));
    assert!(!pass.has_text("여기에 필기하세요 — 펜으로 그리면 됩니다"));
    assert_eq!(pass.count("Raw"), 1);

    // 여는 중: 표면을 만들지 않는다(아직 문서가 없다).
    let (_, pass) = plan_of(view(Tool::Pen, Stage::Loading));
    assert!(pass.has_text("PDF 여는 중…"));
    assert_eq!(pass.count("Raw"), 0, "여는 중에는 표면을 만들지 않는다");

    // 실패: 이유 + 같은 자리에 복구 동선.
    let (_, pass) = plan_of(view(Tool::Pen, Stage::Failed("암호화된 PDF".to_string())));
    assert!(
        pass.has_text("암호화된 PDF"),
        "이유를 보여줘야 복구할 수 있다"
    );
    assert!(pass.has_label("다시 시도"));
    assert_eq!(pass.count("Raw"), 0, "실패했으면 표면도 없다");
}

#[test]
fn the_status_bar_reflects_tool_zoom_and_flags() {
    let mut status = view(Tool::Highlighter, Stage::Ready);
    status.zoom = 150.0;
    status.width_pt = 14.0;
    status.stroke_count = 3;
    status.baked = 2;
    status.live_shapes = 5;
    status.can_undo = true;
    status.dirty = true;

    let (_, pass) = plan_of(status);
    assert!(pass.has_text("도구: 형광펜"));
    assert!(pass.has_text("굵기 14.0pt"));
    assert!(pass.has_text("배율 150%"));
    assert!(pass.has_text("획 3개"));
    assert!(
        pass.has_text("베이스 2획 · 라이브 5도형"),
        "파이프라인 상태가 한 줄로 보인다"
    );
    assert!(pass.has_text("되돌릴 수 있음"));
    assert!(pass.has_text("저장 안 됨"));
    assert!(
        !pass.has_text("다시할 수 없음"),
        "다시하기가 없으면 표시도 없다"
    );
    assert!(pass.has_text(Style::hint(Tool::Highlighter)));
}

#[test]
fn the_sidebar_lists_every_page() {
    let mut view = view(Tool::Pen, Stage::Ready);
    view.page = 1;
    view.page_count = 2;
    view.page_labels = vec![page_label(0, 4), page_label(1, 0)];

    let (_, pass) = plan_of(view);
    assert!(pass.has_text("페이지 2 / 2"));
    assert!(pass.has_text("1페이지 · 획 4개"));
    assert!(pass.has_text("2페이지 · 획 0개"));
}

#[test]
fn the_shortcut_panel_opens_with_f1_and_closes_with_escape() {
    let mut app = elm_magic::mount_with::<Screen>(props(
        view(Tool::Pen, Stage::Ready),
        &Rc::new(RefCell::new(Vec::new())),
    ));
    assert!(!app.text().contains("F1 안내 열기/닫기"));

    app.press_key("F1");
    assert!(app.text().contains("F1 안내 열기/닫기"), "F1로 열린다");
    app.press_key("Escape");
    assert!(!app.text().contains("F1 안내 열기/닫기"), "Esc로 닫힌다");

    // 같은 패널을 **버튼으로도** 열 수 있다(키 라우팅이 제한적인 어댑터의 우회로).
    app.click("단축키");
    assert!(app.text().contains("F1 안내 열기/닫기"));
    app.click("단축키");
    assert!(!app.text().contains("F1 안내 열기/닫기"));
}

thread_local! {
    /// 빌더가 받은 재료 — 호스트 대역(표면 빌더는 UI 스레드에서만 돈다).
    static SEEN: RefCell<Option<Frame>> = const { RefCell::new(None) };
}

/// 표면 빌더 대역 — WinUI 뷰는 창이 있어야 만들 수 있으므로 `None`을 돌려준다.
fn recording_builder(frame: &Frame) -> SurfaceSlot {
    SEEN.with(|slot| *slot.borrow_mut() = Some(frame.clone()));
    None
}

#[test]
fn the_surface_reaches_the_registered_builder_through_one_raw_slot() {
    clear_frame();
    clear_surface_builder();
    set_surface_builder(Rc::new(recording_builder));

    // ③이 만든 재료 — 호스트가 `view()` 안에서 `stage_frame`으로 넘기는 것과 같다.
    let scale = Scale::new(1.5);
    let stroke = Stroke::new(
        Tool::Pen,
        Style::for_tool(Tool::Pen),
        InkPoint::at(Pt::new(10.0, 10.0)),
    );
    let frame = Frame {
        tail: Rc::from(vec![live_ink(&stroke, scale)]),
        size: Size::new(200.0, 100.0),
        scale,
        baked: 2,
        strokes: 3,
        ..Default::default()
    };
    stage_frame(frame.clone());

    let mut ctx = Ctx::new();
    let tree = elm_magic::frame::<Screen>(
        &mut ctx,
        &props(
            view(Tool::Pen, Stage::Ready),
            &Rc::new(RefCell::new(Vec::new())),
        ),
    );
    let (_, pass) = plan(&tree);
    assert_eq!(pass.count("Raw"), 1, "표면은 <Raw> **하나**로만 붙는다");

    let mut slot: SurfaceSlot = None;
    elm_magic::raw::invoke(&tree, &mut slot);
    // Windows의 슬롯은 `Option<View>`이고 뷰는 WinUI 스레드에서만 만들어진다 —
    // 헤드리스에서는 채울 수 없다. 그래서 **빌더가 불렸다**는 사실을 받은 재료로 확인한다.
    assert!(slot.is_none(), "헤드리스에서는 WinUI 뷰를 만들 수 없다");

    let seen = SEEN
        .with(|slot| slot.borrow().clone())
        .expect("빌더가 받은 재료");
    assert_eq!(seen.size, frame.size);
    assert_eq!(seen.scale, frame.scale);
    assert_eq!(seen.baked, frame.baked);
    assert_eq!(seen.strokes, frame.strokes);
    assert_eq!(seen.tail.len(), frame.tail.len(), "꼬리 도형이 그대로 간다");

    // 재료는 **프레임당 유지**된다 — 같은 발행이 여러 번 그려도 같은 것을 본다.
    SEEN.with(|slot| *slot.borrow_mut() = None);
    let mut again: SurfaceSlot = None;
    elm_magic::raw::invoke(&tree, &mut again);
    assert!(
        SEEN.with(|slot| slot.borrow().is_some()),
        "재료는 소비되지 않는다"
    );

    // 발행이 끝나면 비운다 — 다음 프레임이 옛 재료로 그려지면 안 된다.
    clear_frame();
    SEEN.with(|slot| *slot.borrow_mut() = None);
    let mut empty: SurfaceSlot = None;
    elm_magic::raw::invoke(&tree, &mut empty);
    assert!(
        SEEN.with(|slot| slot.borrow().is_none()),
        "재료가 없으면 빌더도 불리지 않는다"
    );

    clear_surface_builder();
}

#[test]
fn a_surface_without_a_builder_leaves_the_slot_empty() {
    // 호스트가 빌더를 등록하지 않은 상태(테스트/헤드리스)에서도 화면은 계획까지 만들어져야 한다.
    clear_frame();
    clear_surface_builder();
    stage_frame(Frame::default());

    let mut ctx = Ctx::new();
    let tree = elm_magic::frame::<Screen>(
        &mut ctx,
        &props(
            view(Tool::Pen, Stage::Ready),
            &Rc::new(RefCell::new(Vec::new())),
        ),
    );
    let mut slot: SurfaceSlot = None;
    elm_magic::raw::invoke(&tree, &mut slot);
    assert!(slot.is_none());
    clear_frame();
}

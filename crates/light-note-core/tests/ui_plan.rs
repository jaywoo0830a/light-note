//! 화면(elm-magic) ↔ 어댑터 **계획(plan)** 계약.
//!
//! 리눅스에서도 도는 이유: elm 화면과 어댑터의 계획 층은 플랫폼 독립이기 때문이다.
//! 여기서 잡히는 회귀는 WinUI에서도 그대로 회귀다(계획 → 컨트롤 번역은 기계적).

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;

use elm_magic::prelude::*;
use elm_magic_windows_reactor::{plan, Pass, PlanNode};
use light_note_core::doc::Document;
use light_note_core::geom::{Point, Size};
use light_note_core::ink::Tool;
use light_note_core::surface::InkSurface;
use light_note_core::ui::{
    clear_staged_surface, clear_surface_builder, set_surface_builder, stage_surface, NoteApp,
    NoteAppProps, NoteIntents, NoteViewModel, Phase, SurfaceData, SurfaceSlot,
};

/// 표면 빌더가 받은 재료를 기록한다(호스트 대역).
static SEEN_SURFACE: Mutex<Option<SurfaceData>> = Mutex::new(None);

fn recording_builder(data: &SurfaceData) -> SurfaceSlot {
    let mut slot = SEEN_SURFACE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *slot = Some(data.clone());
    Some(())
}

fn props(view: &NoteViewModel, log: &Rc<RefCell<Vec<&'static str>>>) -> NoteAppProps {
    NoteAppProps::from_view_model(view, &NoteIntents::recording(Rc::clone(log)))
}

fn plan_for(view: &NoteViewModel) -> (PlanNode, Pass) {
    let mut ctx = Ctx::new();
    let tree =
        elm_magic::frame::<NoteApp>(&mut ctx, &props(view, &Rc::new(RefCell::new(Vec::new()))));
    plan(&tree)
}

#[test]
fn toolbar_has_one_button_per_intent() {
    let view = NoteViewModel::from_document(&Document::blank(Size::A4), Tool::Pen, 1.8, 100.0);
    let (node, pass) = plan_for(&view);

    assert_eq!(node.control(), "StackPanel");
    assert_eq!(pass.count("Button"), 18, "의도 하나 = 버튼 하나 (+단축키 패널)");
    for label in [
        "펜",
        "형광펜",
        "지우개",
        "되돌리기",
        "다시하기",
        "페이지 비우기",
        "PDF 열기",
        "PNG 저장",
        "PDF 저장",
        "단축키",
        "페이지 추가",
        "페이지 삭제",
        "이전 페이지",
        "다음 페이지",
        "축소",
        "확대",
        "가늘게",
        "굵게",
    ] {
        assert!(pass.has_label(label), "버튼 라벨 `{label}`이 계획에 있어야 한다");
    }
    assert!(pass.has_text("굵기 1.8pt"));
    assert!(pass.has_text("도구: 펜"));
    assert!(pass.has_text(Tool::Pen.hint()));
}

#[test]
fn clicking_a_tool_button_reports_the_intent() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let view = NoteViewModel::from_document(&Document::blank(Size::A4), Tool::Pen, 1.8, 100.0);
    let mut app = elm_magic::mount_with::<NoteApp>(props(&view, &log));

    app.click("지우개");
    app.click("되돌리기");
    app.click("확대");
    app.click("PDF 저장");

    assert_eq!(
        log.borrow().as_slice(),
        ["eraser", "undo", "zoom_in", "export_pdf"],
        "버튼 → 의도 매핑이 정확해야 한다"
    );
}

#[test]
fn pdf_state_switches_the_sidebar() {
    let document = Document::blank(Size::A4);
    let with_pdf =
        NoteViewModel::from_document(&document, Tool::Pen, 1.8, 100.0).with_pdf(Some("논문.pdf"));
    let (_, pass) = plan_for(&with_pdf);
    assert!(pass.has_text("배경: 논문.pdf"));

    let without = NoteViewModel::from_document(&document, Tool::Pen, 1.8, 100.0).with_pdf(None);
    let (_, pass) = plan_for(&without);
    assert!(!pass.has_text("배경: 논문.pdf"), "PDF가 없으면 안내도 없다");
}

#[test]
fn loading_and_failure_use_the_documented_controls() {
    let document = Document::blank(Size::A4);
    let loading =
        NoteViewModel::from_document(&document, Tool::Pen, 1.8, 100.0).with_phase(Phase::Loading);
    let (_, pass) = plan_for(&loading);
    assert_eq!(pass.count("ProgressRing"), 1, "로딩은 ProgressRing");
    assert!(pass.has_text("PDF 여는 중…"));
    assert_eq!(pass.count("Raw"), 0, "여는 중에는 표면을 만들지 않는다");

    let failed = NoteViewModel::from_document(&document, Tool::Pen, 1.8, 100.0)
        .with_phase(Phase::Failed("암호화된 PDF".to_string()));
    let (_, pass) = plan_for(&failed);
    assert_eq!(pass.count("InfoBar"), 1, "오류는 InfoBar");
    assert!(
        pass.has_text("암호화된 PDF"),
        "오류 문구가 화면에 보여야 복구 동선을 만들 수 있다"
    );
    assert!(
        pass.has_label("다시 시도"),
        "실패 화면에는 복구 동선이 같은 자리에 있어야 한다(예제 19)"
    );
}

#[test]
fn empty_page_shows_a_hint_and_still_renders_the_surface() {
    let view = NoteViewModel::from_document(&Document::blank(Size::A4), Tool::Pen, 1.8, 100.0);
    assert_eq!(view.phase, Phase::Empty, "빈 페이지는 Empty 상태");
    let (_, pass) = plan_for(&view);
    assert!(pass.has_text("여기에 필기하세요 — 펜으로 그리면 됩니다"));
    assert_eq!(pass.count("Raw"), 1, "빈 페이지에도 표면은 있어야 한다");

    // 필기를 하면 Ready로 바뀐다.
    let mut document = Document::blank(Size::A4);
    common::draw_line(
        &mut document,
        Point::new(10.0, 10.0),
        Point::new(90.0, 10.0),
        2.0,
    );
    let ready = NoteViewModel::from_document(&document, Tool::Pen, 1.8, 100.0);
    assert_eq!(ready.phase, Phase::Ready);
    let (_, pass) = plan_for(&ready);
    assert!(!pass.has_text("여기에 필기하세요 — 펜으로 그리면 됩니다"));
}

#[test]
fn retry_button_reports_the_intent() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let view = NoteViewModel::from_document(&Document::blank(Size::A4), Tool::Pen, 1.8, 100.0)
        .with_phase(Phase::Failed("손상된 파일".to_string()));
    let mut app = elm_magic::mount_with::<NoteApp>(props(&view, &log));

    app.click("다시 시도");
    assert_eq!(log.borrow().as_slice(), ["retry"]);
}

#[test]
fn f1_toggles_the_shortcut_panel() {
    let view = NoteViewModel::from_document(&Document::blank(Size::A4), Tool::Pen, 1.8, 100.0);
    let mut app = elm_magic::mount_with::<NoteApp>(props(&view, &Rc::new(RefCell::new(Vec::new()))));

    assert!(!app.text().contains("F1 안내 열기/닫기"));
    app.press_key("F1");
    assert!(app.text().contains("F1 안내 열기/닫기"), "F1로 안내가 열린다");
    app.press_key("Escape");
    assert!(!app.text().contains("F1 안내 열기/닫기"), "Esc로 닫힌다");
}

#[test]
fn shortcut_panel_is_reachable_without_keys() {
    // 어댑터 0.8.6은 호스트가 ElmView 핸들을 잡을 수 없어 키 라우팅이 제한적이다 —
    // 그래서 같은 패널을 **버튼으로도** 열 수 있어야 한다(문서화된 우회로).
    let view = NoteViewModel::from_document(&Document::blank(Size::A4), Tool::Pen, 1.8, 100.0);
    let mut app = elm_magic::mount_with::<NoteApp>(props(&view, &Rc::new(RefCell::new(Vec::new()))));

    app.click("단축키");
    assert!(app.text().contains("F1 안내 열기/닫기"));
    app.click("단축키");
    assert!(!app.text().contains("F1 안내 열기/닫기"));
}

#[test]
fn status_bar_reflects_zoom_tool_and_flags() {
    let document = Document::blank(Size::A4);
    let view = NoteViewModel::from_document(&document, Tool::Highlighter, 14.0, 150.0);
    let (_, pass) = plan_for(&view);

    assert!(pass.has_text("도구: 형광펜"));
    assert!(pass.has_text("확대 150%"));
    assert!(pass.has_text("스트로크 0개"));
    assert!(pass.has_text(Tool::Highlighter.hint()));
    assert!(!pass.has_text("되돌릴 수 있음"), "Undo할 게 없으면 표시도 없다");
    assert!(!pass.has_text("저장 안 됨"));
}

#[test]
fn view_model_follows_the_document() {
    let mut document = Document::blank(Size::A4);
    common::draw_line(
        &mut document,
        Point::new(10.0, 10.0),
        Point::new(90.0, 10.0),
        2.0,
    );
    document.add_page_after_active(Size::LETTER);

    let view = NoteViewModel::from_document(&document, Tool::Pen, 1.8, 100.0);
    assert_eq!(view.page, 1);
    assert_eq!(view.page_count, 2);
    assert_eq!(view.stroke_count, 0, "새 페이지에는 스트로크가 없다");
    assert!(view.can_undo);
    assert!(view.dirty);
    assert_eq!(view.page_labels.len(), 2);
    assert!(view.page_labels[0].contains("스트로크 1개"));

    let (_, pass) = plan_for(&view);
    assert!(pass.has_text(&view.page_labels[0]), "사이드바가 라벨을 그린다");
}

#[test]
fn surface_data_reaches_the_registered_builder_through_raw() {
    clear_staged_surface();
    clear_surface_builder();
    set_surface_builder(Rc::new(recording_builder));

    let size = Size::new(200.0, 100.0);
    let view = NoteViewModel::from_document(&Document::blank(size), Tool::Pen, 1.8, 100.0);
    let surface = InkSurface::build(size, 1.5, &[], None);
    stage_surface(SurfaceData {
        png: None,
        lines: surface.lines.clone(),
        width: surface.width,
        height: surface.height,
        scale: surface.scale,
    });

    let mut ctx = Ctx::new();
    let tree = elm_magic::frame::<NoteApp>(
        &mut ctx,
        &props(&view, &Rc::new(RefCell::new(Vec::new()))),
    );
    let (_, pass) = plan(&tree);
    assert_eq!(pass.count("Raw"), 1, "표면은 <Raw> 하나로만 붙는다");

    let mut slot: SurfaceSlot = None;
    elm_magic::raw::invoke(&tree, &mut slot);
    assert_eq!(slot, Some(()), "<Raw>가 빌더를 불러 슬롯을 채운다");

    let seen = SEEN_SURFACE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .expect("빌더가 받은 재료");
    assert_eq!((seen.width, seen.height), (surface.width, surface.height));
    assert!((seen.scale - 1.5).abs() < 1e-4);

    // 발행 한 번 = 스테이징 한 번: 두 번째 호출은 빈손이다.
    let mut second: SurfaceSlot = None;
    elm_magic::raw::invoke(&tree, &mut second);
    assert_eq!(second, None, "스테이징은 소비된다(프레임당 한 번)");

    clear_surface_builder();
}

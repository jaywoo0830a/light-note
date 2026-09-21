//! 문서/히스토리 계약 — **드래그 하나 = Undo 하나**, 페이지 관리, 취소.
//!
//! 호스트(WinUI)도 화면(elm)도 여기 규칙을 그대로 믿는다. 그래서 이 파일이
//! 코어에서 가장 중요한 계약이다.

mod common;

use common::draw_line;
use light_note_core::doc::Document;
use light_note_core::geom::{Point, Size};
use light_note_core::ink::{InkPoint, StrokeStyle, Tool};

#[test]
fn a_blank_document_starts_with_one_a4_page() {
    let document = Document::blank(Size::A4);
    assert_eq!(document.page_count(), 1);
    assert_eq!(document.active_index(), 0);
    assert_eq!(document.active_page().size(), Size::A4);
    assert!(!document.is_dirty());
    assert!(!document.can_undo() && !document.can_redo());
    assert_eq!(document.status_line(), "1 / 1페이지 · 스트로크 0개");
}

#[test]
fn a_drag_is_one_edit_and_undo_restores_the_page() {
    let mut document = Document::blank(Size::A4);
    draw_line(&mut document, Point::new(10.0, 10.0), Point::new(100.0, 10.0), 2.0);

    assert_eq!(document.active_page().stroke_count(), 1);
    assert!(document.can_undo() && !document.can_redo());
    assert!(document.is_dirty());

    let edit = document.undo().expect("Undo");
    assert_eq!(edit.label(), "필기");
    assert_eq!(edit.affected_strokes(), 1);
    assert_eq!(document.active_page().stroke_count(), 0);
    assert!(document.can_redo());

    document.redo().expect("Redo");
    assert_eq!(document.active_page().stroke_count(), 1);
}

#[test]
fn one_eraser_drag_is_one_undo_even_for_many_strokes() {
    let mut document = Document::blank(Size::A4);
    // 줄 간격(30pt)이 지우개 반경(14pt)보다 훨씬 크다 — 한 번에 하나씩 지운다.
    for y in [20.0, 50.0, 80.0] {
        draw_line(&mut document, Point::new(10.0, y), Point::new(100.0, y), 3.0);
    }
    assert_eq!(document.active_page().stroke_count(), 3);

    let eraser = StrokeStyle::for_tool(Tool::Eraser);
    document.begin(Tool::Eraser, eraser, InkPoint::new(50.0, 20.0));
    assert!(document.extend(InkPoint::new(50.0, 50.0)), "가운데 줄을 지운다");
    assert!(document.extend(InkPoint::new(50.0, 80.0)), "아래 줄을 지운다");
    let edit = document.finish().expect("지우기 편집");

    assert_eq!(edit.label(), "지우기");
    assert_eq!(edit.affected_strokes(), 3, "드래그 하나 = 편집 하나");
    assert_eq!(document.active_page().stroke_count(), 0);

    document.undo().expect("Undo");
    assert_eq!(
        document.active_page().stroke_count(),
        3,
        "지운 스트로크가 한 번에 전부 복구된다"
    );
    assert_eq!(
        document.active_page().strokes()[0].points()[0].x(),
        10.0,
        "복구된 순서도 원래대로다"
    );
}

#[test]
fn cancel_discards_the_in_progress_drag() {
    let mut document = Document::blank(Size::A4);
    document.begin(
        Tool::Pen,
        StrokeStyle::for_tool(Tool::Pen),
        InkPoint::new(5.0, 5.0),
    );
    document.extend(InkPoint::new(20.0, 5.0));
    assert!(document.is_drawing());
    assert!(document.live_stroke().is_some(), "라이브 레이어가 그릴 대상");

    assert!(document.cancel());
    assert!(!document.is_drawing());
    assert_eq!(document.active_page().stroke_count(), 0);
    assert!(!document.can_undo(), "버린 드래그는 히스토리에 남지 않는다");
}

#[test]
fn cancel_restores_strokes_removed_by_an_eraser_drag() {
    let mut document = Document::blank(Size::A4);
    draw_line(&mut document, Point::new(10.0, 50.0), Point::new(100.0, 50.0), 3.0);
    let history_before = document.history().undo_len();

    document.begin(
        Tool::Eraser,
        StrokeStyle::for_tool(Tool::Eraser),
        InkPoint::new(50.0, 50.0),
    );
    assert_eq!(
        document.active_page().stroke_count(),
        0,
        "첫 지점에서 바로 지운다"
    );

    assert!(document.cancel());
    assert_eq!(document.active_page().stroke_count(), 1, "취소하면 되살아난다");
    assert_eq!(
        document.history().undo_len(),
        history_before,
        "취소한 드래그는 히스토리에 남지 않는다"
    );
    assert_eq!(document.undo().expect("필기 Undo").label(), "필기");
    assert_eq!(document.active_page().stroke_count(), 0);
}

#[test]
fn pages_can_be_added_removed_and_selected_but_never_zero() {
    let mut document = Document::blank(Size::A4);
    let second = document.add_page_after_active(Size::LETTER);

    assert_eq!(second, 1);
    assert_eq!(document.page_count(), 2);
    assert_eq!(document.active_index(), 1, "새 페이지로 이동한다");
    assert_eq!(document.active_page().size(), Size::LETTER);

    assert!(document.select_page(0));
    assert!(!document.select_page(9), "범위 밖 선택은 무시된다");
    assert_eq!(document.active_index(), 0);

    assert!(document.remove_page(1));
    assert_eq!(document.page_count(), 1);
    assert!(
        !document.remove_page(0),
        "마지막 한 장은 지울 수 없다(앱이 빈 화면이 되면 안 된다)"
    );

    document.undo().expect("페이지 삭제 Undo");
    assert_eq!(document.page_count(), 2, "페이지 삭제도 되돌릴 수 있다");
}

#[test]
fn clear_page_is_one_undo() {
    let mut document = Document::blank(Size::A4);
    draw_line(&mut document, Point::new(10.0, 10.0), Point::new(60.0, 10.0), 2.0);
    draw_line(&mut document, Point::new(10.0, 30.0), Point::new(60.0, 30.0), 2.0);

    assert!(document.clear_active_page());
    assert_eq!(document.active_page().stroke_count(), 0);
    assert!(!document.clear_active_page(), "빈 페이지는 할 일이 없다");

    document.undo().expect("Undo");
    assert_eq!(document.active_page().stroke_count(), 2);
}

#[test]
fn status_line_reports_page_and_dirty_flag() {
    let mut document = Document::blank(Size::A4);
    document.set_title("회의록");
    assert_eq!(document.title(), "회의록");

    draw_line(&mut document, Point::new(10.0, 10.0), Point::new(60.0, 10.0), 2.0);
    assert_eq!(
        document.status_line(),
        "1 / 1페이지 · 스트로크 1개 · 저장 안 됨"
    );

    document.mark_saved();
    assert_eq!(document.status_line(), "1 / 1페이지 · 스트로크 1개");
}

#[test]
fn a_new_edit_after_undo_drops_the_redo_stack() {
    let mut document = Document::blank(Size::A4);
    draw_line(&mut document, Point::new(10.0, 10.0), Point::new(60.0, 10.0), 2.0);
    document.undo().expect("Undo");
    assert!(document.can_redo());

    draw_line(&mut document, Point::new(10.0, 40.0), Point::new(60.0, 40.0), 2.0);
    assert!(!document.can_redo(), "새 편집은 redo를 버린다");
}

#[test]
fn committed_strokes_exclude_the_live_one() {
    let mut document = Document::blank(Size::A4);
    draw_line(&mut document, Point::new(10.0, 10.0), Point::new(60.0, 10.0), 2.0);
    assert_eq!(document.committed_strokes().len(), 1);

    document.begin(
        Tool::Pen,
        StrokeStyle::for_tool(Tool::Pen),
        InkPoint::new(20.0, 30.0),
    );
    document.extend(InkPoint::new(40.0, 30.0));
    assert_eq!(
        document.active_page().stroke_count(),
        2,
        "모델에는 진행 중인 획도 이미 들어 있다"
    );
    assert_eq!(
        document.committed_strokes().len(),
        1,
        "정적 레이어는 확정분만 그린다(라이브와 겹치면 두 번 그려진다)"
    );

    document.finish();
    assert_eq!(document.committed_strokes().len(), 2);
}

#[test]
fn pdf_pages_become_document_pages_with_a_background() {
    let document = Document::from_pdf_pages(
        [common::sample_pdf_size(), common::sample_pdf_size()],
        "논문.pdf",
    );
    assert_eq!(document.page_count(), 2);
    assert_eq!(document.title(), "논문.pdf");
    assert!(!document.is_dirty(), "열기만 했으면 저장할 게 없다");
    for page in document.pages() {
        assert!(page.has_pdf_background());
    }
    assert_eq!(document.page(0).expect("0페이지").background(), Some(0));
    assert_eq!(document.page(1).expect("1페이지").background(), Some(1));
}

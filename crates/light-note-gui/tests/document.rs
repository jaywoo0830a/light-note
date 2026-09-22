//! 문서 계약 — 편집 하나 = 되돌리기 하나, 그리고 페이지/지우개/히스토리의 경계.
//!
//! ②(도구)는 **적용한 뒤에** 기록한다. 그래서 여기서 확인하는 것은 모델의 규칙이다:
//! 되돌리기가 정확히 원상복구하고, 새 편집이 다시하기를 버리고, 페이지는 최소 한 장이 남는다.

mod common;

use light_note_gui::doc::{Doc, History, Page};
use light_note_gui::geom::{distance_to_segment, Pt, Size};
use light_note_gui::ink::Tool;

/// Letter 크기(612×792pt) — A4와 다른 크기가 필요할 때.
const LETTER: Size = Size::new(612.0, 792.0);

/// 문서에 직선 하나를 커밋한다 — ②를 거치지 않고 모델에 바로.
fn commit(doc: &mut Doc, from: Pt, to: Pt) {
    let page = doc.active_index();
    let stroke = common::line_stroke(Tool::Pen, from, to, 8, 2.0);
    doc.commit_stroke(page, stroke);
}

#[test]
fn a_stroke_is_one_edit_and_undo_restores_the_page() {
    let mut doc = Doc::blank(Size::A4);
    let page = doc.active_index();

    let index = doc.commit_stroke(
        page,
        common::line_stroke(Tool::Pen, Pt::new(10.0, 10.0), Pt::new(80.0, 10.0), 8, 2.0),
    );
    assert_eq!(index, 0, "첫 획의 인덱스는 0");
    assert_eq!(doc.strokes().len(), 1);
    assert!(doc.can_undo() && !doc.can_redo());
    assert_eq!(doc.last_edit(), Some("필기"), "상태바가 편집 이름을 안다");

    assert!(doc.undo().is_some());
    assert_eq!(doc.strokes().len(), 0);
    assert!(doc.can_redo());

    assert!(doc.redo().is_some());
    assert_eq!(
        doc.strokes().len(),
        1,
        "다시하기가 같은 자리에 되돌려 놓는다"
    );
    assert_eq!(doc.total_strokes(), 1);
}

#[test]
fn erasing_several_strokes_is_one_edit() {
    let mut doc = Doc::blank(Size::A4);
    for y in [20.0_f32, 40.0, 60.0] {
        commit(&mut doc, Pt::new(10.0, y), Pt::new(90.0, y));
    }
    assert_eq!(doc.strokes().len(), 3);

    // 지우개 반지름 안에 세 획이 모두 걸리는 자리.
    let page = doc.active_index();
    let removed = doc
        .page_mut(page)
        .expect("페이지")
        .erase_at(Pt::new(50.0, 40.0), 25.0);
    assert_eq!(removed.len(), 3, "반지름 25pt 안에 세 획이 다 걸린다");
    assert!(doc.commit_erasure(page, removed));
    assert_eq!(doc.strokes().len(), 0);

    // 되돌리기 **한 번**이면 셋이 모두 제자리로 돌아온다(인덱스까지 그대로).
    assert!(doc.undo().is_some());
    assert_eq!(doc.strokes().len(), 3);
    assert_eq!(doc.strokes()[0].points()[0].pos, Pt::new(10.0, 20.0));
    assert_eq!(doc.strokes()[2].points()[0].pos, Pt::new(10.0, 60.0));

    assert!(doc.redo().is_some());
    assert_eq!(doc.strokes().len(), 0);
}

#[test]
fn erase_radius_hits_only_near_strokes() {
    let mut doc = Doc::blank(Size::A4);
    commit(&mut doc, Pt::new(10.0, 20.0), Pt::new(90.0, 20.0));
    commit(&mut doc, Pt::new(10.0, 60.0), Pt::new(90.0, 60.0));

    let page = doc.active_index();
    // 판정은 "선분까지의 거리 ≤ 반지름 + 획 반폭"이다(여기서는 15 + 1.0 = 16pt).
    // (50,40)은 두 선에서 20pt 떨어져 있다 → 아무것도 닿지 않는다.
    let removed = doc
        .page_mut(page)
        .expect("페이지")
        .erase_at(Pt::new(50.0, 40.0), 15.0);
    assert_eq!(removed.len(), 0, "어느 선에도 닿지 않는다");

    // (50,45)는 y=60 선에서 15pt — 그 선만 걸린다.
    let removed = doc
        .page_mut(page)
        .expect("페이지")
        .erase_at(Pt::new(50.0, 45.0), 15.0);
    assert_eq!(removed.len(), 1, "가까운 선 하나만 지운다");
    assert_eq!(removed[0].0, 1, "지운 것은 두 번째 획");
}

#[test]
fn hit_test_measures_distance_to_the_segment_not_the_ends() {
    // 지우개 판정은 "선분까지의 거리"다 — 끝점이 멀어도 선 위면 닿는다.
    let a = Pt::new(0.0, 0.0);
    let b = Pt::new(100.0, 0.0);
    assert_eq!(distance_to_segment(Pt::new(50.0, 0.0), a, b), 0.0);
    assert_eq!(distance_to_segment(Pt::new(50.0, 10.0), a, b), 10.0);
    // 선분 밖은 **끝점까지의** 거리다(무한 직선이 아니다).
    assert_eq!(distance_to_segment(Pt::new(140.0, 0.0), a, b), 40.0);
    assert_eq!(distance_to_segment(Pt::new(-30.0, 0.0), a, b), 30.0);
    // 길이 0인 선분(점 하나짜리 획)도 안전하다.
    assert_eq!(distance_to_segment(Pt::new(3.0, 4.0), a, a), 5.0);
}

#[test]
fn clearing_a_page_is_one_edit() {
    let mut doc = Doc::blank(Size::A4);
    for y in [20.0_f32, 40.0] {
        commit(&mut doc, Pt::new(10.0, y), Pt::new(90.0, y));
    }
    assert!(doc.clear_active_page());
    assert_eq!(doc.strokes().len(), 0);
    assert!(!doc.clear_active_page(), "빈 페이지를 비우면 편집도 없다");

    assert!(doc.undo().is_some());
    assert_eq!(doc.strokes().len(), 2, "비우기도 되돌릴 수 있다");
    assert_eq!(doc.strokes()[0].points()[0].pos, Pt::new(10.0, 20.0));
}

#[test]
fn page_insert_and_remove_round_trip() {
    let mut doc = Doc::blank(Size::A4);
    commit(&mut doc, Pt::new(10.0, 10.0), Pt::new(80.0, 10.0));

    let inserted = doc.add_page_after_active(LETTER);
    assert_eq!(inserted, 1);
    assert_eq!(doc.page_count(), 2);
    assert_eq!(doc.active_index(), 1, "새 페이지로 이동한다");
    assert_eq!(doc.active_page().size(), LETTER);
    assert_eq!(doc.strokes().len(), 0);

    assert!(doc.remove_page(0));
    assert_eq!(doc.page_count(), 1);
    assert_eq!(doc.active_page().size(), LETTER);

    // 되돌리면 **획까지 그대로** 돌아온다(페이지 편집도 한 편집이다).
    assert!(doc.undo().is_some());
    assert_eq!(doc.page_count(), 2);
    assert_eq!(doc.page(0).expect("0페이지").size(), Size::A4);
    assert_eq!(doc.page(0).expect("0페이지").stroke_count(), 1);

    assert!(doc.redo().is_some());
    assert_eq!(doc.page_count(), 1);
}

#[test]
fn the_last_page_cannot_be_removed() {
    let mut doc = Doc::blank(Size::A4);
    assert!(!doc.remove_page(0), "한 장뿐이면 지울 수 없다");
    assert!(!doc.remove_page(9), "없는 페이지도 아무 일이 없다");
    assert_eq!(doc.page_count(), 1);
}

#[test]
fn page_navigation_clamps_at_the_edges() {
    let mut doc = Doc::blank(Size::A4);
    doc.add_page_after_active(Size::A4);
    assert_eq!(doc.active_index(), 1);

    assert!(doc.step_page(-1));
    assert_eq!(doc.active_index(), 0);
    assert!(!doc.step_page(-1), "첫 페이지에서 더 못 간다");
    assert!(!doc.select_page(5), "없는 페이지는 선택되지 않는다");
    assert!(doc.select_page(1));
}

#[test]
fn a_new_edit_drops_the_redo_stack() {
    let mut doc = Doc::blank(Size::A4);
    commit(&mut doc, Pt::new(10.0, 10.0), Pt::new(80.0, 10.0));
    assert!(doc.undo().is_some());
    assert!(doc.can_redo());

    commit(&mut doc, Pt::new(10.0, 30.0), Pt::new(80.0, 30.0));
    assert!(!doc.can_redo(), "새 편집이 다시하기를 버린다");
    assert_eq!(doc.strokes().len(), 1);
}

#[test]
fn history_forgets_the_oldest_edit_beyond_the_limit() {
    let mut doc = Doc::blank(Size::A4);
    for step in 0..(History::LIMIT + 10) {
        commit(
            &mut doc,
            Pt::new(10.0, step as f32 % 500.0),
            Pt::new(80.0, step as f32 % 500.0),
        );
    }
    assert_eq!(doc.strokes().len(), History::LIMIT + 10);

    // 한계를 넘은 만큼만 되돌아간다 — 그리고 그 뒤에는 되돌릴 것이 없다.
    let mut undone = 0;
    while doc.undo().is_some() {
        undone += 1;
        assert!(undone <= History::LIMIT + 10, "되돌리기가 무한히 이어진다");
    }
    assert_eq!(
        undone,
        History::LIMIT,
        "히스토리는 최근 {}개만 기억한다",
        History::LIMIT
    );
    assert_eq!(doc.strokes().len(), 10, "한계를 넘겨 지워진 획은 남는다");
}

#[test]
fn document_from_pdf_keeps_page_sizes_and_backgrounds() {
    let doc = Doc::from_pdf([Size::A4, LETTER], "논문.pdf");
    assert_eq!(doc.title(), "논문.pdf");
    assert_eq!(doc.page_count(), 2);
    assert_eq!(doc.page(0).expect("0페이지").size(), Size::A4);
    assert_eq!(doc.page(1).expect("1페이지").size(), LETTER);
    assert_eq!(doc.page(0).expect("0페이지").background(), Some(0));
    assert_eq!(doc.page(1).expect("1페이지").background(), Some(1));
    assert!(!doc.can_undo(), "PDF를 여는 것은 편집이 아니다");
    assert!(!doc.is_dirty());
}

#[test]
fn status_line_reports_position_and_dirty() {
    let mut doc = Doc::blank(Size::A4);
    let clean = doc.status_line();
    assert!(clean.contains("1 / 1페이지"), "{clean}");
    assert!(!clean.contains("저장 안 됨"));

    commit(&mut doc, Pt::new(10.0, 10.0), Pt::new(80.0, 10.0));
    doc.add_page_after_active(Size::A4);
    assert!(doc.is_dirty());
    let dirty = doc.status_line();
    assert!(dirty.contains("2 / 2페이지"), "{dirty}");
    assert!(dirty.contains("저장 안 됨"), "{dirty}");

    doc.mark_saved();
    assert!(!doc.is_dirty());
    assert!(!doc.status_line().contains("저장 안 됨"));
}

#[test]
fn a_blank_page_has_no_strokes_and_a_stable_size() {
    let page = Page::blank(Size::A4);
    assert!(page.is_empty());
    assert_eq!(page.stroke_count(), 0);
    assert_eq!(page.background(), None);
    assert_eq!(page.describe(), "빈 페이지 · 획 0개");
}

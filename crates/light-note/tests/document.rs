//! Document contract: pages, strokes, undo/redo and the eraser.
//!
//! The document is the only truth the app has.  It is a plain value — no
//! platform type, no lock, no thread — which is what lets the UI thread apply a
//! pen sample in O(1) and hand a *snapshot* to the rasterizer.

mod support;

use light_note::doc::{Document, Edit, Page};
use light_note::geom::{Pt, Size};
use light_note::ink::{Stroke, Tool};

fn a4() -> Size {
    support::A4
}

#[test]
fn a_blank_document_has_one_empty_page() {
    let doc = Document::blank(a4());

    assert_eq!(doc.page_count(), 1);
    assert_eq!(doc.current_index(), 0);
    assert_eq!(doc.page().size, a4());
    assert_eq!(doc.page().background, None);
    assert_eq!(doc.page().strokes.len(), 0);
    assert_eq!(doc.total_strokes(), 0);
    assert!(!doc.can_undo());
    assert!(!doc.can_redo());
}

#[test]
fn a_pdf_document_has_one_page_per_pdf_page_with_a_background_index() {
    let doc = Document::from_backgrounds(&[a4(), Size::new(612.0, 792.0)]);

    assert_eq!(doc.page_count(), 2);
    assert_eq!(doc.page().background, Some(0));
    assert_eq!(doc.pages()[1].background, Some(1));
    assert_eq!(doc.pages()[1].size, Size::new(612.0, 792.0));
}

#[test]
fn strokes_are_added_to_the_current_page_and_undone_as_one_edit() {
    let mut doc = Document::blank(a4());
    let stroke = support::line(support::pen(2.0), 0.0, 40.0, 10.0, 1.0);

    assert!(doc.apply(Edit::AddStroke {
        page: 0,
        stroke: stroke.clone()
    }));
    assert_eq!(doc.page().strokes.len(), 1);
    assert_eq!(doc.total_strokes(), 1);
    assert!(doc.can_undo());

    assert!(doc.undo());
    assert_eq!(doc.page().strokes.len(), 0);
    assert!(doc.can_redo());

    assert!(doc.redo());
    assert_eq!(doc.page().strokes.len(), 1);
}

#[test]
fn a_new_edit_clears_the_redo_stack() {
    let mut doc = Document::blank(a4());
    doc.add_stroke(support::line(support::pen(2.0), 0.0, 10.0, 0.0, 1.0));
    doc.undo();
    assert!(doc.can_redo());

    doc.add_stroke(support::line(support::pen(2.0), 0.0, 20.0, 5.0, 1.0));
    assert!(!doc.can_redo(), "history is linear — the old branch is gone");
}

#[test]
fn undo_and_redo_return_false_when_the_history_is_empty() {
    let mut doc = Document::blank(a4());

    assert!(!doc.undo());
    assert!(!doc.redo());
}

#[test]
fn an_edit_for_another_page_is_rejected() {
    let mut doc = Document::blank(a4());
    let stroke = support::line(support::pen(2.0), 0.0, 10.0, 0.0, 1.0);

    assert!(!doc.apply(Edit::AddStroke { page: 7, stroke }));
    assert_eq!(doc.total_strokes(), 0);
    assert!(!doc.can_undo());
}

#[test]
fn clearing_a_page_is_one_undoable_edit() {
    let mut doc = Document::blank(a4());
    doc.add_stroke(support::line(support::pen(2.0), 0.0, 10.0, 0.0, 1.0));
    doc.add_stroke(support::line(support::pen(2.0), 0.0, 20.0, 5.0, 1.0));
    assert_eq!(doc.total_strokes(), 2);

    assert!(doc.clear_page());
    assert_eq!(doc.page().strokes.len(), 0);
    assert_eq!(doc.page().background, None, "the page itself stays");

    assert!(doc.undo());
    assert_eq!(doc.page().strokes.len(), 2, "one undo brings both strokes back");

    // Clearing an empty page changes nothing and does not touch history.
    let mut empty = Document::blank(a4());
    assert!(!empty.clear_page());
    assert!(!empty.can_undo());
}

#[test]
fn removing_strokes_removes_exactly_those_and_is_undoable() {
    let mut doc = Document::blank(a4());
    for y in [10.0, 30.0, 50.0] {
        doc.add_stroke(support::line(support::pen(2.0), 0.0, 40.0, y, 1.0));
    }

    assert!(doc.remove_strokes(&[0, 2]));
    assert_eq!(doc.page().strokes.len(), 1);
    assert_eq!(doc.page().strokes[0].points()[0].pos.y, 30.0);

    assert!(doc.undo());
    assert_eq!(doc.page().strokes.len(), 3);
    assert_eq!(doc.page().strokes[2].points()[0].pos.y, 50.0, "order is restored");

    assert!(!doc.remove_strokes(&[]), "nothing to remove is not an edit");
}

#[test]
fn the_eraser_hits_strokes_the_nib_covers() {
    let mut doc = Document::blank(a4());
    doc.add_stroke(support::line(support::pen(2.0), 10.0, 110.0, 50.0, 1.0));
    doc.add_stroke(support::line(support::pen(2.0), 10.0, 110.0, 200.0, 1.0));

    // On the line: hits stroke 0 only.
    assert_eq!(doc.hits(Pt::new(60.0, 50.0), 3.0, Tool::Eraser), vec![0]);
    // Just outside the nib (2 pt wide + 3 pt radius = 4 pt of reach).
    assert!(doc.hits(Pt::new(60.0, 54.5), 3.0, Tool::Eraser).is_empty());
    // Inside the reach, above the line.
    assert_eq!(doc.hits(Pt::new(60.0, 53.0), 3.0, Tool::Eraser), vec![0]);
    // Before the start of the line.
    assert!(doc.hits(Pt::new(0.0, 50.0), 3.0, Tool::Eraser).is_empty());
    // A pen never erases.
    assert!(doc.hits(Pt::new(60.0, 50.0), 3.0, Tool::Pen).is_empty());
}

#[test]
fn page_navigation_is_bounds_checked() {
    let mut doc = Document::from_backgrounds(&[a4(), a4(), a4()]);

    assert!(doc.go_to(2));
    assert_eq!(doc.current_index(), 2);
    assert!(!doc.go_to(3), "out of range is refused, not clamped silently");
    assert_eq!(doc.current_index(), 2);

    assert!(doc.go_to(0));
    assert!(!doc.previous_page(), "already on the first page");
    assert!(doc.next_page());
    assert_eq!(doc.current_index(), 1);
}

#[test]
fn each_page_keeps_its_own_strokes() {
    let mut doc = Document::from_backgrounds(&[a4(), a4()]);
    doc.add_stroke(support::line(support::pen(2.0), 0.0, 10.0, 0.0, 1.0));
    doc.next_page();
    doc.add_stroke(support::line(support::pen(2.0), 0.0, 10.0, 0.0, 1.0));
    doc.add_stroke(support::line(support::pen(2.0), 0.0, 10.0, 0.0, 1.0));

    assert_eq!(doc.pages()[0].strokes.len(), 1);
    assert_eq!(doc.pages()[1].strokes.len(), 2);
    assert_eq!(doc.total_strokes(), 3);

    doc.previous_page();
    doc.clear_page();
    assert_eq!(doc.total_strokes(), 2, "only the current page is cleared");
}

#[test]
fn a_page_snapshot_can_cross_a_thread_boundary() {
    // The rasterizer runs on another thread, so what it receives must be
    // `Send`.  `Stroke` and `Page` are plain values — this test stops compiling
    // if that ever stops being true.
    fn assert_send<T: Send>() {}
    assert_send::<Page>();
    assert_send::<Stroke>();

    let doc = Document::blank(a4());
    let snapshot: Page = doc.page().clone();
    assert_eq!(snapshot.size, a4());
}

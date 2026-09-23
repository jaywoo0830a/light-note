//! PDF contract — the pdfium side of the app.
//!
//! `pdfium-render` binds to a **runtime** library, so these tests skip (loudly)
//! when `pdfium.dll` is missing instead of failing: a developer without the
//! library can still run the whole core suite.  With the library present, the
//! tests check the three things the app depends on:
//!
//! * a page can be measured in points;
//! * a page can be rasterized into the app's own bitmap type (white paper, so
//!   the screen and the export agree);
//! * ink can be written **into** the PDF as vector paths and survives a save.

mod support;

use light_note::geom::{Scale, Size};
use light_note::pdf::{PageInk, PdfEngine, bind_pdfium};
use light_note::shape::{ink_coverage, pixel_at};

/// Binds pdfium, or prints why the tests are skipped.
fn engine() -> Option<PdfEngine> {
    match bind_pdfium() {
        Ok(engine) => Some(engine),
        Err(error) => {
            println!("pdfium is not available, skipping the PDF tests: {error}");
            println!("fetch it with:  powershell -File run-windows.ps1 -FetchPdfium");
            None
        }
    }
}

/// A one-page A4 PDF built by pdfium itself — no fixture file to keep in sync.
fn fixture() -> Vec<u8> {
    use pdfium_render::prelude::*;

    let pdfium = Pdfium::default();
    let mut document = pdfium.create_new_pdf().expect("a new document");
    document
        .pages_mut()
        .create_page_at_end(PdfPagePaperSize::a4())
        .expect("an A4 page");
    document.save_to_bytes().expect("PDF bytes")
}

fn a4() -> Size {
    support::A4
}

#[test]
fn an_a4_page_measures_a4_in_points() {
    let Some(engine) = engine() else { return };

    let sizes = engine.page_sizes(&fixture()).expect("page sizes");
    assert_eq!(sizes.len(), 1);
    assert!((sizes[0].width - a4().width).abs() < 0.5, "{:?}", sizes[0]);
    assert!((sizes[0].height - a4().height).abs() < 0.5, "{:?}", sizes[0]);
}

#[test]
fn a_blank_page_renders_as_white_paper_at_the_requested_scale() {
    let Some(engine) = engine() else { return };

    let pixmap = engine
        .render_page(&fixture(), 0, Scale::DEFAULT)
        .expect("rendered page");

    // 595.276 * 1.5 = 893 px, 841.89 * 1.5 = 1263 px (one pixel of slack).
    assert!((pixmap.width() as i32 - 893).abs() <= 1, "width {}", pixmap.width());
    assert!((pixmap.height() as i32 - 1263).abs() <= 1, "height {}", pixmap.height());

    let corner = pixel_at(&pixmap, 5, 5);
    assert_eq!(corner.a, 255, "the page is opaque: the background is white paper");
    assert!(corner.r > 240 && corner.g > 240 && corner.b > 240, "{corner:?}");
    assert_eq!(ink_coverage(&pixmap), 0, "an empty page has no ink");
}

#[test]
fn a_page_index_outside_the_document_is_an_error() {
    let Some(engine) = engine() else { return };

    let bytes = fixture();
    assert!(engine.render_page(&bytes, 3, Scale::DEFAULT).is_err());
    assert!(engine.page_sizes(&bytes).expect("sizes").len() == 1);
}

#[test]
fn broken_bytes_are_reported_not_panicked_on() {
    let Some(engine) = engine() else { return };

    let error = engine.page_sizes(b"this is not a PDF").expect_err("must fail");
    assert!(
        matches!(error, light_note::pdf::PdfError::Pdfium(_) | light_note::pdf::PdfError::NoPages),
        "{error:?}"
    );
}

#[test]
fn ink_is_written_into_the_pdf_as_vector_paths() {
    let Some(engine) = engine() else { return };

    let bytes = fixture();
    let page = light_note::doc::Page::with_background(a4(), 0);
    let strokes = vec![support::line(support::pen(4.0), 100.0, 400.0, 300.0, 1.0)];

    let annotated = engine
        .annotate(
            &bytes,
            &[PageInk {
                index: 0,
                strokes: strokes.clone(),
            }],
        )
        .expect("annotated PDF");

    assert_ne!(annotated, bytes, "the output is a new document");
    assert!(annotated.len() > bytes.len(), "the ink added content");

    // The original bytes are untouched and still load: light-note never edits
    // the file it was given.
    assert_eq!(engine.page_sizes(&bytes).expect("original").len(), 1);
    assert_eq!(page.background, Some(0));

    // The saved copy still measures A4 and now has ink where the stroke is.
    let sizes = engine.page_sizes(&annotated).expect("annotated sizes");
    assert!((sizes[0].width - a4().width).abs() < 0.5, "{:?}", sizes[0]);

    let pixmap = engine
        .render_page(&annotated, 0, Scale::DEFAULT)
        .expect("rendered annotated page");
    assert!(ink_coverage(&pixmap) > 300, "the stroke is visible");

    // 100 pt .. 400 pt at y = 300 pt, at 1.5 px/pt.
    let on_the_line = pixel_at(&pixmap, 375, 450);
    assert!(on_the_line.r < 120, "ink at the middle of the stroke: {on_the_line:?}");
    let far_away = pixel_at(&pixmap, 375, 100);
    assert!(far_away.r > 240, "the rest of the page is still white: {far_away:?}");
}

#[test]
fn annotating_several_pages_keeps_every_page() {
    let Some(engine) = engine() else { return };

    use pdfium_render::prelude::*;
    // The library binding must outlive the document it created, so it is a local
    // (a temporary `Pdfium` would be dropped at the end of the statement).
    let pdfium = Pdfium::default();
    let mut document = pdfium.create_new_pdf().expect("new document");
    for _ in 0..3 {
        document
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::a4())
            .expect("page");
    }
    let bytes = document.save_to_bytes().expect("bytes");

    let annotated = engine
        .annotate(
            &bytes,
            &[
                PageInk {
                    index: 0,
                    strokes: vec![support::line(support::pen(2.0), 50.0, 150.0, 50.0, 1.0)],
                },
                PageInk {
                    index: 2,
                    strokes: vec![support::line(support::pen(2.0), 50.0, 150.0, 700.0, 1.0)],
                },
            ],
        )
        .expect("annotated");

    let sizes = engine.page_sizes(&annotated).expect("sizes");
    assert_eq!(sizes.len(), 3, "no page is dropped");

    let first = engine.render_page(&annotated, 0, Scale::DEFAULT).expect("page 1");
    let middle = engine.render_page(&annotated, 1, Scale::DEFAULT).expect("page 2");
    let last = engine.render_page(&annotated, 2, Scale::DEFAULT).expect("page 3");

    assert!(ink_coverage(&first) > 100, "page 1 has ink");
    assert_eq!(ink_coverage(&middle), 0, "page 2 was not touched");
    assert!(ink_coverage(&last) > 100, "page 3 has ink");
}

#[test]
fn translucent_ink_is_composited_onto_white_for_export() {
    // PDF path objects carry no alpha channel, so a highlighter is exported as
    // the color it *looks like* on white paper — what a reader sees is what the
    // app showed.
    let Some(engine) = engine() else { return };

    let strokes = vec![support::line(support::highlighter(14.0), 100.0, 400.0, 300.0, 1.0)];
    let annotated = engine
        .annotate(&fixture(), &[PageInk { index: 0, strokes }])
        .expect("annotated");

    let pixmap = engine
        .render_page(&annotated, 0, Scale::DEFAULT)
        .expect("rendered");
    let pixel = pixel_at(&pixmap, 375, 450);

    let style = light_note::ink::Style::new(light_note::ink::Tool::Highlighter, 14.0);
    assert!(pixel.a == 255, "opaque on the page: {pixel:?}");
    // The marker is yellow (`rgba(255, 214, 64, 90)`), so its red channel is
    // already saturated: the *blue* channel is the one that proves the ink was
    // composited onto the paper instead of written with its own alpha.
    assert!(pixel.b > style.color.b, "lighter than the raw color: {pixel:?}");
    assert!(pixel.b < 255, "but it is still visible: {pixel:?}");
}


//! 내보내기 계약 — **내보낸 파일을 우리가 다시 읽는다**(왕복이 곧 검증이다).
//!
//! 잉크는 PDF에서 벡터로, PNG에서는 픽셀로 나간다. 여기서 확인하는 것은 두 가지다:
//! 페이지 수/크기가 살아 있는가, 그리고 배경(PDF) 위에 잉크가 올라가는가.

mod common;

use light_note_gui::doc::Doc;
use light_note_gui::export::{
    document_to_pdf, export_document_png, export_pdf, page_to_png, ExportError, EXPORT_SCALE,
};
use light_note_gui::geom::{Pt, Scale, Size};
use light_note_gui::ink::Tool;
use light_note_gui::pdf::PdfDocument;
use light_note_gui::shape;

/// Letter 크기(612×792pt) — A4와 다른 크기가 필요할 때.
const LETTER: Size = Size::new(612.0, 792.0);

/// 2페이지 문서(A4 + Letter), 첫 페이지에 잉크 하나.
fn document() -> Doc {
    let mut doc = Doc::from_pdf([Size::A4, LETTER], "테스트.pdf");
    let stroke = common::line_stroke(
        Tool::Pen,
        Pt::new(20.0, 40.0),
        Pt::new(180.0, 40.0),
        16,
        4.0,
    );
    doc.commit_stroke(0, stroke);
    doc
}

#[test]
fn pdf_round_trip_keeps_pages_and_sizes() {
    let doc = document();
    let bytes = document_to_pdf(&doc, None).expect("PDF 쓰기");
    assert_eq!(&bytes[..5], b"%PDF-", "PDF 헤더");

    // 우리가 쓴 PDF를 다시 읽는다 — 읽기(hayro)와 쓰기(pdf-writer)가 같은 모델을 쓴다.
    let read = PdfDocument::from_bytes(bytes).expect("PDF 읽기");
    assert_eq!(read.page_count(), 2);
    assert_eq!(read.page_size(0), Some(Size::A4));
    assert_eq!(read.page_size(1), Some(LETTER));
}

#[test]
fn pdf_export_composes_the_background_pages() {
    // 배경 PDF(1장)에 맞춘 문서 — 페이지 크기와 배경 인덱스가 배경 쪽을 가리킨다.
    let background = PdfDocument::from_bytes(common::sample_pdf()).expect("샘플 PDF");
    let doc = Doc::from_pdf([common::sample_pdf_size()], "테스트.pdf");

    let bytes = document_to_pdf(&doc, Some(&background)).expect("PDF 쓰기");
    let read = PdfDocument::from_bytes(bytes).expect("PDF 읽기");
    assert_eq!(read.page_count(), 1);
    assert_eq!(read.page_size(0), Some(common::sample_pdf_size()));
}

#[test]
fn a_missing_background_page_fails_loudly() {
    // 배경에 없는 페이지를 가리키면 **조용히 흰 종이로 넘어가지 않는다** — 이유를 말하고 멈춘다.
    let background = PdfDocument::from_bytes(common::sample_pdf()).expect("샘플 PDF");
    let doc = Doc::from_pdf([common::sample_pdf_size(), Size::A4], "테스트.pdf");

    let error = document_to_pdf(&doc, Some(&background)).expect_err("배경 페이지가 없다");
    assert!(matches!(error, ExportError::Background(_)), "{error:?}");
}

#[test]
fn png_export_is_white_paper_with_ink() {
    let doc = document();
    let page = doc.page(0).expect("0페이지");
    let bytes = page_to_png(page, None, EXPORT_SCALE).expect("PNG 쓰기");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG 시그니처");

    let pixmap = shape::from_png(&bytes).expect("PNG 읽기");
    assert_eq!(
        (pixmap.width(), pixmap.height()),
        Scale::new(EXPORT_SCALE).pixels(Size::A4)
    );

    let corner = shape::pixel_at(&pixmap, 0, 0);
    assert_eq!(
        (corner.r, corner.g, corner.b, corner.a),
        (255, 255, 255, 255),
        "빈 종이는 흰색 불투명이다"
    );

    // 잉크는 y=40pt, x=20..180pt — 픽셀로는 배율만큼 곱한 자리.
    let inked = shape::pixel_at(&pixmap, 100, 80);
    assert!(inked.a > 200 && inked.b > 100, "파란 잉크: {inked:?}");
}

#[test]
fn png_export_composites_the_pdf_background_under_the_ink() {
    let background = PdfDocument::from_bytes(common::sample_pdf()).expect("샘플 PDF");
    let raster = background.render_page(0, EXPORT_SCALE).expect("배경 렌더");
    assert_eq!(
        (raster.width(), raster.height()),
        Scale::new(EXPORT_SCALE).pixels(common::sample_pdf_size())
    );

    let mut doc = Doc::blank(common::sample_pdf_size());
    let stroke = common::line_stroke(
        Tool::Pen,
        Pt::new(20.0, 50.0),
        Pt::new(180.0, 50.0),
        16,
        4.0,
    );
    doc.commit_stroke(0, stroke);

    let bytes =
        page_to_png(doc.page(0).expect("0페이지"), Some(&raster), EXPORT_SCALE).expect("PNG 쓰기");
    let pixmap = shape::from_png(&bytes).expect("PNG 읽기");

    let left = shape::pixel_at(&pixmap, 40, 20); // 빨간 사각형(배경)
    assert!(left.r > 200 && left.g < 80, "왼쪽은 배경 그대로: {left:?}");
    let right = shape::pixel_at(&pixmap, 380, 20); // 흰 종이
    assert!(
        right.r > 230 && right.g > 230,
        "오른쪽은 흰 종이: {right:?}"
    );

    let inked = shape::pixel_at(&pixmap, 100, 100); // 잉크가 지나가는 자리(px)
    assert!(
        inked.a > 200 && inked.b > 100,
        "잉크가 배경 위에 있다: {inked:?}"
    );
}

#[test]
fn files_are_written_with_the_documented_names() {
    let directory = std::env::temp_dir().join(format!("light-note-export-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("임시 폴더");

    let doc = document();
    let written = export_document_png(&doc, None, &directory, "쪽", 1.0).expect("PNG 저장");
    assert_eq!(written.len(), 2, "페이지마다 한 장");
    assert_eq!(written[0].file_name().unwrap(), "쪽-1.png");
    assert_eq!(written[1].file_name().unwrap(), "쪽-2.png");
    for path in &written {
        let bytes = std::fs::read(path).expect("저장된 파일");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    let pdf_path = export_pdf(&doc, None, directory.join("쪽.pdf")).expect("PDF 저장");
    let bytes = std::fs::read(&pdf_path).expect("저장된 파일");
    assert_eq!(&bytes[..5], b"%PDF-");

    std::fs::remove_dir_all(&directory).expect("임시 폴더 정리");
}

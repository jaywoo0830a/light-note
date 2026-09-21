//! 내보내기 계약 — PNG/PDF를 만들고 **hayro로 다시 읽어** 픽셀까지 검증한다.
//!
//! 왕복이 곧 계약이다: 내보낸 PDF는 다른 뷰어(그리고 우리 자신)에서 원본 배경 +
//! 필기가 같은 위치에 보여야 한다.

mod common;

use light_note_core::doc::Document;
use light_note_core::export;
use light_note_core::geom::{Point, Size};
use light_note_core::pdf::PdfDocument;
use light_note_core::raster::{from_png, pixel_at};

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("light-note-tests");
    std::fs::create_dir_all(&dir).expect("임시 폴더");
    dir
}

#[test]
fn png_export_writes_a_file_with_the_page_pixels() {
    let mut document = Document::blank(Size::new(200.0, 100.0));
    common::draw_line(
        &mut document,
        Point::new(20.0, 50.0),
        Point::new(180.0, 50.0),
        6.0,
    );

    let path = temp_dir().join("page.png");
    let written =
        export::export_page_png(document.active_page(), None, &path, 1.0).expect("PNG 내보내기");
    assert_eq!(written, path);

    let bytes = std::fs::read(&path).expect("파일 읽기");
    let pixmap = from_png(&bytes).expect("PNG 디코딩");
    assert_eq!((pixmap.width(), pixmap.height()), (200, 100));
    assert!(pixel_at(&pixmap, 100, 50).a > 200, "잉크가 저장됐다");
    assert!(pixel_at(&pixmap, 100, 10).r > 230, "빈 곳은 흰 종이");

    std::fs::remove_file(&path).ok();
}

#[test]
fn png_export_of_a_pdf_page_keeps_the_background() {
    let source = PdfDocument::from_bytes(common::sample_pdf()).expect("샘플 PDF");
    let mut document = Document::from_pdf_pages([common::sample_pdf_size()], "샘플");
    common::draw_line(
        &mut document,
        Point::new(120.0, 50.0),
        Point::new(190.0, 50.0),
        4.0,
    );

    let background = source.render_page(0, 1.0).expect("배경 렌더");
    let bytes = export::page_to_png(document.active_page(), Some(&background), 1.0)
        .expect("PNG 만들기");
    let pixmap = from_png(&bytes).expect("PNG 디코딩");

    let red = pixel_at(&pixmap, 30, 50);
    assert!(red.r > 200 && red.g < 90, "배경이 살아 있다: {red:?}");
    let ink = pixel_at(&pixmap, 155, 50);
    assert!(ink.b > ink.r, "잉크가 위에 있다: {ink:?}");
}

#[test]
fn pdf_export_round_trips_through_hayro() {
    let mut document = Document::blank(Size::new(200.0, 100.0));
    document.set_title("필기 노트");
    common::draw_line(
        &mut document,
        Point::new(20.0, 50.0),
        Point::new(180.0, 50.0),
        6.0,
    );

    let bytes = export::document_to_pdf(&document, None).expect("PDF 만들기");
    assert_eq!(&bytes[..5], b"%PDF-", "PDF 헤더");

    let reopened = PdfDocument::from_bytes(bytes).expect("hayro로 다시 열기");
    assert_eq!(reopened.page_count(), 1);
    let size = reopened.page_size(0).expect("페이지 크기");
    assert!(
        (size.width - 200.0).abs() < 1.0 && (size.height - 100.0).abs() < 1.0,
        "페이지 크기가 보존된다: {size:?}"
    );

    let pixmap = reopened.render_page(0, 1.0).expect("렌더");
    let ink = pixel_at(&pixmap, 100, 50);
    assert!(ink.b > ink.r, "필기가 같은 자리에 다시 보인다: {ink:?}");
    let empty = pixel_at(&pixmap, 100, 10);
    assert!(empty.r > 230 && empty.g > 230, "빈 곳은 흰색: {empty:?}");
}

#[test]
fn pdf_export_embeds_the_background_page_as_an_image() {
    let source = PdfDocument::from_bytes(common::sample_pdf()).expect("샘플 PDF");
    let mut document = Document::from_pdf_pages([common::sample_pdf_size()], "샘플");
    common::draw_line(
        &mut document,
        Point::new(120.0, 50.0),
        Point::new(190.0, 50.0),
        4.0,
    );

    let bytes = export::document_to_pdf(&document, Some(&source)).expect("PDF 만들기");
    let reopened = PdfDocument::from_bytes(bytes).expect("다시 열기");
    let pixmap = reopened.render_page(0, 1.0).expect("렌더");

    let red = pixel_at(&pixmap, 30, 50);
    assert!(
        red.r > 180 && red.g < 110,
        "원본 배경이 이미지 XObject로 다시 들어갔다: {red:?}"
    );
    let ink = pixel_at(&pixmap, 155, 50);
    assert!(ink.b > ink.r, "잉크가 배경 위에 얹혔다: {ink:?}");
}

#[test]
fn export_document_png_writes_one_file_per_page() {
    let mut document = Document::blank(Size::new(100.0, 50.0));
    common::draw_line(
        &mut document,
        Point::new(10.0, 25.0),
        Point::new(90.0, 25.0),
        3.0,
    );
    document.add_page_after_active(Size::new(100.0, 50.0));

    let dir = temp_dir().join("pages");
    std::fs::create_dir_all(&dir).expect("폴더");
    let written = export::export_document_png(&document, None, &dir, "note", 1.0).expect("내보내기");

    assert_eq!(written.len(), 2);
    assert!(written[0].ends_with("note-1.png"));
    assert!(written[1].ends_with("note-2.png"));
    for path in &written {
        let bytes = std::fs::read(path).expect("파일");
        assert!(from_png(&bytes).is_ok());
        std::fs::remove_file(path).ok();
    }
}

#[test]
fn exporting_a_pdf_file_writes_bytes_that_hayro_can_open() {
    let mut document = Document::blank(Size::new(120.0, 80.0));
    common::draw_line(
        &mut document,
        Point::new(10.0, 40.0),
        Point::new(110.0, 40.0),
        3.0,
    );

    let path = temp_dir().join("note.pdf");
    let written = export::export_pdf(&document, None, &path).expect("PDF 저장");
    assert_eq!(written, path);

    let bytes = std::fs::read(&path).expect("파일");
    let reopened = PdfDocument::from_bytes(bytes).expect("다시 열기");
    assert_eq!(reopened.page_count(), 1);
    assert_eq!(reopened.file_name(), "메모리 PDF", "바이트로 열면 이름이 없다");

    std::fs::remove_file(&path).ok();
}

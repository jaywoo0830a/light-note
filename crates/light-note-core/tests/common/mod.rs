//! 테스트 공용 도구 — 샘플 PDF와 스트로크 만들기.
//!
//! `tests/common/mod.rs`는 하위 디렉터리라 별도 테스트 타깃이 되지 않는다
//! (Cargo 규칙) — 각 테스트가 `mod common;`으로 가져다 쓴다.
//!
//! 모든 테스트가 모든 헬퍼를 쓰지는 않으므로 `dead_code`는 허용한다.

#![allow(dead_code)]

use light_note_core::doc::Document;
use light_note_core::geom::{Point, Size};
use light_note_core::ink::{InkPoint, Rgba, Stroke, StrokeStyle, Tool};

/// 테스트용 PDF: 200×100pt 한 장, **왼쪽 절반이 빨간 사각형**.
///
/// hayro로 읽고 래스터화하면 왼쪽은 빨강, 오른쪽은 흰색이어야 한다 —
/// "PDF를 실제로 읽었다"를 픽셀로 확인할 수 있는 최소 문서다.
pub fn sample_pdf() -> Vec<u8> {
    use pdf_writer::types::LineCapStyle;
    use pdf_writer::{Content, Finish, Pdf, Rect, Ref};

    let mut pdf = Pdf::new();
    let catalog_id = Ref::new(1);
    let pages_id = Ref::new(2);
    let page_id = Ref::new(3);
    let content_id = Ref::new(4);

    pdf.catalog(catalog_id).pages(pages_id);
    pdf.pages(pages_id).kids([page_id]).count(1);

    let mut page = pdf.page(page_id);
    page.parent(pages_id);
    page.media_box(Rect::new(0.0, 0.0, 200.0, 100.0));
    page.contents(content_id);
    page.finish();

    let mut content = Content::new();
    content.set_line_cap(LineCapStyle::RoundCap);
    content.set_fill_rgb(1.0, 0.0, 0.0);
    content.rect(0.0, 0.0, 100.0, 100.0);
    content.fill_nonzero();
    pdf.stream(content_id, &content.finish());

    pdf.finish()
}

/// 샘플 PDF의 페이지 크기(pt).
pub fn sample_pdf_size() -> Size {
    Size::new(200.0, 100.0)
}

/// 직선 스트로크 — `(x1,y1)`에서 `(x2,y2)`까지 `steps`개의 샘플.
pub fn line_stroke(tool: Tool, from: Point, to: Point, steps: usize, width: f32) -> Stroke {
    let style = match tool {
        Tool::Highlighter => StrokeStyle::highlighter(Rgba::HIGHLIGHT_YELLOW, width),
        Tool::Eraser => StrokeStyle::pen(Rgba::RED, width),
        Tool::Pen => StrokeStyle::pen(Rgba::INK_BLUE, width),
    };
    let mut stroke = Stroke::new(tool, style, InkPoint::at(from));
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        stroke.push(InkPoint::at(from.lerp(to, t)));
    }
    stroke
}

/// 문서에 직선 하나를 그린다(드래그 = 편집 하나).
pub fn draw_line(document: &mut Document, from: Point, to: Point, width: f32) {
    let stroke = line_stroke(Tool::Pen, from, to, 16, width);
    let mut points = stroke.points().iter().copied();
    let first = points.next().expect("첫 샘플");
    document.begin(Tool::Pen, stroke.style, first);
    for point in points {
        document.extend(point);
    }
    document.finish();
}

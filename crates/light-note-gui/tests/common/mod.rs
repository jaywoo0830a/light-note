//! 테스트 공용 도구 — 샘플 PDF와 획 만들기.
//!
//! `tests/common/mod.rs`는 하위 디렉터리라 별도 테스트 타깃이 되지 않는다(Cargo 규칙) —
//! 각 테스트가 `mod common;`으로 가져다 쓴다. 모든 테스트가 모든 헬퍼를 쓰지는 않으므로
//! `dead_code`는 허용한다.

#![allow(dead_code)]

use light_note_gui::geom::{Pt, Size};
use light_note_gui::ink::{InkPoint, Stroke, Style, Tool};

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

/// 점 목록 `(x, y, 압력)`으로 획 하나를 만든다 — 모델에 **직접** 넣는 재료.
///
/// 파이프라인 ②를 거치지 않는 이유: 여기서 검사하는 것은 기하이지 제스처가 아니다.
pub fn stroke_of(tool: Tool, width_pt: f32, samples: &[(f32, f32, f32)]) -> Stroke {
    let first = samples.first().expect("점이 하나는 있어야 한다");
    // 색은 **도구가 정한다**(형광펜은 반투명) — 굵기만 테스트가 고른다.
    let mut style = Style::for_tool(tool);
    style.width_pt = width_pt.clamp(Style::MIN_WIDTH_PT, Style::MAX_WIDTH_PT);
    let mut stroke = Stroke::new(
        tool,
        style,
        InkPoint::new(Pt::new(first.0, first.1), first.2),
    );
    for (x, y, pressure) in &samples[1..] {
        stroke.push(InkPoint::new(Pt::new(*x, *y), *pressure));
    }
    stroke
}

/// 일정 압력의 직선 획 — 폭이 변하지 않으므로 도형은 **곡선 하나**(`InkShape::Curve`)다.
pub fn line_stroke(tool: Tool, from: Pt, to: Pt, steps: usize, width_pt: f32) -> Stroke {
    let samples: Vec<(f32, f32, f32)> = (0..=steps)
        .map(|step| {
            let t = step as f32 / steps as f32;
            (
                from.x + (to.x - from.x) * t,
                from.y + (to.y - from.y) * t,
                1.0,
            )
        })
        .collect();
    stroke_of(tool, width_pt, &samples)
}

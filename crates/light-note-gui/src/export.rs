//! 내보내기 — PNG(래스터)와 PDF(벡터 잉크 + 배경 이미지).
//!
//! ## 왜 `pdf-writer`인가
//! hayro의 PDF 쓰기 크레이트(`hayro-write`)가 쓰는 writer가 `pdf-writer`다. 그래서
//! **읽기(hayro)·쓰기(pdf-writer)** 가 같은 PDF 모델을 공유한다 — 내보낸 PDF를 곧바로
//! hayro로 다시 읽어 검증할 수 있다(테스트가 그렇게 한다).
//!
//! ## 무엇을 쓰나
//! - 잉크는 **벡터 경로**로 쓴다 — 확대해도 계단이 없다.
//! - 필압으로 굵기가 변하면 구간별 폭으로 나눠 **한 번에 채운다**(래스터·라이브와 같은 규칙).
//! - 형광펜 반투명은 `ExtGState`(`/CA`·`/ca`)로 표현한다.
//! - PDF 배경 페이지는 **이미지 XObject**로 다시 넣는다(원본은 수정하지 않는다).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hayro::vello_cpu::Pixmap;
use pdf_writer::types::{LineCapStyle, LineJoinStyle};
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref};

use crate::doc::{Doc, Page};
use crate::geom::Scale;
use crate::pdf::PdfDocument;
use crate::shape;

/// 내보내기 기본 해상도 배율 — 1pt당 2px = 144dpi (화면보다 선명한 종이 결과).
pub const EXPORT_SCALE: f32 = 2.0;

/// 이미지 XObject의 리소스 이름.
const IMAGE_NAME: Name<'static> = Name(b"Im0");

/// 내보내기가 실패한 이유.
#[derive(Clone, Debug, PartialEq)]
pub enum ExportError {
    /// 파일을 쓰지 못했다.
    Io(String),
    /// 래스터화/인코딩 실패.
    Raster(String),
    /// 배경 PDF 렌더 실패.
    Background(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Io(message) => write!(formatter, "Could not write the file: {message}"),
            ExportError::Raster(message) => {
                write!(formatter, "Could not build the image: {message}")
            }
            ExportError::Background(message) => {
                write!(formatter, "Could not draw the PDF background: {message}")
            }
        }
    }
}

impl std::error::Error for ExportError {}

/// 페이지 하나를 PNG 바이트로 만든다(테스트가 그대로 쓴다).
pub fn page_to_png(
    page: &Page,
    background: Option<&Pixmap>,
    scale: f32,
) -> Result<Vec<u8>, ExportError> {
    let pixmap = shape::render_on_white(page.strokes(), page.size(), background, Scale::new(scale));
    shape::to_png(pixmap).map_err(ExportError::Raster)
}

/// 페이지 하나를 PNG 파일로 저장한다.
pub fn export_page_png(
    page: &Page,
    background: Option<&Pixmap>,
    path: impl AsRef<Path>,
    scale: f32,
) -> Result<PathBuf, ExportError> {
    let path = path.as_ref();
    let bytes = page_to_png(page, background, scale)?;
    std::fs::write(path, bytes).map_err(|error| ExportError::Io(error.to_string()))?;
    Ok(path.to_path_buf())
}

/// 문서 전체를 PNG 파일들로 저장한다 (`<stem>-1.png`, `<stem>-2.png` …).
pub fn export_document_png(
    doc: &Doc,
    pdf: Option<&PdfDocument>,
    directory: impl AsRef<Path>,
    stem: &str,
    scale: f32,
) -> Result<Vec<PathBuf>, ExportError> {
    let directory = directory.as_ref();
    let mut written = Vec::with_capacity(doc.page_count());
    for (index, page) in doc.pages().iter().enumerate() {
        let background = pdf.and_then(|pdf| pdf.render_optional(page.background(), scale));
        let path = directory.join(format!("{stem}-{}.png", index + 1));
        written.push(export_page_png(page, background.as_ref(), path, scale)?);
    }
    Ok(written)
}

/// 문서 전체를 PDF 바이트로 만든다 — 잉크는 벡터, 배경은 이미지 XObject.
///
/// 내보낸 바이트는 곧바로 `hayro`로 다시 읽을 수 있다(테스트가 왕복을 검증한다).
pub fn document_to_pdf(doc: &Doc, pdf: Option<&PdfDocument>) -> Result<Vec<u8>, ExportError> {
    let catalog_id = Ref::new(1);
    let pages_id = Ref::new(2);
    let mut next = 3;

    // ① 반투명 획(형광펜)용 ExtGState — 알파별로 하나.
    let alphas: Vec<u8> = doc
        .pages()
        .iter()
        .flat_map(|page| page.strokes().iter().map(|stroke| stroke.style.color.a))
        .filter(|alpha| *alpha < 255)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let names: Vec<String> = (0..alphas.len()).map(|slot| format!("GS{slot}")).collect();

    let mut gstate_refs = Vec::new();
    for (slot, alpha) in alphas.iter().enumerate() {
        let id = Ref::new(next);
        next += 1;
        gstate_refs.push((*alpha, id, slot));
    }

    // ② 참조 배정 — 페이지/내용/배경 이미지.
    struct PageRefs {
        index: usize,
        page_id: Ref,
        content_id: Ref,
        image_id: Option<Ref>,
    }
    let mut pages = Vec::new();
    for (index, page) in doc.pages().iter().enumerate() {
        let page_id = Ref::new(next);
        next += 1;
        let content_id = Ref::new(next);
        next += 1;
        let image_id = page.background().map(|_| {
            let id = Ref::new(next);
            next += 1;
            id
        });
        pages.push(PageRefs {
            index,
            page_id,
            content_id,
            image_id,
        });
    }

    let mut writer = Pdf::new();

    // ③ 배경 이미지 XObject — 페이지 래스터를 RGB8로 넣는다.
    for page_refs in &pages {
        let (Some(image_id), Some(source), Some(page)) =
            (page_refs.image_id, pdf, doc.page(page_refs.index))
        else {
            continue;
        };
        let Some(pdf_index) = page.background() else {
            continue;
        };
        let pixmap = source
            .render_page(pdf_index, EXPORT_SCALE)
            .map_err(|error| ExportError::Background(error.to_string()))?;
        let rgb = shape::flatten_to_rgb8(&pixmap);
        let mut image = writer.image_xobject(image_id, &rgb);
        image.width(pixmap.width() as i32);
        image.height(pixmap.height() as i32);
        image.color_space().device_rgb();
        image.bits_per_component(8);
        image.finish();
    }

    // ④ ExtGState 사전.
    for (alpha, id, _) in &gstate_refs {
        let opacity = *alpha as f32 / 255.0;
        writer
            .ext_graphics(*id)
            .stroking_alpha(opacity)
            .non_stroking_alpha(opacity)
            .finish();
    }

    // ⑤ 카탈로그 + 페이지 트리.
    writer.catalog(catalog_id).pages(pages_id);
    writer
        .pages(pages_id)
        .kids(pages.iter().map(|page| page.page_id))
        .count(pages.len() as i32);

    // ⑥ 페이지 사전 + 리소스.
    for page_refs in &pages {
        let Some(page) = doc.page(page_refs.index) else {
            continue;
        };
        let size = page.size();
        let mut page_writer = writer.page(page_refs.page_id);
        page_writer.parent(pages_id);
        page_writer.media_box(Rect::new(0.0, 0.0, size.width, size.height));
        page_writer.contents(page_refs.content_id);
        {
            let mut resources = page_writer.resources();
            if let Some(image_id) = page_refs.image_id {
                resources.x_objects().pair(IMAGE_NAME, image_id);
            }
            if !gstate_refs.is_empty() {
                let mut states = resources.ext_g_states();
                for (_, id, slot) in &gstate_refs {
                    states.pair(Name(names[*slot].as_bytes()), *id);
                }
            }
        }
        page_writer.finish();
    }

    // ⑦ 내용 스트림 — 배경 이미지 + 벡터 잉크.
    for page_refs in &pages {
        let Some(page) = doc.page(page_refs.index) else {
            continue;
        };
        let size = page.size();
        let mut content = Content::new();

        if page_refs.image_id.is_some() {
            content.save_state();
            content.transform([size.width, 0.0, 0.0, size.height, 0.0, 0.0]);
            content.x_object(IMAGE_NAME);
            content.restore_state();
        }

        content.save_state();
        content.set_line_cap(LineCapStyle::RoundCap);
        content.set_line_join(LineJoinStyle::RoundJoin);
        for stroke in page.strokes() {
            if stroke.tool.is_eraser() {
                continue;
            }
            let color = stroke.style.color;
            if color.a < 255 {
                if let Some((_, _, slot)) =
                    gstate_refs.iter().find(|(alpha, _, _)| *alpha == color.a)
                {
                    content.set_parameters(Name(names[*slot].as_bytes()));
                }
            }
            let (red, green, blue) = (
                color.r as f32 / 255.0,
                color.g as f32 / 255.0,
                color.b as f32 / 255.0,
            );
            content.set_stroke_rgb(red, green, blue);
            content.set_fill_rgb(red, green, blue);

            // 화면과 **같은 도형 결정**을 쓴다 — 배율만 1.0(pt = PDF 단위)이다.
            match shape::ink_shape(stroke, Scale::new(1.0)) {
                Some(shape::InkShape::Curve { width, path }) => {
                    emit_path(&mut content, &path, size.height);
                    content.set_line_width(width).stroke();
                }
                Some(shape::InkShape::Spans { spans }) => {
                    // 화면과 **같은 합집합**을 한 번에 채운다 — 관절에서 알파가 겹치지 않는다.
                    emit_path(&mut content, &shape::spans_union(&spans), size.height);
                    content.fill_nonzero();
                }
                Some(shape::InkShape::Dot { center, radius }) => {
                    let mut dot = hayro::vello_cpu::kurbo::BezPath::new();
                    shape::push_disc(&mut dot, center, radius);
                    emit_path(&mut content, &dot, size.height);
                    content.fill_nonzero(); // 점 하나 = 채운 원
                }
                None => {}
            }
        }
        content.restore_state();

        writer.stream(page_refs.content_id, &content.finish());
    }

    Ok(writer.finish())
}

/// 문서를 PDF 파일로 저장한다.
pub fn export_pdf(
    doc: &Doc,
    pdf: Option<&PdfDocument>,
    path: impl AsRef<Path>,
) -> Result<PathBuf, ExportError> {
    let path = path.as_ref();
    let bytes = document_to_pdf(doc, pdf)?;
    std::fs::write(path, bytes).map_err(|error| ExportError::Io(error.to_string()))?;
    Ok(path.to_path_buf())
}

/// 경로를 PDF 연산자로 옮긴다 — 모델 좌표(좌상단)를 PDF 좌표(좌하단)로 뒤집는다.
///
/// 2차 베지어는 **3차로 승격**한다(PDF에는 2차 곡선이 없고 `pdf-writer`에는 `curve_to`밖에
/// 없다). 화면·래스터와 **같은 도형 결정**을 쓰므로, 확대해서 비교해도 선이 일치한다.
fn emit_path(content: &mut Content, path: &hayro::vello_cpu::kurbo::BezPath, height: f32) {
    use hayro::vello_cpu::kurbo::PathEl;

    let flip = |y: f64| height - y as f32;
    let mut current = (0.0_f64, 0.0_f64);

    for element in path.iter() {
        match element {
            PathEl::MoveTo(point) => {
                content.move_to(point.x as f32, flip(point.y));
                current = (point.x, point.y);
            }
            PathEl::LineTo(point) => {
                content.line_to(point.x as f32, flip(point.y));
                current = (point.x, point.y);
            }
            PathEl::QuadTo(control, point) => {
                // 2차 → 3차 승격(제어점 2/3 규칙).
                let first = (
                    current.0 + 2.0 / 3.0 * (control.x - current.0),
                    current.1 + 2.0 / 3.0 * (control.y - current.1),
                );
                let second = (
                    point.x + 2.0 / 3.0 * (control.x - point.x),
                    point.y + 2.0 / 3.0 * (control.y - point.y),
                );
                content.cubic_to(
                    first.0 as f32,
                    flip(first.1),
                    second.0 as f32,
                    flip(second.1),
                    point.x as f32,
                    flip(point.y),
                );
                current = (point.x, point.y);
            }
            PathEl::CurveTo(first, second, point) => {
                content.cubic_to(
                    first.x as f32,
                    flip(first.y),
                    second.x as f32,
                    flip(second.y),
                    point.x as f32,
                    flip(point.y),
                );
                current = (point.x, point.y);
            }
            PathEl::ClosePath => {
                content.close_path();
            }
        }
    }
}

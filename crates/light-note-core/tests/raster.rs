//! 래스터라이저 계약 — 잉크가 **어디에** 올라갔는지 픽셀로 확인한다.
//!
//! WinUI 표면(정적 PNG)과 PNG/PDF 내보내기가 모두 이 결과를 쓴다.

mod common;

use light_note_core::doc::{Document, Page};
use light_note_core::geom::{Point, Size};
use light_note_core::ink::{InkPoint, Rgba, Stroke, StrokeStyle, Tool};
use light_note_core::raster::{
    self, ViewTransform, composite_into, ink_coverage, pixel_at, render_ink, render_page_on_white,
};
use light_note_core::surface::InkSurface;

fn page_with_strokes(strokes: Vec<Stroke>, size: Size) -> Page {
    let mut page = Page::blank(size);
    for stroke in strokes {
        page.push_stroke(stroke);
    }
    page
}

/// 주어진 x 열에서 세로로 잉크가 있는 픽셀 수(굵기 측정).
fn vertical_ink_height(pixmap: &hayro::vello_cpu::Pixmap, x: u16) -> usize {
    let height = pixmap.height();
    (0..height)
        .filter(|y| pixel_at(pixmap, x, *y).a > 40)
        .count()
}

#[test]
fn view_transform_maps_points_and_sizes() {
    let transform = ViewTransform::new(2.0);
    assert_eq!(transform.pixels(Size::new(200.0, 100.0)), (400, 200));
    assert_eq!(transform.to_pixels(Point::new(10.0, 5.0)), (20.0, 10.0));
    let back = transform.to_page(20.0, 10.0);
    assert!((back.x - 10.0).abs() < 1e-4 && (back.y - 5.0).abs() < 1e-4);
    assert!((transform.len(3.0) - 6.0).abs() < 1e-4);
}

#[test]
fn ink_lands_where_the_model_says() {
    let size = Size::new(200.0, 100.0);
    let stroke = common::line_stroke(
        Tool::Pen,
        Point::new(20.0, 50.0),
        Point::new(180.0, 50.0),
        32,
        4.0,
    );
    let pixmap = render_ink(std::slice::from_ref(&stroke), size, 1.0);

    assert_eq!((pixmap.width(), pixmap.height()), (200, 100));
    let center = pixel_at(&pixmap, 100, 50);
    assert!(center.a > 200, "선 위에는 잉크가 있어야 한다: {center:?}");
    assert!(
        (center.r as i32 - Rgba::INK_BLUE.r as i32).abs() < 12
            && (center.g as i32 - Rgba::INK_BLUE.g as i32).abs() < 12,
        "색은 도구 색과 같아야 한다: {center:?}"
    );

    assert_eq!(pixel_at(&pixmap, 100, 10).a, 0, "선에서 먼 곳은 투명");
    assert_eq!(pixel_at(&pixmap, 5, 50).a, 0, "선분 밖은 투명");
    assert!(ink_coverage(&pixmap) > 400, "선 길이만큼 픽셀이 칠해진다");
}

#[test]
fn empty_pages_render_to_nothing() {
    let pixmap = render_ink(&[], Size::A4, 1.0);
    assert_eq!(ink_coverage(&pixmap), 0);
    assert!(InkSurface::build(Size::A4, 1.0, &[], None)
        .static_png
        .is_none());
}

#[test]
fn rendering_is_deterministic() {
    let size = Size::new(200.0, 100.0);
    let stroke = common::line_stroke(
        Tool::Pen,
        Point::new(10.0, 20.0),
        Point::new(150.0, 80.0),
        24,
        3.0,
    );
    let first = render_ink(std::slice::from_ref(&stroke), size, 1.5);
    let second = render_ink(std::slice::from_ref(&stroke), size, 1.5);
    assert_eq!(
        first.data_as_u8_slice(),
        second.data_as_u8_slice(),
        "같은 입력은 같은 픽셀을 내야 한다(스냅샷 계약)"
    );
}

#[test]
fn pressure_changes_the_stroke_thickness() {
    let size = Size::new(200.0, 100.0);
    let mut stroke = Stroke::new(
        Tool::Pen,
        StrokeStyle::pen(Rgba::BLACK, 20.0),
        InkPoint::with_pressure(20.0, 50.0, 1.0),
    );
    for step in 1..=20 {
        let x = 20.0 + step as f32 * 8.0;
        stroke.push(InkPoint::with_pressure(x, 50.0, 0.2));
    }
    let pixmap = render_ink(std::slice::from_ref(&stroke), size, 1.0);

    let thick = vertical_ink_height(&pixmap, 30);
    let thin = vertical_ink_height(&pixmap, 180);
    assert!(thick > thin, "필압이 높은 쪽이 더 두껍다: {thick} vs {thin}");
    assert!(thin > 0, "필압이 낮아도 잉크는 남는다");
}

#[test]
fn a_tap_draws_a_dot() {
    let size = Size::new(100.0, 100.0);
    let stroke = Stroke::new(
        Tool::Pen,
        StrokeStyle::pen(Rgba::BLACK, 6.0),
        InkPoint::new(50.0, 50.0),
    );
    let pixmap = render_ink(std::slice::from_ref(&stroke), size, 1.0);
    assert!(pixel_at(&pixmap, 50, 50).a > 200, "점이 찍혀야 한다");
    assert!(ink_coverage(&pixmap) > 10);
}

#[test]
fn highlighter_is_translucent() {
    let size = Size::new(200.0, 100.0);
    let stroke = common::line_stroke(
        Tool::Highlighter,
        Point::new(20.0, 50.0),
        Point::new(180.0, 50.0),
        16,
        StrokeStyle::HIGHLIGHTER_WIDTH_PT,
    );
    let pixmap = render_ink(std::slice::from_ref(&stroke), size, 1.0);
    let center = pixel_at(&pixmap, 100, 50);
    assert!(center.a > 0, "형광펜도 잉크다");
    assert!(
        center.a < 255,
        "형광펜은 반투명이어야 아래 글자가 비친다: {center:?}"
    );
}

#[test]
fn pdf_background_is_composited_under_the_ink() {
    let size = common::sample_pdf_size();
    let source = light_note_core::pdf::PdfDocument::from_bytes(common::sample_pdf()).expect("PDF");
    let background = source.render_page(0, 1.0).expect("배경 렌더");

    // 배경만: 왼쪽은 빨강, 오른쪽은 흰색.
    let red = pixel_at(&background, 30, 50);
    let white = pixel_at(&background, 170, 50);
    assert!(red.r > 200 && red.g < 80, "왼쪽은 빨간 사각형: {red:?}");
    assert!(white.r > 230 && white.g > 230, "오른쪽은 흰 종이: {white:?}");

    // 잉크를 올리면 그 자리만 바뀐다.
    let mut page = Page::with_pdf_background(size, 0);
    page.push_stroke(common::line_stroke(
        Tool::Pen,
        Point::new(150.0, 50.0),
        Point::new(190.0, 50.0),
        8,
        4.0,
    ));
    let composed = light_note_core::raster::render_page(&page, Some(&background), 1.0);
    let inked = pixel_at(&composed, 170, 50);
    assert!(inked.a > 200 && inked.b > 100, "잉크가 흰 종이 위에 올라간다");
    let still_red = pixel_at(&composed, 30, 50);
    assert!(still_red.r > 200 && still_red.g < 80, "배경은 그대로");
}

#[test]
fn white_page_rendering_is_opaque() {
    let page = page_with_strokes(Vec::new(), Size::new(50.0, 50.0));
    let pixmap = render_page_on_white(&page, None, 1.0);
    let corner = pixel_at(&pixmap, 1, 1);
    assert_eq!((corner.r, corner.g, corner.b, corner.a), (255, 255, 255, 255));
}

#[test]
fn composite_keeps_the_background_where_the_source_is_transparent() {
    let size = Size::new(20.0, 20.0);
    let mut base = render_page_on_white(&Page::blank(size), None, 1.0);
    let overlay = render_ink(
        &[Stroke::new(
            Tool::Pen,
            StrokeStyle::pen(Rgba::BLACK, 6.0),
            InkPoint::new(10.0, 10.0),
        )],
        size,
        1.0,
    );
    composite_into(&mut base, &overlay);
    assert!(pixel_at(&base, 10, 10).r < 60, "가운데는 검은 점");
    assert!(pixel_at(&base, 0, 0).r > 230, "구석은 흰 종이 그대로");
}

#[test]
fn png_round_trip_preserves_pixels() {
    let size = Size::new(64.0, 32.0);
    let page = page_with_strokes(
        vec![common::line_stroke(
            Tool::Pen,
            Point::new(4.0, 16.0),
            Point::new(60.0, 16.0),
            12,
            3.0,
        )],
        size,
    );
    let pixmap = render_page_on_white(&page, None, 1.0);
    let bytes = raster::to_png(pixmap).expect("PNG 인코딩");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG 시그니처");

    let decoded = raster::from_png(&bytes).expect("PNG 디코딩");
    assert_eq!((decoded.width(), decoded.height()), (64, 32));
    assert!(pixel_at(&decoded, 32, 16).a > 200, "잉크 픽셀이 살아 있다");
}

#[test]
fn document_pages_render_through_the_surface_contract() {
    let mut document = Document::blank(Size::new(200.0, 100.0));
    common::draw_line(
        &mut document,
        Point::new(20.0, 50.0),
        Point::new(180.0, 50.0),
        4.0,
    );

    let page = document.active_page();
    let surface = InkSurface::build(page.size(), 1.0, page.strokes(), document.live_stroke());
    assert_eq!((surface.width, surface.height), (200, 100));
    assert!(surface.static_png.is_some(), "확정 레이어 PNG가 있어야 한다");
    assert!(surface.lines.is_empty(), "드래그가 끝났으면 라이브 선분은 없다");

    let decoded = surface.decode_static().expect("PNG 디코딩");
    assert!(pixel_at(&decoded, 100, 50).a > 200);
}

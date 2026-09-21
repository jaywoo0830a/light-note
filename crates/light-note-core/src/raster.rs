//! 래스터라이저 — 스트로크를 픽셀로 바꾼다.
//!
//! hayro가 재수출하는 `vello_cpu`(= hayro 자신의 CPU 래스터 엔진)를 그대로 쓴다.
//! 그래서 **PDF 배경과 우리 잉크가 같은 픽셀 타입**([`Pixmap`], 프리멀티플라이드
//! RGBA8)을 공유하고, 합성·내보내기·WinUI 전달이 모두 같은 버퍼로 이어진다.
//!
//! 계약:
//! - 좌표는 페이지 pt → 픽셀 변환은 [`ViewTransform`] **하나만** 한다.
//! - 스트로크는 부드러운 폴리라인(중점 2차 베지어)으로 그리고, 압력이 변하면
//!   구간별 폭으로 나눠 그린다(둥근 캡이라 이음매가 보이지 않는다).
//! - 지우개는 여기서 그리지 않는다 — 모델에서 스트로크를 제거한다([`crate::doc`]).

use hayro::vello_cpu::{
    kurbo::{BezPath, Cap, Circle, Join, Point as KurboPoint, Shape, Stroke as KurboStroke},
    peniko::color::{AlphaColor, Srgb},
    Pixmap, RenderContext, Resources,
};

use crate::doc::Page;
use crate::geom::{Point, Size};
use crate::ink::{Rgba, Stroke};

/// 페이지(pt) ↔ 픽셀 변환. `scale` = 1pt당 픽셀 수 (1.0 = 72dpi, 2.0 = 144dpi).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewTransform {
    pub scale: f32,
}

impl ViewTransform {
    /// Windows 11의 150% 배율을 기본값으로 둔다.
    pub const DEFAULT_SCALE: f32 = 1.5;

    pub const fn new(scale: f32) -> Self {
        Self { scale }
    }

    /// 페이지 크기(pt) → 픽셀 크기(최소 1px, u16 한계로 클램프).
    pub fn pixels(self, size: Size) -> (u16, u16) {
        let width = (size.width * self.scale).round().clamp(1.0, u16::MAX as f32) as u16;
        let height = (size.height * self.scale).round().clamp(1.0, u16::MAX as f32) as u16;
        (width, height)
    }

    /// 픽셀 좌표(캔버스 로컬) → 페이지 좌표(pt).
    pub fn to_page(self, x: f32, y: f32) -> Point {
        Point::new(x / self.scale, y / self.scale)
    }

    /// 페이지 좌표(pt) → 픽셀 좌표.
    pub fn to_pixels(self, point: Point) -> (f32, f32) {
        (point.x * self.scale, point.y * self.scale)
    }

    /// pt 단위 길이 → 픽셀.
    pub fn len(self, pt: f32) -> f32 {
        pt * self.scale
    }
}

/// 스트로크를 그릴 (폭, 경로) 목록으로 바꾼다.
///
/// - 압력이 일정하면 **경로 하나**로 그린다(형광펜/마우스 입력의 일반 경로).
/// - 압력이 변하면 구간별로 나눈다 — 둥근 캡이 이음매를 메운다.
pub fn stroke_paths(stroke: &Stroke, scale: f32) -> Vec<(f32, BezPath)> {
    let points = stroke.points();
    if points.is_empty() {
        return Vec::new();
    }
    if points.len() == 1 {
        let radius = (stroke.style.width_at(points[0].pressure) * scale * 0.5).max(0.35);
        let circle = Circle::new(
            KurboPoint::new(points[0].x() as f64 * scale as f64, points[0].y() as f64 * scale as f64),
            radius as f64,
        );
        return vec![(0.0, circle.to_path(0.05))];
    }

    let widths: Vec<f32> = points
        .iter()
        .map(|p| stroke.style.width_at(p.pressure) * scale)
        .collect();
    let max = widths.iter().copied().fold(0.0_f32, f32::max);
    let min = widths.iter().copied().fold(f32::MAX, f32::min);
    let uniform = (max - min) <= max * 0.05;

    if uniform {
        return vec![(max, polyline(points, scale))];
    }

    let mut paths = Vec::with_capacity(points.len() - 1);
    for (index, pair) in points.windows(2).enumerate() {
        let width = (widths[index] + widths[index + 1]) * 0.5;
        let mut path = BezPath::new();
        path.move_to((
            pair[0].x() as f64 * scale as f64,
            pair[0].y() as f64 * scale as f64,
        ));
        path.line_to((
            pair[1].x() as f64 * scale as f64,
            pair[1].y() as f64 * scale as f64,
        ));
        paths.push((width, path));
    }
    paths
}

/// 중점을 잇는 2차 베지어 폴리라인 — 각진 입력을 부드럽게 만든다.
fn polyline(points: &[crate::ink::InkPoint], scale: f32) -> BezPath {
    let at = |p: &crate::ink::InkPoint| {
        (
            p.x() as f64 * scale as f64,
            p.y() as f64 * scale as f64,
        )
    };
    let mut path = BezPath::new();
    path.move_to(at(&points[0]));
    if points.len() == 2 {
        path.line_to(at(&points[1]));
        return path;
    }
    for pair in points.windows(2).skip(1) {
        // 제어점 = 직전 점, 끝점 = 직전/현재의 중점 → 곡선이 점들을 관통한다.
        let end = (
            (pair[0].x() + pair[1].x()) as f64 * 0.5 * scale as f64,
            (pair[0].y() + pair[1].y()) as f64 * 0.5 * scale as f64,
        );
        path.quad_to(at(&pair[0]), end);
    }
    let last = points[points.len() - 1];
    path.line_to(at(&last));
    path
}

fn kurbo_stroke(width: f32) -> KurboStroke {
    KurboStroke {
        width: width as f64,
        join: Join::Round,
        miter_limit: 4.0,
        start_cap: Cap::Round,
        end_cap: Cap::Round,
        dash_pattern: Vec::new().into(),
        dash_offset: 0.0,
    }
}

fn paint(color: Rgba) -> AlphaColor<Srgb> {
    AlphaColor::from_rgba8(color.r, color.g, color.b, color.a)
}

/// 페이지(pt)를 픽셀로 그릴 크기.
pub fn page_pixel_size(size: Size, scale: f32) -> (u16, u16) {
    ViewTransform::new(scale).pixels(size)
}

/// 잉크만 **투명 배경**에 그린다 — WinUI 레이어의 재료이자 합성 입력.
pub fn render_ink(strokes: &[Stroke], size: Size, scale: f32) -> Pixmap {
    let (width, height) = page_pixel_size(size, scale);
    let mut context = RenderContext::new(width, height);
    let mut painted = false;

    for stroke in strokes {
        if stroke.tool.is_eraser() {
            continue; // 지우개는 모델에서 사라진다(래스터 대상이 아니다).
        }
        let color = paint(stroke.style.color);
        for (width_pt, path) in stroke_paths(stroke, scale) {
            if width_pt <= 0.0 {
                context.set_paint(color);
                context.fill_path(&path); // 점 하나 = 채운 원
            } else {
                context.set_paint(color);
                context.set_stroke(kurbo_stroke(width_pt));
                context.stroke_path(&path);
            }
            painted = true;
        }
    }

    let mut pixmap = Pixmap::new(width, height);
    if painted {
        let mut resources = Resources::new();
        context.render_to_pixmap(&mut resources, &mut pixmap);
    }
    pixmap
}

/// 페이지를 그린다 — `background`(PDF 래스터)가 있으면 아래에 깔고 잉크를 올린다.
pub fn render_page(page: &Page, background: Option<&Pixmap>, scale: f32) -> Pixmap {
    let (width, height) = page_pixel_size(page.size(), scale);
    let mut canvas = Pixmap::new(width, height);
    if let Some(background) = background {
        composite_into(&mut canvas, background);
    }
    composite_into(&mut canvas, &render_ink(page.strokes(), page.size(), scale));
    canvas
}

/// 흰 종이 위의 페이지 — PNG/PDF 내보내기의 기준 이미지.
pub fn render_page_on_white(page: &Page, background: Option<&Pixmap>, scale: f32) -> Pixmap {
    let (width, height) = page_pixel_size(page.size(), scale);
    let mut canvas = Pixmap::new(width, height);
    fill(&mut canvas, Rgba::WHITE);
    if let Some(background) = background {
        composite_into(&mut canvas, background);
    }
    composite_into(&mut canvas, &render_ink(page.strokes(), page.size(), scale));
    canvas
}

/// 픽스맵 전체를 색으로 채운다(프리멀티플라이드로 변환해서 넣는다).
pub fn fill(pixmap: &mut Pixmap, color: Rgba) {
    let alpha = color.a as u32;
    let r = ((color.r as u32 * alpha + 127) / 255) as u8;
    let g = ((color.g as u32 * alpha + 127) / 255) as u8;
    let b = ((color.b as u32 * alpha + 127) / 255) as u8;
    for pixel in pixmap.data_as_u8_slice_mut().chunks_exact_mut(4) {
        pixel.copy_from_slice(&[r, g, b, color.a]);
    }
}

/// `src`를 `dst` 위에 합성한다 (프리멀티플라이드 `src over dst`, dst 크기 기준).
pub fn composite_into(dst: &mut Pixmap, src: &Pixmap) {
    let (dst_w, dst_h) = (dst.width() as usize, dst.height() as usize);
    let (src_w, src_h) = (src.width() as usize, src.height() as usize);
    if src_w == 0 || src_h == 0 {
        return;
    }
    let src_data = src.data_as_u8_slice();
    let dst_data = dst.data_as_u8_slice_mut();

    for y in 0..dst_h.min(src_h) {
        for x in 0..dst_w.min(src_w) {
            let source = (y * src_w + x) * 4;
            let target = (y * dst_w + x) * 4;
            let alpha = src_data[source + 3] as u32;
            if alpha == 0 {
                continue;
            }
            if alpha == 255 {
                dst_data[target..target + 4].copy_from_slice(&src_data[source..source + 4]);
                continue;
            }
            let inverse = 255 - alpha;
            for channel in 0..4 {
                let value = src_data[source + channel] as u32
                    + (dst_data[target + channel] as u32 * inverse + 127) / 255;
                dst_data[target + channel] = value.min(255) as u8;
            }
        }
    }
}

/// 프리멀티플라이드 픽셀 하나를 **비프리멀티플라이드** RGBA로 읽는다.
pub fn pixel_at(pixmap: &Pixmap, x: u16, y: u16) -> Rgba {
    let (width, height) = (pixmap.width() as usize, pixmap.height() as usize);
    if x as usize >= width || y as usize >= height {
        return Rgba::new(0, 0, 0, 0);
    }
    let data = pixmap.data_as_u8_slice();
    let index = (y as usize * width + x as usize) * 4;
    let alpha = data[index + 3];
    if alpha == 0 {
        return Rgba::new(0, 0, 0, 0);
    }
    let unpremultiply = |value: u8| {
        ((value as u32 * 255 + alpha as u32 / 2) / alpha as u32).min(255) as u8
    };
    Rgba::new(
        unpremultiply(data[index]),
        unpremultiply(data[index + 1]),
        unpremultiply(data[index + 2]),
        alpha,
    )
}

/// 잉크가 올라간 픽셀 수 — "그렸다/안 그렸다"의 객관적 증거(테스트 계약).
pub fn ink_coverage(pixmap: &Pixmap) -> usize {
    pixmap
        .data_as_u8_slice()
        .chunks_exact(4)
        .filter(|pixel| pixel[3] > 8)
        .count()
}

/// 흰 종이 위에 평탄화해 RGB8로 만든다 — PDF 이미지 XObject에 넣을 바이트.
pub fn flatten_to_rgb8(pixmap: &Pixmap) -> Vec<u8> {
    let data = pixmap.data_as_u8_slice();
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    for pixel in data.chunks_exact(4) {
        let alpha = pixel[3] as u32;
        let inverse = 255 - alpha;
        for channel in 0..3 {
            let value = pixel[channel] as u32 + (255 * inverse + 127) / 255;
            out.push(value.min(255) as u8);
        }
    }
    out
}

/// PNG로 인코딩 — hayro가 쓰는 vello `Pixmap`의 인코더를 그대로 쓴다.
pub fn to_png(pixmap: Pixmap) -> Result<Vec<u8>, String> {
    pixmap.into_png().map_err(|error| error.to_string())
}

/// PNG에서 복원 — WinUI `EncodedImage`로 넘긴 바이트를 테스트가 되짚을 때 쓴다.
pub fn from_png(bytes: &[u8]) -> Result<Pixmap, String> {
    Pixmap::from_png(std::io::Cursor::new(bytes)).map_err(|error| error.to_string())
}

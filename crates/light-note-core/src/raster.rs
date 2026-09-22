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
    kurbo::{
        flatten as kurbo_flatten, Affine, BezPath, Circle, PathEl, Point as KurboPoint, Rect, Shape,
    },
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
        let width = (size.width * self.scale)
            .round()
            .clamp(1.0, u16::MAX as f32) as u16;
        let height = (size.height * self.scale)
            .round()
            .clamp(1.0, u16::MAX as f32) as u16;
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

/// 곡선을 직선 조각으로 펼 때의 **허용 오차**(픽셀).
///
/// 래스터(PNG)와 라이브 레이어가 **같은 값**을 쓴다 — 값이 다르면 한쪽만 얇아져서
/// 획을 확정하는 순간 잉크가 바뀐다. 0.25px면 150% 배율에서도 보이지 않는다.
pub const FLATTEN_TOLERANCE_PX: f64 = 0.25;

/// 곡선 → 직선 구간 목록 — **래스터와 라이브 레이어가 같은 펴기**를 쓴다.
///
/// 라이브 레이어(WinUI)는 곡선을 그릴 수 없으므로, 래스터도 같은 폴리라인으로 그려
/// 두 쪽이 **픽셀 단위로 같은 잉크**가 되게 한다(폴리라인은 곡선 안쪽에 새겨지므로,
/// 한쪽만 펴면 그쪽만 얇아진다 — 승격 순간에 눈에 띄는 차이다).
pub fn flatten_spans(path: &BezPath, width: f32, tolerance: f64) -> Vec<InkSpan> {
    let mut points: Vec<(f64, f64)> = Vec::new();
    kurbo_flatten(path.iter(), tolerance, |element| match element {
        PathEl::MoveTo(point) | PathEl::LineTo(point) => points.push((point.x, point.y)),
        _ => {}
    });
    points
        .windows(2)
        .map(|pair| InkSpan {
            width,
            from: pair[0],
            to: pair[1],
        })
        .collect()
}

/// 스트로크 하나를 그릴 **도형** — 래스터와 라이브 레이어가 이 **하나의 결정**을 공유한다.
///
/// 라이브 레이어(WinUI `Line` + `Ellipse`)는 곡선을 그릴 수 없으므로 도형을 **직선 조각**으로
/// 펴서 그린다([`crate::surface::live_ink`]). 두 쪽이 같은 결정을 쓰지 않으면 획을 확정하는
/// 순간(라이브 → PNG 승격)에 잉크 모양이 "딱" 바뀐다 — 그게 부자연스러움의 정체다.
#[derive(Clone, Debug, PartialEq)]
pub enum InkShape {
    /// 폭이 일정하다 → 중점 2차 베지어 곡선 **하나**(둥근 캡/조인으로 긋는다).
    Curve { width: f32, path: BezPath },
    /// 폭이 변한다 → 구간별 직선 + 둥근 캡.
    Spans { spans: Vec<InkSpan> },
    /// 점 하나 → 채운 원.
    Dot { center: (f64, f64), radius: f64 },
}

impl InkShape {
    /// 이 도형을 **늘린 구간 사각형 + 둥근 캡**의 합집합으로 — 래스터와 라이브가 함께 쓴다.
    pub fn union(&self) -> BezPath {
        match self {
            InkShape::Curve { width, path } => {
                spans_union(&flatten_spans(path, *width, FLATTEN_TOLERANCE_PX))
            }
            InkShape::Spans { spans } => spans_union(spans),
            InkShape::Dot { center, radius } => {
                let mut dot = BezPath::new();
                push_disc(&mut dot, *center, *radius);
                dot
            }
        }
    }
}

/// 가변 폭 스트로크의 구간 하나 — 폭은 두 끝 압력의 평균(픽셀 좌표).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InkSpan {
    pub width: f32,
    pub from: (f64, f64),
    pub to: (f64, f64),
}

impl InkSpan {
    /// 둥근 캡의 반지름 — 래스터도 라이브 레이어도 **이 값**을 쓴다.
    pub fn half_width(&self) -> f64 {
        (self.width as f64 * 0.5).max(0.35)
    }
}

/// 스트로크 → 도형. 빈 스트로크는 `None`.
///
/// - 압력이 일정하면 **곡선 하나**로 그린다(중점 2차 베지어).
/// - 압력이 변하면 구간별 직선으로 나눈다 — 둥근 캡이 이음매를 메운다.
pub fn ink_shape(stroke: &Stroke, scale: f32) -> Option<InkShape> {
    let points = stroke.points();
    if points.is_empty() {
        return None;
    }
    if points.len() == 1 {
        return Some(InkShape::Dot {
            center: at(&points[0], scale),
            radius: (stroke.style.width_at(points[0].pressure) * scale * 0.5).max(0.35) as f64,
        });
    }

    let widths: Vec<f32> = points
        .iter()
        .map(|p| stroke.style.width_at(p.pressure) * scale)
        .collect();
    let max = widths.iter().copied().fold(0.0_f32, f32::max);
    let min = widths.iter().copied().fold(f32::MAX, f32::min);
    if (max - min) <= max * 0.05 {
        return Some(InkShape::Curve {
            width: max,
            path: polyline(points, scale),
        });
    }

    let spans = points
        .windows(2)
        .enumerate()
        .map(|(index, pair)| InkSpan {
            width: (widths[index] + widths[index + 1]) * 0.5,
            from: at(&pair[0], scale),
            to: at(&pair[1], scale),
        })
        .collect();
    Some(InkShape::Spans { spans })
}

/// 점 하나를 픽셀 좌표로.
fn at(point: &crate::ink::InkPoint, scale: f32) -> (f64, f64) {
    (
        point.x() as f64 * scale as f64,
        point.y() as f64 * scale as f64,
    )
}

/// 관절을 덮도록 구간을 늘린 사본.
///
/// **원은 변의 절반이 r인 정사각형에 내접한다** — 그래서 앞 구간을 자기 반지름만큼 앞으로
/// 늘리면 관절의 원(둥근 조인)이 **전부 덮인다**. WinUI 라이브 레이어에는 원을 하나 더
/// 그릴 필요가 없다(도형 수가 늘지 않는다). 대신 관절 모서리가 `≈0.15r²`만큼 더 채워지는데
/// 눈에 띄지 않고, 그 덕분에 **래스터(PNG/PDF)와 라이브 레이어가 완전히 같은 도형**이 된다.
///
/// 첫 구간의 시작과 마지막 구간의 끝은 늘리지 않는다 — 그 자리는 둥근 **캡**(원)이 맡는다.
pub fn extended_span(span: &InkSpan, extend_from: bool, extend_to: bool) -> InkSpan {
    let (x0, y0) = span.from;
    let (x1, y1) = span.to;
    let (dx, dy) = (x1 - x0, y1 - y0);
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f64::EPSILON {
        return *span;
    }
    let (ux, uy) = (dx / length, dy / length);
    let r = span.half_width();
    InkSpan {
        width: span.width,
        from: if extend_from {
            (x0 - ux * r, y0 - uy * r)
        } else {
            span.from
        },
        to: if extend_to {
            (x1 + ux * r, y1 + uy * r)
        } else {
            span.to
        },
    }
}

/// 가변 폭 스트로크 = **늘린 구간 사각형 + 양 끝의 둥근 캡** 합집합.
///
/// 한 번의 `fill_path`(nonzero)로 채우기 때문에 겹치는 부분이 **두 번 곱해지지 않는다**.
/// 예전에는 구간마다 따로 `stroke_path`했고, 그래서 반투명 획(형광펜)은 관절마다 알파가
/// 겹쳐 **구슬처럼 얼룩**졌다(실측: 중간 90 → 관절 149). 라이브 레이어가 캡을 겹쳐 그리는
/// 것과 같은 규칙이라, 승격 순간에도 색이 바뀌지 않는다.
pub fn spans_union(spans: &[InkSpan]) -> BezPath {
    let mut path = BezPath::new();
    let last = spans.len().saturating_sub(1);
    for (index, span) in spans.iter().enumerate() {
        let extended = extended_span(span, index > 0, index < last);
        push_span_rect(&mut path, &extended);
    }
    if let (Some(first), Some(end)) = (spans.first(), spans.last()) {
        push_disc(&mut path, first.from, first.half_width());
        push_disc(&mut path, end.to, end.half_width());
    }
    path
}

/// 채운 원 하나를 경로에 더한다(캡/점).
pub fn push_disc(path: &mut BezPath, center: (f64, f64), radius: f64) {
    let circle = Circle::new(KurboPoint::new(center.0, center.1), radius.max(0.35));
    path.extend(circle.to_path(0.05).iter());
}

/// 구간 사각형(회전)을 경로에 더한다 — `Rect`를 만들어 옮기고 돌린다.
///
/// 방향(감김)을 직접 만들지 않고 kurbo의 `Shape::to_path`에 맡긴다: 원([`push_disc`])과
/// **같은 방향**이어야 nonzero 채움에서 겹친 부분이 구멍이 되지 않는다.
pub fn push_span_rect(path: &mut BezPath, span: &InkSpan) {
    let (x0, y0) = span.from;
    let (x1, y1) = span.to;
    let (dx, dy) = (x1 - x0, y1 - y0);
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f64::EPSILON {
        push_disc(path, span.from, span.half_width());
        return;
    }
    let half = span.half_width();
    let mut quad = Rect::new(0.0, -half, length, half).to_path(0.01);
    quad.apply_affine(Affine::translate((x0, y0)) * Affine::rotate(dy.atan2(dx)));
    path.extend(quad.iter());
}

/// 중점을 잇는 2차 베지어 폴리라인 — 각진 입력을 부드럽게 만든다.
fn polyline(points: &[crate::ink::InkPoint], scale: f32) -> BezPath {
    let at = |p: &crate::ink::InkPoint| (p.x() as f64 * scale as f64, p.y() as f64 * scale as f64);
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

/// 캡/조인 없이 **한 번에 채우기**만 쓴다 — 둥근 캡은 [`push_disc`]가, 관절은
/// [`extended_span`]이 맡는다(래스터와 라이브 레이어가 같은 도형이 되도록).

fn paint(color: Rgba) -> AlphaColor<Srgb> {
    AlphaColor::from_rgba8(color.r, color.g, color.b, color.a)
}

/// 페이지(pt)를 픽셀로 그릴 크기.
pub fn page_pixel_size(size: Size, scale: f32) -> (u16, u16) {
    ViewTransform::new(scale).pixels(size)
}

/// 잉크만 **투명 배경**에 그린다 — WinUI 레이어의 재료이자 합성 입력.
///
/// 도형은 [`ink_shape`]가 정하고, 여기서는 **어떻게 채우는가**만 정한다: 한 획은 언제나
/// [`InkShape::union`]을 **한 번의 `fill_path`**(nonzero)로 채운다 — 겹쳐도 알파가 두 번
/// 곱해지지 않는다(예전에는 구간마다 따로 `stroke_path`해서 형광펜 관절이 얼룩졌다).
/// 라이브 레이어도 같은 합집합을 도형 여러 개 + **그룹 불투명도**로 그린다.
pub fn render_ink(strokes: &[Stroke], size: Size, scale: f32) -> Pixmap {
    let (width, height) = page_pixel_size(size, scale);
    let mut context = RenderContext::new(width, height);
    let mut painted = false;

    for stroke in strokes {
        if stroke.tool.is_eraser() {
            continue; // 지우개는 모델에서 사라진다(래스터 대상이 아니다).
        }
        let Some(shape) = ink_shape(stroke, scale) else {
            continue;
        };
        context.set_paint(paint(stroke.style.color));
        context.fill_path(&shape.union());
        painted = true;
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
    let unpremultiply =
        |value: u8| ((value as u32 * 255 + alpha as u32 / 2) / alpha as u32).min(255) as u8;
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

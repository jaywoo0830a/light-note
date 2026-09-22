//! 도형과 픽셀 — **래스터(④-워커)와 라이브 도형(④-UI)이 이 하나의 결정을 공유한다.**
//!
//! 이 파일이 지키는 계약:
//! - 한 획의 도형은 [`ink_shape`]가 **한 번만** 정한다(곡선 / 구간 / 점).
//! - 곡선은 같은 오차로 직선 조각으로 펴고([`flatten_spans`]), **래스터도 그 폴리라인을
//!   채운다** — 한쪽만 펴면 그쪽만 얇아져서 "획을 확정하는 순간"에 잉크가 바뀐다.
//! - 관절의 둥근 조인은 **구간 사각형을 반지름만큼 늘려** 덮는다([`extended_span`]).
//!   원은 변의 절반이 `r`인 정사각형에 내접하므로 늘린 사각형이 관절을 **전부** 덮는다 →
//!   도형을 더 그릴 필요가 없다(WinUI `Line`에는 캡 속성이 없고 `Path`도 없다).
//! - 한 획은 **한 번의 `fill_path`**(nonzero)로 채운다 — 겹쳐도 알파가 두 번 곱해지지 않는다.
//!   라이브 도형도 같은 이유로 **한 획 = 한 합성 그룹**(투명도는 그룹이 맡는다).

use std::sync::{Arc, OnceLock};

use hayro::vello_cpu::kurbo::{
    flatten as kurbo_flatten, Affine, BezPath, Circle, PathEl, Point, Rect, Shape,
};
use hayro::vello_cpu::peniko::color::{AlphaColor, Srgb};
use hayro::vello_cpu::{Pixmap, RenderContext, Resources};

use crate::geom::{Scale, Size};
use crate::ink::{InkPoint, Rgba, Stroke};

/// 곡선을 직선 조각으로 펼 때의 허용 오차(픽셀) — 래스터와 라이브가 **같은 값**을 쓴다.
pub const FLATTEN_TOLERANCE_PX: f64 = 0.25;

/// 한 획의 도형 — 래스터와 라이브가 이 결정을 함께 쓴다.
#[derive(Clone, Debug, PartialEq)]
pub enum InkShape {
    /// 폭이 일정하다 → 중점 2차 베지어 곡선 하나.
    Curve { width: f32, path: BezPath },
    /// 폭이 변한다 → 구간별 직선.
    Spans { spans: Vec<InkSpan> },
    /// 점 하나 → 채운 원.
    Dot { center: (f64, f64), radius: f64 },
}

/// 가변 폭 스트로크의 구간 하나(픽셀) — 폭은 두 끝 압력의 평균.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InkSpan {
    pub width: f32,
    pub from: (f64, f64),
    pub to: (f64, f64),
}

impl InkSpan {
    /// 둥근 캡/조인의 반지름 — 래스터와 라이브가 **같은 값**을 쓴다.
    pub fn half_width(&self) -> f64 {
        (self.width as f64 * 0.5).max(0.35)
    }
}

/// 스트로크 → 도형. 빈 획은 `None`.
pub fn ink_shape(stroke: &Stroke, scale: Scale) -> Option<InkShape> {
    let points = stroke.points();
    let scale = scale.get();
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
impl InkShape {
    /// 이 도형을 **늘린 구간 사각형 + 둥근 캡**의 합집합으로 — 래스터와 라이브가 함께 쓴다.
    ///
    /// 한 번의 `fill_path`(nonzero)로 채운다: 겹치는 조각이 **두 번 곱해지지 않는다**.
    /// (예전에는 구간마다 따로 `stroke_path`해서 반투명 획의 관절마다 알파가 겹쳐
    /// 구슬처럼 얼룩졌다 — 실측 중간 90 vs 관절 149.)
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

/// 중점을 잇는 2차 베지어 폴리라인 — 각진 입력을 부드럽게 만든다.
///
/// 제어점 = 직전 점, 끝점 = 직전/현재의 중점 → 곡선이 점들을 관통하고 G1 연속이다.
fn polyline(points: &[InkPoint], scale: f32) -> BezPath {
    let at = |p: &InkPoint| (p.pos.x as f64 * scale as f64, p.pos.y as f64 * scale as f64);
    let mut path = BezPath::new();
    path.move_to(at(&points[0]));
    if points.len() == 2 {
        path.line_to(at(&points[1]));
        return path;
    }
    for pair in points.windows(2).skip(1) {
        let end = (
            (pair[0].pos.x + pair[1].pos.x) as f64 * 0.5 * scale as f64,
            (pair[0].pos.y + pair[1].pos.y) as f64 * 0.5 * scale as f64,
        );
        path.quad_to(at(&pair[0]), end);
    }
    path.line_to(at(&points[points.len() - 1]));
    path
}

/// 점 하나를 픽셀 좌표로.
fn at(point: &InkPoint, scale: f32) -> (f64, f64) {
    (
        point.pos.x as f64 * scale as f64,
        point.pos.y as f64 * scale as f64,
    )
}

/// 관절을 덮도록 구간을 늘린 사본 — **원은 변의 절반이 `r`인 정사각형에 내접한다.**
///
/// 그래서 앞 구간을 자기 반지름만큼 앞으로 늘리면 관절의 원(둥근 조인)이 **전부** 덮인다 →
/// WinUI 라이브 레이어에 원을 하나 더 그릴 필요가 없다(도형 수가 늘지 않는다). 대신 관절
/// 모서리가 `≈0.15r²`만큼 더 채워지는데 눈에 띄지 않고, 그 덕분에 **래스터(PNG/PDF)와
/// 라이브 도형이 완전히 같은 모양**이 된다.
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

/// 곡선 → 직선 구간 목록 — 래스터와 라이브가 **같은 펴기**를 쓴다.
///
/// 라이브 도형(WinUI `Line` + `Ellipse`)은 곡선을 그릴 수 없으므로 래스터도 같은 폴리라인을
/// 채운다. 폴리라인은 곡선 **안쪽**에 새겨지므로, 한쪽만 펴면 그쪽만 얇아진다.
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

/// 가변 폭 스트로크 = **늘린 구간 사각형 + 양 끝의 둥근 캡** 합집합.
pub fn spans_union(spans: &[InkSpan]) -> BezPath {
    let mut path = BezPath::new();
    let last = spans.len().saturating_sub(1);
    for (index, span) in spans.iter().enumerate() {
        push_span_rect(&mut path, &extended_span(span, index > 0, index < last));
    }
    if let (Some(first), Some(end)) = (spans.first(), spans.last()) {
        push_disc(&mut path, first.from, first.half_width());
        push_disc(&mut path, end.to, end.half_width());
    }
    path
}

/// 채운 원 하나를 경로에 더한다(캡/점).
pub fn push_disc(path: &mut BezPath, center: (f64, f64), radius: f64) {
    let circle = Circle::new(Point::new(center.0, center.1), radius.max(0.35));
    path.extend(circle.to_path(0.05).iter());
}

/// 구간 사각형(회전)을 경로에 더한다 — 감김 방향은 kurbo에 맡긴다.
///
/// 직접 만들지 않는 이유: 원([`push_disc`])과 **같은 방향**이어야 nonzero 채움에서 겹친
/// 부분이 구멍이 되지 않는다.
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

// ── 라이브 도형 — ④-UI가 WinUI로 옮기는 도형들 ────────────────────────────────

/// 선분 하나(픽셀). **폭이 있어 캡이 필요 없다** — 늘린 사각형이 관절을 덮는다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveLine {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub width: f64,
}

/// 둥근 캡 하나 — 획의 양 끝(그리고 점 하나짜리 획).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveCap {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
}

/// 획 하나의 라이브 도형 — **한 획 = 한 합성 그룹**(투명도는 그룹이 맡는다).
///
/// 그룹이 필요한 이유: 반투명 획의 도형들이 겹치면(관절·캡) 알파가 두 번 곱해져 얼룩진다.
/// 래스터가 [`InkShape::union`]을 한 번에 채워 같은 문제를 피하는 것과 **같은 규칙**이다.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveInk {
    pub color: Rgba,
    pub lines: Vec<LiveLine>,
    pub caps: Vec<LiveCap>,
}

impl LiveInk {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.caps.is_empty()
    }

    /// ④-UI가 만들 WinUI 도형 수 — **예산 판단이 이 값으로 한다**.
    pub fn shape_count(&self) -> usize {
        self.lines.len() + self.caps.len()
    }

    /// 합성 그룹의 투명도 — 반투명 획(형광펜)만 1.0이 아니다.
    pub fn opacity(&self) -> f64 {
        self.color.a as f64 / 255.0
    }

    /// 그룹 안에서 그릴 색 — 알파는 그룹이 맡으므로 불투명하게 그린다.
    pub fn solid(&self) -> Rgba {
        self.color.with_alpha(255)
    }
}

/// 획 → 라이브 도형. **래스터와 같은 도형**([`ink_shape`] + [`FLATTEN_TOLERANCE_PX`])을 편다.
///
/// 그래서 획을 확정해도(라이브 → 베이스 PNG 승격) 잉크의 **모양과 색이 바뀌지 않는다.**
pub fn live_ink(stroke: &Stroke, scale: Scale) -> LiveInk {
    let color = stroke.style.color;
    let mut live = LiveInk {
        color,
        lines: Vec::new(),
        caps: Vec::new(),
    };
    match ink_shape(stroke, scale) {
        None => {}
        Some(InkShape::Dot { center, radius }) => live.caps.push(cap(center, radius)),
        Some(InkShape::Curve { width, path }) => live_spans(
            &mut live,
            &flatten_spans(&path, width, FLATTEN_TOLERANCE_PX),
        ),
        Some(InkShape::Spans { spans }) => live_spans(&mut live, &spans),
    }
    live
}

/// 구간 목록 → 선분 + 양 끝 캡 — 래스터의 합집합([`spans_union`])과 **같은 규칙**.
fn live_spans(live: &mut LiveInk, spans: &[InkSpan]) {
    let last = spans.len().saturating_sub(1);
    live.lines = spans
        .iter()
        .enumerate()
        .map(|(index, span)| {
            let drawn = extended_span(span, index > 0, index < last);
            LiveLine {
                x1: drawn.from.0,
                y1: drawn.from.1,
                x2: drawn.to.0,
                y2: drawn.to.1,
                width: span.width.max(0.5) as f64,
            }
        })
        .collect();
    if let (Some(first), Some(end)) = (spans.first(), spans.last()) {
        live.caps.push(cap(first.from, first.half_width()));
        live.caps.push(cap(end.to, end.half_width()));
    }
}

/// 둥근 캡 하나 — 래스터의 둥근 캡과 **같은 반지름**.
fn cap(center: (f64, f64), radius: f64) -> LiveCap {
    LiveCap {
        x: center.0,
        y: center.1,
        radius: radius.max(0.35),
    }
}

/// 확정 획들 + 진행 중 획 → 라이브 도형 목록(빈 획은 버린다).
///
/// **③ Canvas의 규칙**: 라이브 = "아직 베이스에 안 구운 확정 획" + "진행 중 획".
pub fn live_inks(unbaked: &[Stroke], drawing: Option<&Stroke>, scale: Scale) -> Vec<LiveInk> {
    let mut live: Vec<LiveInk> = unbaked.iter().map(|s| live_ink(s, scale)).collect();
    if let Some(stroke) = drawing {
        live.push(live_ink(stroke, scale));
    }
    live.retain(|ink| !ink.is_empty());
    live
}

// ── 래스터 — ④-워커만 부른다(픽셀 작업은 여기 하나뿐이다) ─────────────────────

/// 색 → vello 페인트(프리멀티플라이드는 vello가 처리한다).
fn paint(color: Rgba) -> AlphaColor<Srgb> {
    AlphaColor::from_rgba8(color.r, color.g, color.b, color.a)
}

/// 그릴 잉크가 있는가 — 베이스를 만들 가치가 있는지 **싸게** 판단한다.
///
/// 지우개는 모델에 획을 남기지 않으므로 "지우개가 아닌 획이 하나라도 있으면 잉크가 있다"가
/// 페이지를 래스터화하지 않고 즉시 같은 답을 준다.
pub fn has_ink(strokes: &[Stroke]) -> bool {
    strokes
        .iter()
        .any(|stroke| !stroke.tool.is_eraser() && !stroke.is_empty())
}

/// 잉크만 **투명 배경**에 그린다 — 베이스 레이어의 재료.
///
/// 도형은 [`ink_shape`]가 정하고, 여기서는 **어떻게 채우는가**만 정한다: 한 획은 언제나
/// [`InkShape::union`]을 **한 번의 `fill_path`**(nonzero)로 채운다.
pub fn render_ink(strokes: &[Stroke], size: Size, scale: Scale) -> Pixmap {
    let (width, height) = scale.pixels(size);
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

/// 페이지(pt)를 덮는 **투명 PNG** — ④-워커가 만드는 베이스 한 장.
///
/// 잉크가 없으면 `None`이다 — 빈 페이지에 1.13Mpx 이미지를 만들 이유가 없다.
pub fn bake(strokes: &[Stroke], size: Size, scale: Scale) -> Option<Arc<[u8]>> {
    if !has_ink(strokes) {
        return None;
    }
    to_png(render_ink(strokes, size, scale)).ok().map(Arc::from)
}

/// 흰 종이 위의 페이지 — PNG/PDF 내보내기의 기준 이미지.
pub fn render_on_white(
    strokes: &[Stroke],
    size: Size,
    background: Option<&Pixmap>,
    scale: Scale,
) -> Pixmap {
    let (width, height) = scale.pixels(size);
    let mut canvas = Pixmap::new(width, height);
    fill(&mut canvas, crate::ink::Rgba::rgb(255, 255, 255));
    if let Some(background) = background {
        composite_into(&mut canvas, background);
    }
    composite_into(&mut canvas, &render_ink(strokes, size, scale));
    canvas
}

/// 픽스맵 전체를 색으로 채운다(프리멀티플라이드로 변환해서 넣는다).
pub fn fill(pixmap: &mut Pixmap, color: crate::ink::Rgba) {
    let alpha = color.a as u32;
    let r = ((color.r as u32 * alpha + 127) / 255) as u8;
    let g = ((color.g as u32 * alpha + 127) / 255) as u8;
    let b = ((color.b as u32 * alpha + 127) / 255) as u8;
    for pixel in pixmap.data_as_u8_slice_mut().as_chunks_mut::<4>().0 {
        *pixel = [r, g, b, color.a];
    }
}

/// `src`를 `dst` 위에 합성한다(프리멀티플라이드 `src over dst`).
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

/// PNG로 인코딩한다(vello `Pixmap`이 그대로 압축한다).
pub fn to_png(pixmap: Pixmap) -> Result<Vec<u8>, String> {
    pixmap.into_png().map_err(|error| error.to_string())
}

/// PNG에서 복원 — 테스트가 바이트를 검사하거나 배경을 재사용할 때.
pub fn from_png(bytes: &[u8]) -> Result<Pixmap, String> {
    Pixmap::from_png(std::io::Cursor::new(bytes)).map_err(|error| error.to_string())
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
    let unpremultiply = |value: u8| ((value as u32 * 255 + alpha as u32 / 2) / alpha as u32) as u8;
    Rgba::new(
        unpremultiply(data[index]),
        unpremultiply(data[index + 1]),
        unpremultiply(data[index + 2]),
        alpha,
    )
}

/// 픽스맵의 알파를 8비트 그레이로 편다 — 합성/내보내기 검사용.
pub fn flatten_to_rgb8(pixmap: &Pixmap) -> Vec<u8> {
    let data = pixmap.data_as_u8_slice();
    data.as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| pixel[3])
        .collect()
}

/// 빈 레이어용 **1×1 투명 PNG** — 한 번만 만들어 재사용한다(`Arc`라 복사도 없다).
///
/// 왜 필요한가: 어댑터에는 **`Image`의 소스를 비우는 API가 없다**(`Property::Inherited`는
/// "그대로 두기"다). 그래서 "빈 레이어"는 소스 없음이 아니라 **빈 그림**으로 표현한다.
pub fn blank_png() -> Arc<[u8]> {
    static BLANK: OnceLock<Arc<[u8]>> = OnceLock::new();
    Arc::clone(BLANK.get_or_init(|| {
        to_png(Pixmap::new(1, 1))
            .map(Arc::from)
            .unwrap_or_else(|_| Arc::from(Vec::new()))
    }))
}

/// 잉크가 덮은 픽셀 수 — "빈 페이지인가"를 픽셀로 확인할 때.
pub fn ink_coverage(pixmap: &Pixmap) -> usize {
    pixmap
        .data_as_u8_slice()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[3] > 0)
        .count()
}

/// 불투명 캔버스 크기(픽셀) — 디버그/상태 표시용.
pub fn pixel_size(size: Size, scale: Scale) -> (u16, u16) {
    scale.pixels(size)
}

//! WinUI 표면이 **무엇을 그릴지** — 플랫폼 독립 서술.
//!
//! WinUI에서는 `<Raw>`가 `Canvas` + `Line`(라이브 레이어) +
//! `Image(EncodedImage)`(정적 레이어)로 번역한다. 그 번역의 **입력이 이 파일**이라,
//! 표면 계약이 리눅스 CI에서도 검증된다.
//!
//! ## Windows 11 최적화 규칙 (계약)
//! - **확정 레이어는 PNG 한 장** — 드래그가 *끝날 때만* 다시 만든다. 프레임마다
//!   전체를 다시 그리지 않는다(잉크가 많아도 입력 지연이 늘지 않는다).
//! - **라이브 레이어는 선분 목록** — 진행 중인 획 하나만 WinUI 도형으로 그린다.
//!   WinUI 도형은 GPU 합성이라 포인터 이동에 즉시 반응한다.
//! - 좌표는 **표면 DIP** 이며, WinUI `Canvas`의 `Line.X1/Y1/X2/Y2`에 그대로 들어간다.

use crate::geom::Size;
use crate::ink::{Rgba, Stroke};
use crate::raster::{self, ViewTransform};
use hayro::vello_cpu::Pixmap;

/// WinUI `Line` 하나로 그릴 선분 (표면 DIP 좌표).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceLine {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    /// 선 굵기(DIP).
    pub width: f64,
    pub color: Rgba,
}

impl SurfaceLine {
    /// 길이(DIP) — 0에 가까운 선분은 WinUI가 그리지 않으므로 점을 대신 만든다.
    pub fn length(&self) -> f64 {
        ((self.x2 - self.x1).powi(2) + (self.y2 - self.y1).powi(2)).sqrt()
    }
}

/// 표면 서술 — 정적 PNG + 라이브 선분 + 크기.
#[derive(Clone, Debug, PartialEq)]
pub struct InkSurface {
    /// 확정 스트로크를 그린 PNG(투명 배경). 스트로크가 없으면 `None`.
    pub static_png: Option<Vec<u8>>,
    /// 진행 중인 획의 선분들(라이브 레이어).
    pub lines: Vec<SurfaceLine>,
    /// 표면 크기(픽셀 = DIP) — WinUI `Canvas`의 `Width`/`Height`가 된다.
    pub width: u16,
    pub height: u16,
    /// 1pt당 픽셀 수.
    pub scale: f32,
}

impl InkSurface {
    /// 페이지 하나를 서술한다.
    ///
    /// `committed`는 확정 스트로크, `live`는 진행 중인 획이다. 정적 레이어는
    /// **확정 스트로크가 있을 때만** PNG를 만든다(빈 페이지는 이미지를 만들지 않는다).
    pub fn build(size: Size, scale: f32, committed: &[Stroke], live: Option<&Stroke>) -> Self {
        let (width, height) = raster::page_pixel_size(size, scale);
        Self {
            static_png: static_layer(committed, size, scale),
            lines: live.map(|stroke| live_lines(stroke, scale)).unwrap_or_default(),
            width,
            height,
            scale,
        }
    }

    /// 확정 레이어 PNG만 다시 만든다 — 스트로크가 커밋된 직후 한 번.
    pub fn rebuild_static(&mut self, committed: &[Stroke], size: Size) {
        self.static_png = static_layer(committed, size, self.scale);
    }

    pub fn is_empty(&self) -> bool {
        self.static_png.is_none() && self.lines.is_empty()
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// 정적 PNG 바이트 크기(계측/로그).
    pub fn static_bytes(&self) -> usize {
        self.static_png.as_ref().map(Vec::len).unwrap_or(0)
    }

    /// 정적 레이어를 디코드한다 — 호스트가 WinUI에 넘기기 전 확인용.
    pub fn decode_static(&self) -> Option<Pixmap> {
        self.static_png
            .as_deref()
            .and_then(|bytes| raster::from_png(bytes).ok())
    }
}

/// 확정 스트로크 → 투명 배경 PNG.
pub fn static_layer(strokes: &[Stroke], size: Size, scale: f32) -> Option<Vec<u8>> {
    if strokes.iter().all(|stroke| stroke.tool.is_eraser()) {
        return None;
    }
    let pixmap = raster::render_ink(strokes, size, scale);
    if raster::ink_coverage(&pixmap) == 0 {
        return None;
    }
    raster::to_png(pixmap).ok()
}

/// 진행 중인 획 → 선분 목록(라이브 레이어).
///
/// 폭은 구간 양 끝의 평균이다(선분마다 굵기를 바꿀 수 있는 WinUI의 `Line`에 맞춘다).
pub fn live_lines(stroke: &Stroke, scale: f32) -> Vec<SurfaceLine> {
    let transform = ViewTransform::new(scale);
    let color = stroke.style.color;
    let points = stroke.points();
    if points.is_empty() {
        return Vec::new();
    }

    if points.len() == 1 {
        // 점 하나 = 그리기: WinUI는 길이 0인 선을 그리지 않으므로 폭만큼 늘린다.
        let width = (stroke.style.width_at(points[0].pressure) * scale) as f64;
        let (x, y) = transform.to_pixels(points[0].pos);
        let half = (width * 0.5).max(0.5);
        return vec![SurfaceLine {
            x1: x as f64 - half,
            y1: y as f64,
            x2: x as f64 + half,
            y2: y as f64,
            width,
            color,
        }];
    }

    points
        .windows(2)
        .map(|pair| {
            let width = (stroke.style.width_at(pair[0].pressure)
                + stroke.style.width_at(pair[1].pressure))
                * 0.5
                * scale;
            let (x1, y1) = transform.to_pixels(pair[0].pos);
            let (x2, y2) = transform.to_pixels(pair[1].pos);
            SurfaceLine {
                x1: x1 as f64,
                y1: y1 as f64,
                x2: x2 as f64,
                y2: y2 as f64,
                width: width.max(0.5) as f64,
                color,
            }
        })
        .collect()
}

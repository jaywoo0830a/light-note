//! WinUI 표면이 **무엇을 그릴지** — 플랫폼 독립 서술.
//!
//! WinUI에서는 `<Raw>`가 `Canvas` + `Line`(라이브 레이어) +
//! `Image(EncodedImage)`(정적 레이어)로 번역한다. 그 번역의 **입력이 이 파일**이라,
//! 표면 계약이 리눅스 CI에서도 검증된다.
//!
//! ## Windows 11 최적화 규칙 (계약)
//! - **확정 레이어는 PNG 한 장** — 드래그가 *끝날 때만* 다시 만든다. 프레임마다
//!   전체를 다시 그리지 않는다(잉크가 많아도 입력 지연이 늘지 않는다).
//! - **그 PNG는 백그라운드에서 만든다** — 페이지 전체 래스터 + PNG 인코딩은
//!   A4에서 13ms(release)~750ms(dev)다(예제 `surface_cost`). 포인터 메시지 처리
//!   안에서 돌리면 **획을 끝낼 때마다 화면이 멈춘다** — 그래서 호스트는
//!   [`static_layer`]를 워커에 맡기고, 도착 전까지는 방금 확정한 획을
//!   [`pending_lines`]로 라이브 선분으로 계속 그린다(빈 틈이 보이지 않는다).
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
///
/// 잉크가 없으면 `None`이다 — 빈 페이지에 1.13Mpx 이미지를 만들 이유가 없다.
pub fn static_layer(strokes: &[Stroke], size: Size, scale: f32) -> Option<Vec<u8>> {
    if !has_ink(strokes) {
        return None;
    }
    raster::to_png(raster::render_ink(strokes, size, scale)).ok()
}

/// 그릴 잉크가 있는가 — 정적 레이어를 만들 가치가 있는지 **싸게** 판단한다.
///
/// 예전에는 페이지 전체를 래스터화한 뒤 [`raster::ink_coverage`]로 픽셀을 세었다
/// (1.13Mpx 전수 스캔 = dev 빌드 33ms, release 0.4ms). 지우개 드래그는 모델에
/// 스트로크를 남기지 않으므로 "지우개가 아닌 스트로크가 하나라도 있으면 잉크가 있다"가
/// 같은 답을 즉시 준다(LIMIT: 점이 없는 스트로크도 '없음'으로 본다).
pub fn has_ink(strokes: &[Stroke]) -> bool {
    strokes
        .iter()
        .any(|stroke| stroke.tool.is_draw() && !stroke.is_empty())
}

/// 라이브 레이어에 그릴 선분 — **아직 정적 PNG에 없는** 확정 획 + 진행 중인 획.
///
/// `snapshot`은 화면에 있는 정적 PNG가 반영한 확정 스트로크 수다. 그 PNG는
/// 백그라운드에서 만들어지므로(모듈 문서의 계약), 렌더가 끝나기 전에 확정된 획은
/// PNG에 없다 — 그 획들을 벡터 선분으로 계속 그려 **빈 틈을 메운다**.
///
/// `snapshot >= committed.len()`이면 꼬리는 없다: 화면의 PNG를 믿지 않는 상태
/// (되돌리기/페이지 이동/줌 직후)를 호스트가 그렇게 표시한다.
pub fn pending_lines(
    committed: &[Stroke],
    snapshot: usize,
    live: Option<&Stroke>,
    scale: f32,
) -> Vec<SurfaceLine> {
    let mut lines: Vec<SurfaceLine> = committed[snapshot.min(committed.len())..]
        .iter()
        .flat_map(|stroke| live_lines(stroke, scale))
        .collect();
    if let Some(stroke) = live {
        lines.extend(live_lines(stroke, scale));
    }
    lines
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

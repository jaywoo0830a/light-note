//! WinUI 표면이 **무엇을 그릴지** — 플랫폼 독립 서술.
//!
//! WinUI에서는 `<Raw>`가 `Canvas` + `Line`·`Ellipse`(라이브 레이어) +
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
//!   [`pending_ink`]로 라이브 도형으로 계속 그린다(빈 틈이 보이지 않는다).
//! - **정적 레이어는 두 장이다(더블 버퍼)** — WinUI `Image.Source`를 바꾸면 어댑터가
//!   먼저 소스를 비우고 나서 비동기 디코드를 하므로, 그 레이어가 **통째로 사라진다**.
//!   [`StaticLayers`]는 새 PNG를 보이지 않는 뒤 레이어에 올리고, 디코드가 끝난 신호를
//!   받은 뒤에만 앞뒤를 바꾼다 — **화면의 잉크는 한 프레임도 비지 않는다**.
//! - **라이브 레이어는 래스터와 *같은 도형*을 그린다** — 라이브 도형([`LiveStroke`])은
//!   [`crate::raster::ink_shape`]가 정한 도형을 **직선 조각 + 둥근 캡**으로 편 것이다.
//!   도형이 어긋나면 획을 확정하는 순간(라이브 → PNG 승격)에 잉크가 "딱" 바뀐다.
//! - **한 획 = 한 합성 그룹** — 래스터는 한 획을 *한 번의 채움*으로 그리므로 겹친 부분이
//!   두 번 곱해지지 않는다. 라이브 레이어도 같게 만들기 위해 색을 불투명으로 내려 그리고
//!   그룹 불투명도([`LiveStroke::opacity`])를 준다.
//! - 좌표는 **표면 DIP** 이며, WinUI `Line.X1/Y1/X2/Y2`·`Canvas.Left/Top`에 그대로 들어간다.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::geom::Size;
use crate::ink::{Rgba, Stroke};
use crate::raster::{self, InkShape, InkSpan};
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

/// WinUI `Ellipse` 하나로 그릴 **둥근 캡**(또는 점 하나) — 래스터의 둥근 캡과 **같은 원**.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceCap {
    /// 원의 중심(표면 DIP) — 빌더는 `(x - radius, y - radius)`에 놓는다.
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub color: Rgba,
}

/// 라이브 레이어의 **한 획** — WinUI에서는 **한 합성 그룹**이다.
///
/// ## 왜 그룹인가 (반투명 색)
/// 래스터는 한 획을 **한 번의 채움**으로 그린다([`crate::raster::render_ink`]) — 겹치는
/// 부분이 두 번 곱해지지 않는다. 라이브 레이어는 도형 여러 개로 같은 도형을 흉내내므로,
/// 그냥 그리면 반투명 획(형광펜)의 겹친 자리(캡/관절)가 **더 진해진다**. 그래서 자식 도형은
/// **불투명하게** 그리고 그룹 불투명도([`Self::opacity`])를 준다 — WinUI가 자식들을 먼저
/// 합성한 뒤 투명도를 한 번만 적용하므로 결과가 래스터와 같아진다.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveStroke {
    /// 잉크 색(알파 포함) — 자식 도형은 `with_alpha(255)`로 그린다.
    pub color: Rgba,
    pub lines: Vec<SurfaceLine>,
    pub caps: Vec<SurfaceCap>,
}

impl LiveStroke {
    /// 그릴 것이 없는가.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.caps.is_empty()
    }

    /// WinUI 도형 수(선분 + 캡) — 계측/상태바용.
    pub fn element_count(&self) -> usize {
        self.lines.len() + self.caps.len()
    }

    /// 그룹 불투명도(0.0~1.0).
    pub fn opacity(&self) -> f64 {
        self.color.a as f64 / 255.0
    }

    /// 자식 도형에 쓸 색 — 알파는 그룹이 맡으므로 불투명하다.
    pub fn opaque_color(&self) -> Rgba {
        self.color.with_alpha(255)
    }
}

/// 표면 서술 — 정적 레이어 두 장(더블 버퍼) + 라이브 획 + 크기.
#[derive(Clone, Debug, PartialEq)]
pub struct InkSurface {
    /// 확정 스트로크를 그린 PNG 두 장 — **보이는 레이어는 절대 비지 않는다**([`StaticLayers`]).
    pub layers: StaticLayers,
    /// 진행 중인 획 + 아직 PNG에 없는 꼬리(라이브 레이어) — **획마다 한 그룹**.
    pub ink: Vec<LiveStroke>,
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
        let mut layers = StaticLayers::default();
        layers.show_now(static_layer(committed, size, scale), committed.len());
        Self {
            layers,
            ink: live
                .map(|stroke| live_ink(stroke, scale))
                .into_iter()
                .collect(),
            width,
            height,
            scale,
        }
    }

    /// 확정 레이어 PNG만 다시 만든다 — 스트로크가 커밋된 직후 한 번(동기 경로).
    ///
    /// 호스트는 이걸 쓰지 않는다(워커가 [`static_layer`]를 만들고 [`StaticLayers::stage`]로
    /// 올린다). 디코드를 기다릴 수 없는 자리(테스트/초기 상태)를 위한 통로다.
    pub fn rebuild_static(&mut self, committed: &[Stroke], size: Size) {
        let png = static_layer(committed, size, self.scale);
        self.layers.show_now(png, committed.len());
    }

    pub fn is_empty(&self) -> bool {
        self.layers.visible_png().is_none() && self.ink.iter().all(LiveStroke::is_empty)
    }

    /// 라이브 레이어의 WinUI 도형 수(선분 + 캡).
    pub fn element_count(&self) -> usize {
        self.ink.iter().map(LiveStroke::element_count).sum()
    }

    /// 보이는 정적 PNG 바이트 크기(계측/로그).
    pub fn static_bytes(&self) -> usize {
        self.layers.visible_png().map(|png| png.len()).unwrap_or(0)
    }

    /// 보이는 정적 레이어를 디코드한다 — 호스트가 WinUI에 넘기기 전 확인용.
    pub fn decode_static(&self) -> Option<Pixmap> {
        self.layers
            .visible_png()
            .and_then(|bytes| raster::from_png(bytes).ok())
    }
}

/// 정적 레이어 한 장 — 화면에 올릴 PNG 한 벌과, 그 PNG가 반영한 확정 스트로크 수.
#[derive(Clone, Debug, Default, PartialEq)]
struct Layer {
    png: Option<Arc<[u8]>>,
    /// 이 PNG가 반영한 확정 스트로크 수 — 꼬리(라이브) 계산의 기준.
    count: usize,
}

/// 정적 레이어 **두 장** — WinUI `Image`의 깜빡임을 구조적으로 없애는 더블 버퍼.
///
/// ## 왜 두 장인가 (windows-reactor 0.100의 사실)
/// `Image.Source`를 바꾸면 어댑터는 **먼저 소스를 비우고**(`clear_property`) 그 다음
/// 비동기로 디코드한다(`SetSourceAsync` → 끝나면 `SetSource` + `ImageOpened`).
/// 그 사이 그 레이어는 화면에서 **통째로 사라진다** — 잉크가 매 획마다 번쩍이던 정체다.
///
/// ## 규칙 (계약)
/// - 새 PNG는 **보이지 않는 뒤 레이어**에 올린다([`Self::stage`]). 보이는 레이어는 그 동안
///   손대지 않는다 — 소스가 바뀌지 않으니 **비지 않는다**.
/// - 뒤 레이어의 디코드가 끝났다는 신호(`ImageOpened`)가 온 뒤에만 앞뒤를 바꾼다
///   ([`Self::promote`]). 그래서 화면에는 항상 **완성된 그림**만 올라온다.
/// - 뒤 레이어가 **이미 같은 그림**을 디코드해 두었으면 기다릴 필요가 없다
///   ([`Self::stage`]가 `true` → [`Self::promote_staged`]).
/// - 꼬리(라이브 도형)의 기준은 **보이는 레이어**가 반영한 스트로크 수다
///   ([`Self::visible_count`]) — 아직 화면에 없는 획은 라이브 레이어가 계속 그린다.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticLayers {
    layers: [Layer; 2],
    front: usize,
    /// 뒤 레이어에 올렸지만 아직 승격하지 못한 것: (레이어 인덱스, 올린 시각).
    ///
    /// 시각을 함께 두는 이유: 디코드 완료 신호가 오지 않는 경우(창 밖으로 밀어 둔
    /// 요소를 WinUI가 게으르게 다룰 때)를 대비해 [`Self::promote_if_settled`]가
    /// **충분히 기다렸는지** 판단해야 한다 — 그래야 새 스테이징을 성급히 승격하지 않는다.
    staged: Option<(usize, Instant)>,
}

impl Default for StaticLayers {
    fn default() -> Self {
        Self {
            layers: [Layer::default(), Layer::default()],
            front: 0,
            staged: None,
        }
    }
}

impl StaticLayers {
    /// 레이어 수 — 빌더가 이 수만큼 `Image`를 만든다.
    pub const COUNT: usize = 2;

    /// 화면에 보이는 레이어의 인덱스.
    pub fn front(&self) -> usize {
        self.front
    }

    /// 이 레이어가 화면에 보이는가 — 빌더는 **보이는 레이어만 페이지 크기**로 그린다
    /// (나머지는 0 크기 = 그려지지 않음. 소스는 디코드된 채 대기한다).
    pub fn is_visible(&self, index: usize) -> bool {
        index == self.front
    }

    /// 레이어 `index`의 PNG(없으면 빈 레이어).
    pub fn png(&self, index: usize) -> Option<&Arc<[u8]>> {
        self.layers.get(index).and_then(|layer| layer.png.as_ref())
    }

    /// 두 레이어의 PNG — 호스트가 표면 재료로 넘길 때 쓴다.
    pub fn pngs(&self) -> [Option<Arc<[u8]>>; Self::COUNT] {
        [self.layers[0].png.clone(), self.layers[1].png.clone()]
    }

    /// **보이는** 레이어의 PNG.
    pub fn visible_png(&self) -> Option<&Arc<[u8]>> {
        self.png(self.front)
    }

    /// 보이는 레이어가 반영한 확정 스트로크 수 — 꼬리(라이브) 계산의 기준.
    pub fn visible_count(&self) -> usize {
        self.layers[self.front].count
    }

    /// 뒤 레이어에 새 그림을 올린다 — **보이는 레이어는 그대로 둔다**.
    ///
    /// 돌려주는 값이 `true`면 뒤 레이어가 이미 **같은 그림**을 디코드해 두었다는 뜻이라
    /// 기다릴 필요가 없다([`Self::promote_staged`]를 바로 부르면 된다).
    pub fn stage(&mut self, png: Option<Arc<[u8]>>, count: usize) -> bool {
        let back = 1 - self.front;
        let same = same_bytes(self.layers[back].png.as_ref(), png.as_ref());
        self.layers[back] = Layer { png, count };
        self.staged = Some((back, Instant::now()));
        same
    }

    /// 스테이징된 레이어의 디코드가 끝났다 → 앞뒤를 바꾼다. 실제로 바뀌었으면 `true`.
    ///
    /// `index`가 스테이징된 레이어가 아니면 아무 일도 하지 않는다 — 낡은 신호는 무시한다.
    pub fn promote(&mut self, index: usize) -> bool {
        if self.staged.map(|(staged, _)| staged) != Some(index) || index == self.front {
            return false;
        }
        self.front = index;
        self.staged = None;
        true
    }

    /// 스테이징된 레이어를 **기다리지 않고** 승격한다(같은 그림이라 디코드가 필요 없을 때).
    pub fn promote_staged(&mut self) -> bool {
        match self.staged {
            Some((index, _)) => self.promote(index),
            None => false,
        }
    }

    /// 디코드 신호가 오지 않아도 **충분히 기다렸으면** 승격한다 — 안전망.
    ///
    /// 호스트가 워커에서 잠깐 기다렸다가 부른다. 그 사이에 새 그림이 스테이징됐다면
    /// (`staged`의 시각이 최근이면) 아무 일도 하지 않는다 — 갓 바뀐 소스는 아직
    /// 비어 있을 수 있으므로 **성급히 승격하면 그게 바로 깜빡임**이 된다.
    pub fn promote_if_settled(&mut self, settled: Duration) -> bool {
        match self.staged {
            Some((index, at)) if at.elapsed() >= settled => self.promote(index),
            _ => false,
        }
    }

    /// 스테이징을 버린다 — 화면(보이는 레이어)은 그대로 두고 "새 그림 없음"으로 되돌린다.
    pub fn cancel_staging(&mut self) {
        self.staged = None;
    }

    /// 디코드를 기다리지 않고 즉시 보여준다 — 초기 상태/테스트(동기 경로) 전용.
    pub fn show_now(&mut self, png: Option<Arc<[u8]>>, count: usize) {
        self.layers[self.front] = Layer { png, count };
        self.staged = None;
    }
}

/// 두 PNG가 **같은 그림**인가 — `Arc`가 같으면 바이트 비교도 하지 않는다.
fn same_bytes(left: Option<&Arc<[u8]>>, right: Option<&Arc<[u8]>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Arc::ptr_eq(left, right) || left == right,
        _ => false,
    }
}

/// 빈 레이어용 **1×1 투명 PNG** — 한 번만 만들어 재사용한다(`Arc`라 복사도 없다).
///
/// 왜 필요한가: 어댑터의 `Image::source_data`에는 **소스를 비우는 API가 없다**
/// (`Property::Inherited`는 "그대로 두기"다). 그래서 "빈 레이어"는 소스 없음이 아니라
/// **빈 그림**으로 표현한다 — 투명하니 화면에는 아무것도 보이지 않는다.
pub fn blank_png() -> Arc<[u8]> {
    static BLANK: OnceLock<Arc<[u8]>> = OnceLock::new();
    Arc::clone(BLANK.get_or_init(|| {
        raster::to_png(Pixmap::new(1, 1))
            .map(Arc::from)
            .unwrap_or_else(|_| Arc::from(Vec::new()))
    }))
}

/// 확정 스트로크 → 투명 배경 PNG.
///
/// 잉크가 없으면 `None`이다 — 빈 페이지에 1.13Mpx 이미지를 만들 이유가 없다.
pub fn static_layer(strokes: &[Stroke], size: Size, scale: f32) -> Option<Arc<[u8]>> {
    if !has_ink(strokes) {
        return None;
    }
    raster::to_png(raster::render_ink(strokes, size, scale))
        .ok()
        .map(Arc::from)
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

/// 라이브 레이어에 그릴 **획들** — **아직 정적 PNG에 없는** 확정 획 + 진행 중인 획.
///
/// `snapshot`은 화면에 있는 정적 PNG가 반영한 확정 스트로크 수다. 그 PNG는
/// 백그라운드에서 만들어지므로(모듈 문서의 계약), 렌더가 끝나기 전에 확정된 획은
/// PNG에 없다 — 그 획들을 라이브 도형으로 계속 그려 **빈 틈을 메운다**.
///
/// `snapshot >= committed.len()`이면 꼬리는 없다: 화면의 PNG를 믿지 않는 상태
/// (되돌리기/페이지 이동/줌 직후)를 호스트가 그렇게 표시한다.
pub fn pending_ink(
    committed: &[Stroke],
    snapshot: usize,
    live: Option<&Stroke>,
    scale: f32,
) -> Vec<LiveStroke> {
    let mut ink: Vec<LiveStroke> = committed[snapshot.min(committed.len())..]
        .iter()
        .map(|stroke| live_ink(stroke, scale))
        .collect();
    if let Some(stroke) = live {
        ink.push(live_ink(stroke, scale));
    }
    ink.retain(|stroke| !stroke.is_empty());
    ink
}

/// 진행 중인 획 하나 → 라이브 도형(선분 + 둥근 캡).
///
/// **래스터와 같은 도형**([`raster::ink_shape`] + [`raster::FLATTEN_TOLERANCE_PX`])을
/// 직선 조각으로 편다:
/// - 곡선(폭 일정) → 같은 중점 2차 베지어를 같은 오차로 펼친다([`raster::flatten_spans`]).
/// - 구간(폭 가변) → 구간 그대로, 양 끝은 관절을 덮도록 반지름만큼 늘린다
///   ([`raster::extended_span`]) — 래스터의 합집합과 **완전히 같은 도형**이 된다.
/// - 점 하나 → 둥근 캡 하나(채운 원).
///
/// 그래서 획을 확정해도(라이브 → PNG 승격) 잉크의 **모양과 색이 바뀌지 않는다**.
pub fn live_ink(stroke: &Stroke, scale: f32) -> LiveStroke {
    let color = stroke.style.color;
    let mut live = LiveStroke {
        color,
        lines: Vec::new(),
        caps: Vec::new(),
    };
    match raster::ink_shape(stroke, scale) {
        None => {}
        Some(InkShape::Dot { center, radius }) => live.caps.push(cap(center, radius, color)),
        Some(InkShape::Curve { width, path }) => live_spans(
            &mut live,
            &raster::flatten_spans(&path, width, raster::FLATTEN_TOLERANCE_PX),
        ),
        Some(InkShape::Spans { spans }) => live_spans(&mut live, &spans),
    }
    live
}

/// 구간 목록 → 선분 + 양 끝 캡 — 래스터의 합집합([`raster::spans_union`])과 **같은 규칙**.
fn live_spans(live: &mut LiveStroke, spans: &[InkSpan]) {
    let color = live.color;
    let last = spans.len().saturating_sub(1);
    live.lines = spans
        .iter()
        .enumerate()
        .map(|(index, span)| {
            let drawn = raster::extended_span(span, index > 0, index < last);
            SurfaceLine {
                x1: drawn.from.0,
                y1: drawn.from.1,
                x2: drawn.to.0,
                y2: drawn.to.1,
                width: span.width.max(0.5) as f64,
                color,
            }
        })
        .collect();
    if let (Some(first), Some(end)) = (spans.first(), spans.last()) {
        live.caps.push(cap(first.from, first.half_width(), color));
        live.caps.push(cap(end.to, end.half_width(), color));
    }
}

/// 둥근 캡 하나 — 래스터의 둥근 캡과 **같은 반지름**.
fn cap(center: (f64, f64), radius: f64, color: Rgba) -> SurfaceCap {
    SurfaceCap {
        x: center.0,
        y: center.1,
        radius: radius.max(0.35),
        color,
    }
}

//! 잉크 모델 — 색·도구·스타일·샘플·획. **순수 데이터 + 수학**만 있다(그리기 없음).
//!
//! 압력은 선택이다: WinUI `PointerEventInfo`에는 압력이 없어서 **속도로 만든다**
//! ([`pressure_from_speed`]). 모델은 두 경우를 구분하지 않는다.

use crate::geom::{distance_to_segment, Pt};

/// 화면에 보이는 도구 3종.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Pen,
    Highlighter,
    Eraser,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::Pen, Tool::Highlighter, Tool::Eraser];

    /// 화면 라벨 — UI와 테스트가 **같은 문자열**을 쓴다.
    pub const fn label(self) -> &'static str {
        match self {
            Tool::Pen => "펜",
            Tool::Highlighter => "형광펜",
            Tool::Eraser => "지우개",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.label() == label)
    }

    /// 지우개는 획을 그리지 않고 **없앤다**.
    pub const fn is_eraser(self) -> bool {
        matches!(self, Tool::Eraser)
    }
}

/// RGBA8 — **프리멀티플라이드가 아니다**(래스터라이저가 변환한다).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const INK_BLUE: Self = Self::new(23, 78, 166, 255);
    pub const HIGHLIGHT_YELLOW: Self = Self::new(255, 214, 0, 90);
    pub const RED: Self = Self::new(196, 43, 28, 255);

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 255)
    }

    pub const fn is_opaque(self) -> bool {
        self.a == 255
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// WinUI `Color::rgb`에 넘길 (r, g, b).
    pub const fn to_rgb(self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }
}

/// 획 스타일 — 폭은 pt라 **확대하면 화면에서도 같이 굵어진다**.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub color: Rgba,
    pub width_pt: f32,
}

impl Style {
    pub const PEN_WIDTH_PT: f32 = 1.8;
    pub const HIGHLIGHTER_WIDTH_PT: f32 = 14.0;
    pub const MIN_WIDTH_PT: f32 = 0.4;
    pub const MAX_WIDTH_PT: f32 = 64.0;

    pub fn new(color: Rgba, width_pt: f32) -> Self {
        Self {
            color,
            width_pt: width_pt.clamp(Self::MIN_WIDTH_PT, Self::MAX_WIDTH_PT),
        }
    }

    /// 도구별 기본 스타일(색·반투명은 도구가 정한다).
    pub fn for_tool(tool: Tool) -> Self {
        match tool {
            Tool::Pen => Self::new(Rgba::INK_BLUE, Self::PEN_WIDTH_PT),
            Tool::Highlighter => Self::new(Rgba::HIGHLIGHT_YELLOW, Self::HIGHLIGHTER_WIDTH_PT),
            Tool::Eraser => Self::new(Rgba::RED, Self::PEN_WIDTH_PT),
        }
    }

    /// 압력 → 이 샘플의 폭. 압력 1.0이면 온전한 폭, 0.0이면 45%.
    pub fn width_at(&self, pressure: f32) -> f32 {
        self.width_pt * (0.45 + 0.55 * pressure.clamp(0.0, 1.0))
    }

    /// 도구 안내 문구 — 상태바가 쓴다.
    pub const fn hint(tool: Tool) -> &'static str {
        match tool {
            Tool::Pen => "펜 — 빠르게 그으면 가늘어집니다",
            Tool::Highlighter => "형광펜 — 반투명으로 덧칠합니다",
            Tool::Eraser => "지우개 — 문지른 획을 통째로 지웁니다",
        }
    }
}
/// 필기 샘플 한 점.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InkPoint {
    pub pos: Pt,
    pub pressure: f32,
}

impl InkPoint {
    pub const DEFAULT_PRESSURE: f32 = 1.0;

    pub fn at(pos: Pt) -> Self {
        Self {
            pos,
            pressure: Self::DEFAULT_PRESSURE,
        }
    }

    pub fn new(pos: Pt, pressure: f32) -> Self {
        Self {
            pos,
            pressure: pressure.clamp(0.0, 1.0),
        }
    }
}

/// 최소 샘플 간격(pt) — 이보다 가깝고 압력도 같으면 버린다(모델을 가볍게).
pub const MIN_SAMPLE_DISTANCE_PT: f32 = 0.6;
/// 같은 자리에서 압력만 이만큼 바뀌면 받아들인다.
pub const PRESSURE_EPSILON: f32 = 0.05;

/// 속도 기반 압력 대체값(pt/ms) — 빠르면 가늘게(0.35), 느리면 굵게(1.0).
pub fn pressure_from_speed(pt_per_ms: f32) -> f32 {
    (1.0 - pt_per_ms / 2.5).clamp(0.35, 1.0)
}

/// 한 번의 획. 좌표는 pt, 순서는 입력 순서 그대로다.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub tool: Tool,
    pub style: Style,
    points: Vec<InkPoint>,
}

impl Stroke {
    pub fn new(tool: Tool, style: Style, first: InkPoint) -> Self {
        Self {
            tool,
            style,
            points: vec![first],
        }
    }

    pub fn points(&self) -> &[InkPoint] {
        &self.points
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// 점 하나만 찍은 획(탭)인가 — 래스터는 채운 원, 라이브는 둥근 캡 하나로 그린다.
    pub fn is_dot(&self) -> bool {
        self.points.len() <= 1
    }

    pub fn first(&self) -> Option<InkPoint> {
        self.points.first().copied()
    }

    pub fn last(&self) -> Option<InkPoint> {
        self.points.last().copied()
    }

    /// 샘플을 추가한다. 최소 간격보다 가깝고 압력도 같으면 버리고 `false`.
    ///
    /// **속도 기반 압력에서는 "가깝지만 압력이 다르다"가 흔하다** — 그래서 폭이 변하는
    /// 획(구간 스트로크)이 기본이고, 폭이 일정한 획만 곡선 하나로 그린다.
    pub fn push(&mut self, point: InkPoint) -> bool {
        match self.points.last().copied() {
            None => {
                self.points.push(point);
                true
            }
            Some(last) => {
                let close = point.pos.distance(last.pos) < MIN_SAMPLE_DISTANCE_PT;
                let same_pressure = (point.pressure - last.pressure).abs() < PRESSURE_EPSILON;
                if close && same_pressure {
                    return false;
                }
                self.points.push(point);
                true
            }
        }
    }

    pub fn max_width(&self) -> f32 {
        self.points
            .iter()
            .map(|p| self.style.width_at(p.pressure))
            .fold(0.0, f32::max)
    }

    /// 원과 교차하는가 — 지우개 히트 테스트.
    pub fn hits_circle(&self, center: Pt, radius: f32) -> bool {
        let Some(first) = self.first() else {
            return false;
        };
        let reach = radius + self.max_width() * 0.5;
        if self.points.len() == 1 {
            return first.pos.distance(center) <= reach;
        }
        self.points
            .windows(2)
            .any(|pair| distance_to_segment(center, pair[0].pos, pair[1].pos) <= reach)
    }
}

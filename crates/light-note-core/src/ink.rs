//! 필기 프리미티브 — 도구, 스타일, 샘플(압력 포함), 스트로크.
//!
//! 이 파일은 **순수 데이터 + 수학**만 갖는다(그리기/플랫폼 의존 없음).
//! 스트로크를 픽셀로 바꾸는 일은 [`crate::raster`], WinUI 도형으로 바꾸는 일은
//! [`crate::surface`]가 맡는다.

use crate::geom::{distance_to_segment, Bounds, Point};

/// 화면에 보이는 도구 3종.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Pen,
    Highlighter,
    Eraser,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::Pen, Tool::Highlighter, Tool::Eraser];

    /// 화면 라벨 — UI와 테스트가 **같은 문자열**을 쓰도록 여기서 한 번만 정의한다.
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

    /// 지우개는 스트로크를 그리지 않고 **없앤다**.
    pub const fn is_eraser(self) -> bool {
        matches!(self, Tool::Eraser)
    }

    pub const fn is_draw(self) -> bool {
        !self.is_eraser()
    }

    /// 상태바용 설명.
    pub const fn hint(self) -> &'static str {
        match self {
            Tool::Pen => "펜 — 필압에 따라 굵기가 변합니다",
            Tool::Highlighter => "형광펜 — 반투명으로 덧칠합니다",
            Tool::Eraser => "지우개 — 문지른 스트로크를 통째로 지웁니다",
        }
    }
}

/// RGBA8 색. **프리멀티플라이드가 아니다** — 래스터라이저가 변환한다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const BLACK: Self = Self::new(0, 0, 0, 255);
    pub const WHITE: Self = Self::new(255, 255, 255, 255);
    /// Windows 11 기본 잉크색(파랑).
    pub const INK_BLUE: Self = Self::new(23, 78, 166, 255);
    pub const HIGHLIGHT_YELLOW: Self = Self::new(255, 214, 0, 90);
    pub const RED: Self = Self::new(196, 43, 28, 255);

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::new(r, g, b, 255)
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    pub const fn is_opaque(self) -> bool {
        self.a == 255
    }

    /// WinUI `Color::rgb`에 넘길 (r, g, b).
    pub const fn to_rgb(self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }
}

/// 스트로크 스타일. 폭은 pt(모델 좌표)라 **확대하면 화면에서도 같이 굵어진다**.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeStyle {
    pub color: Rgba,
    pub width: f32,
}

impl StrokeStyle {
    pub const PEN_WIDTH_PT: f32 = 1.8;
    pub const HIGHLIGHTER_WIDTH_PT: f32 = 14.0;
    pub const MIN_WIDTH_PT: f32 = 0.4;
    pub const MAX_WIDTH_PT: f32 = 64.0;

    pub fn new(color: Rgba, width: f32) -> Self {
        Self {
            color,
            width: width.clamp(Self::MIN_WIDTH_PT, Self::MAX_WIDTH_PT),
        }
    }

    pub fn pen(color: Rgba, width: f32) -> Self {
        Self::new(color, width)
    }

    pub fn highlighter(color: Rgba, width: f32) -> Self {
        Self::new(color, width)
    }

    /// 압력 → 이 샘플의 폭. 압력 1.0이면 온전한 폭, 0.0이면 45%.
    pub fn width_at(&self, pressure: f32) -> f32 {
        self.width * (0.45 + 0.55 * pressure.clamp(0.0, 1.0))
    }

    /// 도구별 기본 스타일.
    pub fn for_tool(tool: Tool) -> Self {
        match tool {
            Tool::Pen => Self::pen(Rgba::INK_BLUE, Self::PEN_WIDTH_PT),
            Tool::Highlighter => {
                Self::highlighter(Rgba::HIGHLIGHT_YELLOW, Self::HIGHLIGHTER_WIDTH_PT)
            }
            Tool::Eraser => Self::pen(Rgba::RED, Self::PEN_WIDTH_PT),
        }
    }
}

/// 필기 샘플 한 점.
///
/// **압력은 선택**이다 — WinUI 어댑터의 `PointerEventInfo`에는 압력이 없으므로
/// 호스트는 [`pressure_from_speed`]로 대체값을 만들거나 `1.0`을 넣는다.
/// 모델은 두 경우를 구분하지 않는다(둘 다 0.0~1.0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InkPoint {
    pub pos: Point,
    pub pressure: f32,
}

impl InkPoint {
    pub const DEFAULT_PRESSURE: f32 = 1.0;

    pub fn new(x: f32, y: f32) -> Self {
        Self {
            pos: Point::new(x, y),
            pressure: Self::DEFAULT_PRESSURE,
        }
    }

    pub fn at(pos: Point) -> Self {
        Self {
            pos,
            pressure: Self::DEFAULT_PRESSURE,
        }
    }

    pub fn with_pressure(x: f32, y: f32, pressure: f32) -> Self {
        Self {
            pos: Point::new(x, y),
            pressure: pressure.clamp(0.0, 1.0),
        }
    }

    pub fn x(self) -> f32 {
        self.pos.x
    }

    pub fn y(self) -> f32 {
        self.pos.y
    }
}

/// 최소 샘플 간격(pt) — 이보다 가까운 샘플은 버려 모델을 가볍게 유지한다.
pub const MIN_SAMPLE_DISTANCE_PT: f32 = 0.6;
/// 같은 자리에서 압력만 이만큼 바뀌면 받아들인다.
pub const PRESSURE_EPSILON: f32 = 0.05;

/// 속도 기반 압력 대체값 (pt/ms).
///
/// 빠르게 그으면 가늘게(0.35), 느리면 굵게(1.0) — 펜 압력이 없는 입력에서도
/// "사람이 쓴 것 같은" 굵기 변화를 만든다.
pub fn pressure_from_speed(pt_per_ms: f32) -> f32 {
    (1.0 - pt_per_ms / 2.5).clamp(0.35, 1.0)
}

/// 한 번의 획. 좌표는 페이지 pt, 순서는 입력 순서 그대로다.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub tool: Tool,
    pub style: StrokeStyle,
    points: Vec<InkPoint>,
}

impl Stroke {
    pub fn new(tool: Tool, style: StrokeStyle, first: InkPoint) -> Self {
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

    /// 점 하나만 찍은 스트로크(탭)인가 — 래스터라이저가 원으로 그린다.
    pub fn is_dot(&self) -> bool {
        self.points.len() <= 1
    }

    pub fn first(&self) -> Option<InkPoint> {
        self.points.first().copied()
    }

    pub fn last(&self) -> Option<InkPoint> {
        self.points.last().copied()
    }

    /// 샘플을 추가한다. 최소 간격보다 가까우면 버리고 `false`.
    pub fn push(&mut self, point: InkPoint) -> bool {
        match self.points.last().copied() {
            None => {
                self.points.push(point);
                true
            }
            Some(last) => {
                let distance = point.pos.distance(last.pos);
                if distance < MIN_SAMPLE_DISTANCE_PT
                    && (point.pressure - last.pressure).abs() < PRESSURE_EPSILON
                {
                    return false;
                }
                self.points.push(point);
                true
            }
        }
    }

    pub fn push_xy(&mut self, x: f32, y: f32) -> bool {
        self.push(InkPoint::new(x, y))
    }

    /// 총 길이(pt).
    pub fn length(&self) -> f32 {
        self.points
            .windows(2)
            .map(|pair| pair[0].pos.distance(pair[1].pos))
            .sum()
    }

    /// 점 경계(폭 무시).
    pub fn point_bounds(&self) -> Option<Bounds> {
        Bounds::from_points(self.points.iter().map(|p| p.pos))
    }

    /// 잉크 경계(폭의 절반만큼 확장) — 더티 렉트와 히트 테스트에 쓴다.
    pub fn bounds(&self) -> Option<Bounds> {
        self.point_bounds().map(|b| b.expanded(self.max_width() * 0.5))
    }

    pub fn max_width(&self) -> f32 {
        self.points
            .iter()
            .map(|p| self.style.width_at(p.pressure))
            .fold(0.0, f32::max)
    }

    /// 원과 교차하는가 — 스트로크 지우개의 히트 테스트.
    pub fn hits_circle(&self, center: Point, radius: f32) -> bool {
        let Some(first) = self.points.first().copied() else {
            return false;
        };
        let hit_radius = radius + self.max_width() * 0.5;
        if self.points.len() == 1 {
            return first.pos.distance(center) <= hit_radius;
        }
        self.points
            .windows(2)
            .any(|pair| distance_to_segment(center, pair[0].pos, pair[1].pos) <= hit_radius)
    }

    pub fn hits_point(&self, point: Point, radius: f32) -> bool {
        self.hits_circle(point, radius)
    }

    /// 상태바/로그용 요약.
    pub fn describe(&self) -> String {
        format!(
            "{} · {}점 · {:.0}pt · 폭 {:.1}pt",
            self.tool.label(),
            self.len(),
            self.length(),
            self.style.width
        )
    }
}

//! 좌표계 — **pt(모델)** 와 **픽셀(표면 DIP)** 두 단위만 쓴다.
//!
//! - 모델(획 좌표, 페이지 크기)은 pt다. 화면 배율이 바뀌어도 잉크는 변하지 않는다.
//! - 표면은 픽셀 = DIP다(150% 배율의 Windows 11에서 1pt = 1.5 DIP).
//! - 둘을 잇는 값은 [`Scale`] **하나**뿐이다.

/// 페이지 위의 한 점(pt) — 원점은 **좌상단**(화면 좌표계).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

/// 페이지 크기(pt).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Pt {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn distance(self, other: Self) -> f32 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// A4(595×842pt) — 새 문서의 기본 페이지.
    pub const A4: Self = Self::new(595.0, 842.0);
}

/// pt → 픽셀 변환. `scale` = 1pt당 픽셀 수(1.5 = Windows 11 기본 배율).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale(f32);

impl Default for Scale {
    fn default() -> Self {
        Self::new(Self::DEFAULT)
    }
}

impl Scale {
    /// Windows 11의 150% 배율.
    pub const DEFAULT: f32 = 1.5;

    pub fn new(scale: f32) -> Self {
        Self(scale.clamp(0.1, 16.0))
    }

    /// 줌(%)을 곱한 배율 — `zoom = 100`이면 기본 배율.
    pub fn from_zoom(zoom: f32) -> Self {
        Self::new(Self::DEFAULT * zoom / 100.0)
    }

    pub fn get(self) -> f32 {
        self.0
    }

    /// 페이지 크기(pt) → 픽셀(최소 1).
    pub fn pixels(self, size: Size) -> (u16, u16) {
        let to = |value: f32| (value * self.0).round().clamp(1.0, u16::MAX as f32) as u16;
        (to(size.width), to(size.height))
    }

    /// 표면 DIP → pt.
    pub fn to_pt(self, x: f32, y: f32) -> Pt {
        Pt::new(x / self.0, y / self.0)
    }
}

/// 점 `p`에서 선분 `a-b`까지의 최단 거리 — 지우개 히트 테스트의 기준.
pub fn distance_to_segment(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f32::EPSILON {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / length_squared).clamp(0.0, 1.0);
    p.distance(Pt::new(a.x + dx * t, a.y + dy * t))
}

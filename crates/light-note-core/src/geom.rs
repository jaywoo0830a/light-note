//! 기하 기본형 — 좌표계 계약은 이 파일 하나에 모아 둔다.
//!
//! **계약**: 모델 좌표의 단위는 PDF 사용자 공간과 같은 **pt(1/72인치)** 이고,
//! **원점은 페이지 좌상단, y는 아래로 증가**한다. PDF의 좌하단 원점은
//! [`crate::pdf`]가 변환하고, 화면(px) 변환은 [`crate::raster::ViewTransform`]만 한다.

use std::ops::{Add, Sub};

/// 페이지 좌표의 한 점 (pt, 좌상단 원점).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ORIGIN: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    pub fn distance(self, other: Self) -> f32 {
        (self - other).length()
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }
}

impl Add for Point {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Point {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl From<(f32, f32)> for Point {
    fn from(value: (f32, f32)) -> Self {
        Self::new(value.0, value.1)
    }
}

/// 페이지/캔버스 크기 (pt).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    /// A4 (210×297mm).
    pub const A4: Self = Self {
        width: 595.28,
        height: 841.89,
    };
    /// US Letter.
    pub const LETTER: Self = Self {
        width: 612.0,
        height: 792.0,
    };

    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub fn area(self) -> f32 {
        self.width * self.height
    }

    pub fn is_empty(self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    pub fn aspect(self) -> f32 {
        if self.height == 0.0 {
            0.0
        } else {
            self.width / self.height
        }
    }

    /// `max` 상자 안에 비율을 유지한 채 들어가는 크기.
    ///
    /// 창 크기가 바뀔 때 "페이지를 창에 맞춘다"가 이 한 줄이 된다.
    pub fn fit_into(self, max: Self) -> Self {
        if self.is_empty() || max.is_empty() {
            return Self::new(0.0, 0.0);
        }
        let scale = (max.width / self.width).min(max.height / self.height);
        Self::new(self.width * scale, self.height * scale)
    }
}

impl Default for Size {
    fn default() -> Self {
        Self::A4
    }
}

/// 축 정렬 경계 상자. 항상 `min <= max`로 정규화된다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: Point,
    pub max: Point,
}

impl Bounds {
    pub fn new(a: Point, b: Point) -> Self {
        Self {
            min: Point::new(a.x.min(b.x), a.y.min(b.y)),
            max: Point::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }

    pub fn from_points(points: impl IntoIterator<Item = Point>) -> Option<Self> {
        let mut iter = points.into_iter();
        let first = iter.next()?;
        let mut bounds = Self {
            min: first,
            max: first,
        };
        for point in iter {
            bounds = bounds.union_point(point);
        }
        Some(bounds)
    }

    pub fn union_point(self, point: Point) -> Self {
        self.union(Self {
            min: point,
            max: point,
        })
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min: Point::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Point::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }

    pub fn expanded(self, pad: f32) -> Self {
        Self {
            min: Point::new(self.min.x - pad, self.min.y - pad),
            max: Point::new(self.max.x + pad, self.max.y + pad),
        }
    }

    pub fn width(self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(self) -> f32 {
        self.max.y - self.min.y
    }

    pub fn center(self) -> Point {
        Point::new(
            (self.min.x + self.max.x) * 0.5,
            (self.min.y + self.max.y) * 0.5,
        )
    }

    pub fn contains(self, point: Point) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }

    pub fn intersects(self, other: Self) -> bool {
        self.min.x <= other.max.x
            && other.min.x <= self.max.x
            && self.min.y <= other.max.y
            && other.min.y <= self.max.y
    }
}

/// 점 `p`에서 선분 `a-b`까지의 최단 거리 — 지우개 히트 테스트의 기준.
pub fn distance_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let ab = b - a;
    let len_sq = ab.x * ab.x + ab.y * ab.y;
    if len_sq <= f32::EPSILON {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / len_sq).clamp(0.0, 1.0);
    p.distance(a.lerp(b, t))
}

//! 태블릿 좌표 → 페이지 좌표(pt) — **비율을 지키는 맞춤**.
//!
//! ## 왜 맞춤(fit)인가
//! 태블릿의 가로/세로 비와 A4의 비는 다르다(실측: 51196/31826 = 1.609, A4 = 0.707). 늘려 맞추면
//! **원이 타원이 된다** — 필기에서 그건 종이의 성질이 아니라 버그다. 그래서 태블릿 전체를
//! 페이지 안에 **비율 그대로** 넣고 남는 자리는 여백으로 둔다(가운데 정렬).
//!
//! ## 단위는 장치 단위 그대로 쓴다
//! `MaxX`/`MaxY`는 밀리미터가 아니라 장치 단위지만, **두 축의 비는 같다**(1.609 = 255.98/159.13).
//! 비율만 쓰므로 환산이 필요 없다 — 숫자를 하나 덜 믿는 쪽이 낫다.

use crate::geom::{Pt, Size};
use crate::otd::wire::TabletSpec;

/// 태블릿 → 페이지 변환기. **페이지 크기나 태블릿이 바뀌면 다시 만든다.**
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mapper {
    /// 태블릿 단위 → pt.
    scale: f32,
    /// 맞춤 뒤 남는 여백의 절반(가운데 정렬).
    offset: (f32, f32),
    max_pressure: f32,
}

impl Mapper {
    /// 태블릿 전체를 페이지 안에 **비율 그대로** 넣는다.
    pub fn fit(tablet: &TabletSpec, page: Size) -> Self {
        let scale = (page.width / tablet.max_x).min(page.height / tablet.max_y);
        let used = (tablet.max_x * scale, tablet.max_y * scale);
        Self {
            scale,
            offset: ((page.width - used.0) / 2.0, (page.height - used.1) / 2.0),
            max_pressure: tablet.max_pressure.max(1.0),
        }
    }

    /// 장치 좌표 → 페이지 위의 점(pt).
    pub fn to_page(&self, x: f32, y: f32) -> Pt {
        Pt::new(
            x * self.scale + self.offset.0,
            y * self.scale + self.offset.1,
        )
    }

    /// 원시 필압 → 0.0~1.0. **장치가 보고한 최대값이 기준**이다(16383은 16384가 아니다).
    ///
    /// 원시값이 `f32`인 이유: 공유 메모리 계약이 필압을 float로 쓰고, RPC도 소수점을
    /// 버릴 이유가 없다(굳이 u32로 왕복하면 반올림이 한 번 더 생긴다).
    pub fn pressure(&self, raw: f32) -> f32 {
        (raw / self.max_pressure).clamp(0.0, 1.0)
    }

    /// 페이지가 얼마나 쓰이는가(진단·테스트용).
    pub fn scale(&self) -> f32 {
        self.scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deco() -> TabletSpec {
        TabletSpec {
            name: "XP-Pen Deco 01 V3 (Variant 2)".to_string(),
            max_x: 51196.0,
            max_y: 31826.0,
            max_pressure: 16383.0,
        }
    }

    #[test]
    fn the_aspect_ratio_survives_the_mapping() {
        // 태블릿에서 정사각형이면 화면에서도 정사각형이어야 한다(원이 타원이 되면 버그다).
        let mapper = Mapper::fit(&deco(), Size::A4);
        let a = mapper.to_page(0.0, 0.0);
        let b = mapper.to_page(1000.0, 0.0);
        let c = mapper.to_page(0.0, 1000.0);
        assert!(
            (a.distance(b) - a.distance(c)).abs() < 0.01,
            "{a:?} {b:?} {c:?}"
        );
    }

    #[test]
    fn the_tablet_is_centred_on_the_page() {
        // 태블릿(1.609)이 A4(0.707)보다 가로로 길다 → **가로가 꽉 차고** 세로가 여백이다.
        let mapper = Mapper::fit(&deco(), Size::A4);
        let left = mapper.to_page(0.0, 0.0).x;
        let right = Size::A4.width - mapper.to_page(51196.0, 0.0).x;
        assert!(
            left.abs() < 0.01 && (left - right).abs() < 0.01,
            "{left} {right}"
        );

        let top = mapper.to_page(0.0, 0.0).y;
        let bottom = Size::A4.height - mapper.to_page(0.0, 31826.0).y;
        assert!(top > 0.0, "여백이 있어야 한다(A4는 태블릿보다 세로가 길다)");
        assert!((top - bottom).abs() < 0.01, "{top} {bottom}");
    }

    #[test]
    fn the_extremes_land_on_the_page() {
        let mapper = Mapper::fit(&deco(), Size::A4);
        let corner = mapper.to_page(51196.0, 31826.0);
        assert!(corner.x <= Size::A4.width + 0.01 && corner.y <= Size::A4.height + 0.01);
    }

    #[test]
    fn pressure_uses_the_device_maximum() {
        let mapper = Mapper::fit(&deco(), Size::A4);
        assert_eq!(mapper.pressure(0.0), 0.0);
        assert!((mapper.pressure(16383.0) - 1.0).abs() < 0.0001);
        assert!((mapper.pressure(8191.5) - 0.5).abs() < 0.001);
        // 범위 밖은 자른다(장치가 이상한 값을 보내도 잉크 폭은 0~1이다).
        assert_eq!(mapper.pressure(999_999.0), 1.0);
    }

    #[test]
    fn a_smaller_page_still_fits() {
        // 창이 좁아 페이지가 작아져도 같은 규칙이다(창 크기에 따라 필기 위치가 흔들리지 않는다).
        let mapper = Mapper::fit(&deco(), Size::new(100.0, 100.0));
        let corner = mapper.to_page(51196.0, 31826.0);
        assert!(corner.x <= 100.01 && corner.y <= 100.01);
    }
}

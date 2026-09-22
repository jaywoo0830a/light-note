//! OTD 리포트 → 앱이 쓰는 값. **파싱은 serde가 한다.**
//!
//! ## 왜 serde인가 (직접 스캔을 버린 이유)
//! 이 파일은 한때 필요한 키만 골라 읽는 손수 만든 스캐너였다. 지금은 serde다:
//!
//! - **지연의 핵심 경로가 여기가 아니다.** 실제 필기는 공유 메모리(`OTD.SharedMemoryOutput`
//!   플러그인 + `otd::shm`)로 오고, JSON을 **아예 쓰지 않는다**. 이 파일은 플러그인이 없을 때의
//!   **폴백**(OTD RPC `DeviceReport` 스트림)만 담당한다.
//! - 폴백에서는 **정확·단순**이 이긴다. 손수 만든 스캐너는 이스케이프·중첩·따옴표 경계에서
//!   조용히 틀릴 수 있고, 틀리면 화면이 아니라 필기가 이상해진다.
//! - 비용은 감당 가능하다: 1 KB JSON 파싱이 대략 2~5 µs이고, 400 Hz의 표본 간격은 2.5 ms다
//!   (0.2%). 공유 메모리 경로는 이 비용을 아예 내지 않는다.
//!
//! ## OTD JSON의 모양 (실측)
//! ```text
//! {"jsonrpc":"2.0","method":"DeviceReport","params":[{
//!    "Tablet":{…"Specifications":{"Digitizer":{"MaxX":51196,"MaxY":31826},
//!              "Pen":{"MaxPressure":16383}}…},          ← 리포트마다 반복되는 약 1 KB
//!    "Path":"…XP_PenTabletOverflowReport",
//!    "Data":{"Position":{"X":28388.0,"Y":10286.0},"Pressure":0,
//!            "PenButtons":[false,false],"Tilt":{"X":-1.0,"Y":-1.0},"Eraser":false}}]}
//! ```
//! 속성 이름은 .NET 기본 직렬화 그대로 **PascalCase**다(`Position`, `Pressure`, `PenButtons`).
//! 그래서 구조체는 `rename_all = "PascalCase"`로 받는다 — 이름을 손으로 적지 않는다.
//!
//! **모르는 필드는 무시된다**(serde 기본). OTD가 필드를 더해도 이 파일은 안 깨진다.
//! 테스트 재료는 이 PC에서 실제로 받은 페이로드다(`tests/fixtures/otd/*.json`).

use serde::Deserialize;

/// 태블릿이 무엇을 보냈는가 — **획의 자격과 끝**을 정하는 종류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 펜 리포트 — 좌표·필압이 들어 있다(`Data.Position`이 있는 유일한 종류다).
    Pen,
    /// 펜이 태블릿 **범위를 벗어났다** — 진행 중인 획을 끝내는 신호다.
    OutOfRange,
    /// 보조 키(익스프레스 키) — 지금은 쓰지 않는다.
    Aux,
    /// 모르는 리포트 — 무시한다.
    Other,
}

/// 리포트 한 개 — **화면이 쓰는 것만**.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Report {
    pub kind: Kind,
    /// 태블릿 좌표(장치 단위, `TabletSpec::max_x`/`max_y`가 최대).
    pub x: f32,
    pub y: f32,
    /// 원시 필압(0 = 안 눌림, 최대는 `TabletSpec::max_pressure`).
    pub pressure: u32,
    /// 틸트(도) — 장치가 보고한 경우에만.
    pub tilt: Option<(f32, f32)>,
    /// 펜을 뒤집었는가(지우개 끝).
    pub eraser: bool,
    /// 펜 옆 버튼 — 비트 0이 첫 버튼.
    pub buttons: u8,
}

impl Report {
    /// 펜이 **화면에 닿았는가** — OTD의 `TipActivationThreshold`가 아니라 우리 규칙이다:
    /// 실측에서 호버는 `Pressure:0`, 접촉은 0보다 큰 값이었다.
    pub fn is_tip_down(self) -> bool {
        self.kind == Kind::Pen && self.pressure > 0
    }
}

/// 태블릿의 **범위** — 좌표를 페이지로 옮기는 유일한 근거(`GetTablets`에서 한 번 읽는다).
#[derive(Clone, Debug, PartialEq)]
pub struct TabletSpec {
    pub name: String,
    pub max_x: f32,
    pub max_y: f32,
    pub max_pressure: f32,
}

/// `DeviceReport` 알림 하나 → 리포트. 다른 알림(`Message` 등)이면 `None`.
pub fn report(body: &[u8]) -> Option<Report> {
    let notification: Notification = serde_json::from_slice(body).ok()?;
    if notification.method != "DeviceReport" {
        return None;
    }
    let params = notification.params.first()?;
    let data = params.data.as_ref();

    // **`Position`이 있으면 펜 리포트다**: 파서마다 클래스 이름이 다르므로 `Path`로 판단하지
    // 않는다(그 판단이 XP-Pen·Wacom·Huion마다 달라지는 곳이다).
    if let Some(position) = data.and_then(|data| data.position) {
        return Some(Report {
            kind: Kind::Pen,
            x: position.x,
            y: position.y,
            pressure: data.map_or(0.0, |data| data.pressure).max(0.0) as u32,
            tilt: data.and_then(|data| data.tilt).and_then(reported_tilt),
            eraser: data.is_some_and(|data| data.eraser),
            buttons: data.map_or(0, |data| button_bits(&data.pen_buttons)),
        });
    }

    // 좌표가 없으면 종류만 말한다 — 진행 중인 획을 끝낼지 결정하는 데는 그거면 된다.
    let path = params.path.unwrap_or_default();
    let kind = if path.ends_with("OutOfRangeReport") {
        Kind::OutOfRange
    } else if path.ends_with("AuxReport") {
        Kind::Aux
    } else {
        Kind::Other
    };
    Some(Report {
        kind,
        x: 0.0,
        y: 0.0,
        pressure: 0,
        tilt: None,
        eraser: false,
        buttons: 0,
    })
}

/// 틸트 — **장치가 보고하지 않은 값은 `None`**이다.
///
/// 실측: 이 장치는 안 보고할 때 `Tilt:{"X":-1.0,"Y":-1.0}`을 보낸다(각도 -1도가 아니라 표식).
/// 두 축이 모두 -1이면 "안 보냄"으로 읽는다 — 1도짜리 틸트를 잃는 대신 거짓 값을 만들지 않는다.
fn reported_tilt(tilt: Position) -> Option<(f32, f32)> {
    if tilt.x == -1.0 && tilt.y == -1.0 {
        return None;
    }
    Some((tilt.x, tilt.y))
}

/// 펜 옆 버튼 배열 → 비트 묶음(첫 버튼이 비트 0). 8개를 넘는 것은 버린다(비트가 모자란다).
fn button_bits(buttons: &[bool]) -> u8 {
    let mut bits = 0u8;
    for (index, pressed) in buttons.iter().take(8).enumerate() {
        if *pressed {
            bits |= 1 << index;
        }
    }
    bits
}

// ── serde 구조체: OTD가 실제로 보내는 이름 그대로 ─────────────────

/// JSON-RPC 알림 봉투. `params`는 알림마다 모양이 다르므로 `DeviceReport` 것만 안다.
#[derive(Deserialize)]
struct Notification<'a> {
    #[serde(borrow)]
    method: &'a str,
    #[serde(borrow, default)]
    params: Vec<ReportParams<'a>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ReportParams<'a> {
    /// 파서 클래스 이름 — 종류 판단에만 쓴다(좌표 판단은 `Data.Position`으로 한다).
    #[serde(borrow)]
    path: Option<&'a str>,
    data: Option<ReportData>,
}

/// `Data` — 리포트의 실제 값. 없는 필드는 기본값이다(필압 없는 리포트가 있다).
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ReportData {
    position: Option<Position>,
    #[serde(default)]
    pressure: f32,
    tilt: Option<Position>,
    #[serde(default)]
    eraser: bool,
    #[serde(default)]
    pen_buttons: Vec<bool>,
}

/// `{"X":..,"Y":..}` — 위치와 틸트가 같은 모양을 쓴다.
#[derive(Clone, Copy, Deserialize)]
struct Position {
    #[serde(rename = "X")]
    x: f32,
    #[serde(rename = "Y")]
    y: f32,
}

/// `GetTablets` 응답 → 태블릿 하나의 범위. 태블릿이 없으면 `None`.
///
/// **첫 태블릿만** 본다: 이 앱은 태블릿 하나로 필기한다(여러 개를 고르는 UI는 없다).
pub fn tablet(body: &[u8]) -> Option<TabletSpec> {
    let response: TabletsResponse = serde_json::from_slice(body).ok()?;
    let tablet = response.result.first()?;
    let specifications = &tablet.properties.specifications;
    let digitizer = &specifications.digitizer;
    if digitizer.max_x <= 0.0 || digitizer.max_y <= 0.0 {
        return None;
    }
    Some(TabletSpec {
        name: tablet.properties.name.clone(),
        max_x: digitizer.max_x,
        max_y: digitizer.max_y,
        // 압력을 보고하지 않는 장치를 0으로 나누지 않는다.
        max_pressure: if specifications.pen.max_pressure > 0.0 {
            specifications.pen.max_pressure
        } else {
            1.0
        },
    })
}

/// `GetTablets` 응답. 목록이 비어 있으면 "태블릿이 없다"는 뜻이다.
///
/// **봉투(`result`)에는 `PascalCase` 규칙을 쓰지 않는다**: JSON-RPC 봉투는 소문자이고,
/// 태블릿 정보(`Properties`…)만 .NET 기본 직렬화의 PascalCase다.
#[derive(Deserialize)]
struct TabletsResponse {
    #[serde(default)]
    result: Vec<TabletEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TabletEntry {
    properties: TabletProperties,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TabletProperties {
    name: String,
    specifications: Specifications,
}

/// 이 중 `Digitizer`(범위)와 `Pen`(필압)만 쓴다 — `AuxiliaryButtons` 등은 무시된다.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Specifications {
    digitizer: DigitizerSpec,
    pen: PenSpec,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DigitizerSpec {
    max_x: f32,
    max_y: f32,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PenSpec {
    max_pressure: f32,
}

// ── 테스트 (실측 페이로드가 재료다) ──────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// **실측 그대로** — 이 PC에서 실제로 받은 페이로드 파일이다(잘라내지 않는다).
    /// raw 문자열 대신 파일을 쓰는 이유: 이스케이프 사고가 원천적으로 불가능해진다.
    const REAL: &str = include_str!("../../tests/fixtures/otd/device_report.json");
    const TABLETS: &str = include_str!("../../tests/fixtures/otd/get_tablets.json");

    #[test]
    fn a_real_pen_report_is_read_without_touching_the_tablet_blob() {
        // 매 리포트에 따라오는 1 KB짜리 `Tablet`은 파싱되지 않는다(모르는 필드는 버린다).
        let report = report(REAL.as_bytes()).expect("펜 리포트");
        assert_eq!(report.kind, Kind::Pen);
        assert_eq!(report.x, 28388.0);
        assert_eq!(report.y, 10286.0);
        assert_eq!(report.pressure, 0, "호버는 0이다");
        assert!(!report.eraser);
        assert_eq!(report.buttons, 0);
        assert_eq!(report.tilt, None, "-1/-1은 '안 보냄'이다");
        assert!(!report.is_tip_down(), "안 누른 펜은 획이 아니다");
    }

    #[test]
    fn the_tip_is_down_when_pressure_arrives() {
        // 접촉은 필압으로 온다 — OTD의 TipActivationThreshold를 흉내내지 않는다.
        let down = REAL.replace("\"Pressure\":0", "\"Pressure\":8192");
        let report = report(down.as_bytes()).expect("펜 리포트");
        assert!(report.is_tip_down());
        assert_eq!(report.pressure, 8192);
    }

    #[test]
    fn a_reported_tilt_is_kept() {
        let tilted = REAL.replace(
            "\"Tilt\":{\"X\":-1.0,\"Y\":-1.0}",
            "\"Tilt\":{\"X\":12.5,\"Y\":-30.0}",
        );
        assert_eq!(
            report(tilted.as_bytes()).and_then(|report| report.tilt),
            Some((12.5, -30.0))
        );
    }

    #[test]
    fn the_eraser_and_barrel_buttons_come_through() {
        let erased = REAL.replace("\"Eraser\":false", "\"Eraser\":true").replace(
            "\"PenButtons\":[false,false]",
            "\"PenButtons\":[true,false]",
        );
        let report = report(erased.as_bytes()).expect("펜 리포트");
        assert!(report.eraser);
        assert_eq!(report.buttons, 0b01, "첫 버튼이 비트 0이다");
    }

    #[test]
    fn other_notifications_are_not_reports() {
        // 로그 알림도 `params`가 있다 — 그것을 리포트로 읽으면 안 된다.
        let log = "{\"jsonrpc\":\"2.0\",\"method\":\"Message\",\"params\":[{\"Group\":\"IPC\"}]}";
        assert_eq!(report(log.as_bytes()), None);
        // 응답(`id`가 있는 것)도 리포트가 아니다.
        assert_eq!(report(br#"{"jsonrpc":"2.0","id":2,"result":null}"#), None);
        // JSON이 아니면 조용히 `None`이다(깨진 스트림이 앱을 멈추게 두지 않는다).
        assert_eq!(report(b"Content-Length: 12"), None);
    }

    #[test]
    fn leaving_the_tablet_is_reported_as_out_of_range() {
        // 범위를 벗어나면 좌표가 없다 — 종류만 말한다(진행 중인 획을 끝내는 신호다).
        let leaving = "{\"jsonrpc\":\"2.0\",\"method\":\"DeviceReport\",\"params\":[{\"Path\":\"OpenTabletDriver.Configurations.Parsers.XP_Pen.XP_PenOutOfRangeReport\",\"Data\":{}}]}";
        assert_eq!(
            report(leaving.as_bytes()).map(|report| report.kind),
            Some(Kind::OutOfRange)
        );
        // 보조 키도 같은 길로 온다.
        let aux = "{\"jsonrpc\":\"2.0\",\"method\":\"DeviceReport\",\"params\":[{\"Path\":\"...AuxReport\",\"Data\":{\"Buttons\":[true,false]}}]}";
        assert_eq!(
            report(aux.as_bytes()).map(|report| report.kind),
            Some(Kind::Aux)
        );
    }

    #[test]
    fn the_tablet_range_comes_from_get_tablets() {
        let spec = tablet(TABLETS.as_bytes()).expect("태블릿");
        assert_eq!(spec.name, "XP-Pen Deco 01 V3 (Variant 2)");
        assert_eq!(spec.max_x, 51196.0);
        assert_eq!(spec.max_y, 31826.0);
        assert_eq!(spec.max_pressure, 16383.0);
        // 태블릿이 없으면 `None`이다 — 없는 범위로 좌표를 만들지 않는다.
        assert_eq!(tablet(br#"{"jsonrpc":"2.0","id":1,"result":[]}"#), None);
    }

    #[test]
    fn a_device_without_pressure_does_not_divide_by_zero() {
        let spec = tablet(
            TABLETS
                .replace("\"MaxPressure\":16383", "\"MaxPressure\":0")
                .as_bytes(),
        );
        assert_eq!(spec.map(|spec| spec.max_pressure), Some(1.0));
    }
}

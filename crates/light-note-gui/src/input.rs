//! ① HardwareInput — **태블릿이 보낸 펜 표본**을 모델이 아는 값으로 바꾼다.
//!
//! 이 단계가 아는 것은 넷이다: 어떤 위상인가, 페이지 어디인가, 지금이 언제인가,
//! 그리고 **어떤 장치가 보냈는가**. 도구도 잉크도 모른다 — 그래서 장치 분류 하나와
//! 표본의 모양 하나만 책임진다.
//!
//! ```text
//! OTD 플러그인(공유 메모리) ── otd::shm ── otd::feed ── Sample ──▶ ② CanvasTool
//!                            (링 읽기)   (위상·페이지 매핑)
//! ```
//!
//! ## 이 앱은 **디지타이저(펜)만** 필기한다
//! 드로잉 패드 친화 필기 앱이라 표본은 **장치를 들고 온다**: 펜이면 잉크가 되고,
//! 손가락·마우스는 아무것도 만들지 않는다([`Sample::is_ink`]). 표본을 만드는 곳이 OTD
//! 어댑터 하나뿐이라([`crate::otd::feed`]) **마우스 표본은 아예 생기지 않는다** — 그래도
//! 판정(`Device`)은 남겨 둔다: 다른 입력원이 붙어도 정책이 흔들리지 않게.
//!
//! ## 취소는 어디서 오는가
//! WinUI 이벤트를 읽던 시절에는 포인터 캡처 상실과 시스템 취소가 두 경로로 왔다. 지금은
//! 표본 흐름이 **끊기는 것**(플러그인이 사라짐·앱 종료)이 취소다 — [`crate::otd::feed`]의
//! `finish`가 [`Phase::Canceled`] 하나로 모은다.

use std::time::Instant;

use crate::geom::Pt;

/// 포인터 위상 — ①과 ②의 계약.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pressed,
    Moved,
    Released,
    Canceled,
}

impl Phase {
    /// 제스처가 끝나는 위상인가(②가 커밋하거나 버린다).
    pub const fn is_end(self) -> bool {
        matches!(self, Phase::Released | Phase::Canceled)
    }
}

/// 입력 장치 — **잉크를 만들 자격**을 정한다(이 앱은 디지타이저만 필기한다).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    /// 펜(디지타이저) — 필기할 수 있는 **유일한** 장치.
    Pen,
    /// 손가락 — 화면을 만질 수는 있지만 잉크는 만들지 않는다.
    Touch,
    /// 마우스(또는 장치를 모르는 입력) — 잉크는 만들지 않는다.
    Mouse,
}

impl Device {
    /// 상태바가 읽는 이름 — **사용자가 읽는 문자열**이다.
    pub const fn label(self) -> &'static str {
        match self {
            Device::Pen => "Pen",
            Device::Touch => "Touch",
            Device::Mouse => "Mouse",
        }
    }
}

/// 하드웨어가 알려 준 **한 포인터 프레임**(장치·필압·틸트·**펜의 자세**) — [`crate::otd::feed`]가 채운다.
///
/// Win32 `POINTER_PEN_INFO`의 계약을 그대로 옮긴다(공식 문서의 범위와 자격 규칙):
///
/// | 필드 | 문서 | 우리 쪽 |
/// |---|---|---|
/// | `pressure` | 0~1024, **`PEN_MASK_PRESSURE`일 때만 유효**(없으면 기본 0) | [`pressure_from_raw`] → 0.0~1.0 |
/// | `tiltX`/`tiltY` | −90~+90(오른쪽·사용자 쪽이 +) | [`tilt_from_raw`] → 도, 두 축이 **둘 다** 있을 때만 `Some` |
/// | `rotation` | 0~359도(시계 방향), **`PEN_MASK_ROTATION`일 때만 유효** | [`rotation_from_raw`] → 도, 진단에만 쓴다 |
/// | `penFlags` | `PEN_FLAG_*`의 조합(0 가능) | [`PointerFrame::with_pen_pose`] → [`PointerFrame::inverted`] / [`PointerFrame::has_eraser`] |
///
/// **장치가 보고하지 않은 값은 `None`이다** — 0으로 채우면 "압력 0"과 "압력을 안 보내는
/// 장치"를 구분할 수 없다(그래서 문서가 말하는 "기본 0"을 그대로 쓰지 않는다).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerFrame {
    pub device: Device,
    pub pressure: Option<f32>,
    pub tilt: Option<(f32, f32)>,
    /// 펜을 **뒤집어** 쓰는 중인가(`PEN_FLAG_INVERTED`) — 지우개 끝이다.
    ///
    /// 이 한 점이 획의 성격을 바꾼다([`crate::tool::CanvasTool::press_inverted`]): 뒤집힌 펜은
    /// **지운다**. 도구 선택을 바꾸지 않고 그 제스처만 지운다 — 뒤집기를 풀면 원래 도구다.
    pub inverted: bool,
    /// 펜에 **지우개 끝이 있는가**(`PEN_FLAG_ERASER`) — 장치의 능력이다(자세가 아니다).
    pub has_eraser: bool,
    /// 장치가 보고한 **회전**(0~359도, `PEN_MASK_ROTATION`) — 지금은 진단에만 쓴다.
    pub rotation: Option<f32>,
    pub at: Instant,
}

impl PointerFrame {
    pub fn new(
        device: Device,
        pressure: Option<f32>,
        tilt: Option<(f32, f32)>,
        at: Instant,
    ) -> Self {
        Self {
            device,
            pressure,
            tilt,
            inverted: false,
            has_eraser: false,
            rotation: None,
            at,
        }
    }

    /// **펜의 자세**(뒤집힘·지우개 끝·회전)를 싣는다 — 태블릿 표본을 읽는 [`crate::otd::feed`]가
    /// 디코드해 넘긴다(플랫폼 상수는 저쪽에만 있다).
    pub fn with_pen_pose(
        mut self,
        inverted: bool,
        has_eraser: bool,
        rotation: Option<f32>,
    ) -> Self {
        self.inverted = inverted;
        self.has_eraser = has_eraser;
        self.rotation = rotation;
        self
    }
}

/// 정규화된 표본 — 페이지 좌표(pt), 시각, 그리고 하드웨어 프레임.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub phase: Phase,
    pub at: Pt,
    pub now: Instant,
    /// 이 표본의 하드웨어 프레임 — `None`이면 장치를 모르는 입력이다.
    pub frame: Option<PointerFrame>,
}

impl Sample {
    /// **필기 입력인가** — 디지타이저(펜)만 잉크가 된다(이 앱의 정책).
    pub fn is_ink(self) -> bool {
        self.device() == Device::Pen
    }

    /// 이 표본을 보낸 장치 — 프레임이 없으면 **마우스로 본다**.
    ///
    /// 표본을 만드는 곳(OTD 어댑터)은 언제나 펜이다. 프레임 없는 표본은 다른 입력원이
    /// 붙었을 때를 위한 자리다: 장치를 모르면 잉크도 만들지 않는다.
    pub fn device(self) -> Device {
        self.frame.map_or(Device::Mouse, |frame| frame.device)
    }

    /// 하드웨어 필압 — 디지타이저가 보고한 경우에만 `Some`.
    pub fn pressure(self) -> Option<f32> {
        self.frame.and_then(|frame| frame.pressure)
    }

    /// 틸트(도) — 디지타이저가 보고한 경우에만 `Some`.
    pub fn tilt(self) -> Option<(f32, f32)> {
        self.frame.and_then(|frame| frame.tilt)
    }

    /// 펜을 **뒤집었는가** — 이 표본은 **지운다**(①이 아니라 ②의 정책).
    ///
    /// 장치를 모르는 표본은 뒤집힐 수도 없다(`None` → `false`).
    pub fn inverted(self) -> bool {
        self.frame.is_some_and(|frame| frame.inverted)
    }
}

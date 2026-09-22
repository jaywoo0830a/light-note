//! ① HardwareInput — WinUI 포인터 이벤트를 **모델이 아는 값**으로 바꾼다.
//!
//! 이 단계가 아는 것은 넷이다: 어떤 위상인가, 표면 어디인가, 지금이 언제인가,
//! 그리고 **어떤 장치가 보냈는가**. 도구도 잉크도 페이지도 모른다 — 그래서 배율 변환 하나와
//! 장치 분류 하나만 책임진다.
//!
//! ```text
//! on_pointer_pressed / moved / released / capture_lost / canceled   (④-UI가 붙인다)
//!        │
//!        ▼  Phase + DIP 좌표 + 하드웨어 프레임
//!   sample_with(phase, x, y, scale, frame) → Sample { phase, at, now, frame }   (여기)
//!        │
//!        ▼  메시지로 UI 스레드에 (② CanvasTool이 받는다)
//! ```
//!
//! ## 이 앱은 **디지타이저(펜)만** 필기한다
//! 드로잉 패드 친화 필기 앱이라 표본은 **장치를 들고 온다**: 펜이면 잉크가 되고,
//! 손가락·마우스는 아무것도 만들지 않는다([`Sample::is_ink`]). WinUI는 장치를 알려주지
//! 않으므로(`PointerEventInfo`에는 좌표와 버튼뿐) Win32 `WM_POINTER`를 읽는
//! [`crate::digitizer`]가 프레임을 만들어 여기에 넣는다.
//!
//! 취소는 **두 경로로 온다**: 포인터 캡처 상실(`capture_lost`)과 시스템 취소(`canceled`).
//! 둘 다 [`Phase::Canceled`] 하나로 모은다 — 그래야 ②가 진행 중인 획을 확실히 되돌린다.

use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::geom::{Pt, Scale};

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

/// 하드웨어가 알려 준 **한 포인터 프레임**(장치·필압·틸트·**펜의 자세**) — [`crate::digitizer`]가 채운다.
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

/// 프레임의 수명(ms) — 표본은 WinUI 이벤트 **직후**에 오므로, 이보다 오래된 프레임은 남의 것이다.
///
/// 펜을 떼고 나서 온 마우스 표본에 펜 압력(그리고 "펜이다"라는 자격)이 붙으면 둘 다 거짓이 된다.
pub const FRAME_TTL_MS: u64 = 50;

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

    /// **펜의 자세**(`penFlags`/`PEN_MASK_ROTATION`)를 싣는다 — [`crate::digitizer`]가 창 프로시저에서
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

    /// 지금(`now`) 표본의 재료로 쓸 수 있는가.
    pub fn is_fresh(self, now: Instant) -> bool {
        now.saturating_duration_since(self.at) <= Duration::from_millis(FRAME_TTL_MS)
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
    /// Win32 `WM_POINTER`는 펜·터치에만 온다(마우스는 `WM_MOUSE*`). 그래서 프레임이 없다는
    /// 것은 곧 마우스라는 뜻이고, 마우스는 필기하지 않는다.
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

    /// 펜을 **뒤집었는가**(`PEN_FLAG_INVERTED`) — 이 표본은 **지운다**(①이 아니라 ②의 정책).
    ///
    /// 장치를 모르는 표본은 뒤집힐 수도 없다(`None` → `false`).
    pub fn inverted(self) -> bool {
        self.frame.is_some_and(|frame| frame.inverted)
    }
}

/// WinUI DIP 좌표 → 표본(pt). **변환은 이 함수 하나뿐이다.**
pub fn sample(phase: Phase, x: f64, y: f64, scale: Scale) -> Sample {
    sample_with(phase, x, y, scale, None)
}

/// 하드웨어 프레임까지 받는 표본 — ①의 **유일한 변환 지점**.
///
/// 낡은 프레임은 **버린다**([`FRAME_TTL_MS`]): 장치도 압력도 "지금 이 표본의 것"이어야 한다.
pub fn sample_with(
    phase: Phase,
    x: f64,
    y: f64,
    scale: Scale,
    frame: Option<PointerFrame>,
) -> Sample {
    let now = Instant::now();
    Sample {
        phase,
        at: scale.to_pt(x as f32, y as f32),
        now,
        frame: frame.filter(|frame| frame.is_fresh(now)),
    }
}

/// Win32 압력(0~1024) → 0.0~1.0. `POINTER_PEN_INFO.pressure`의 계약 그대로다.
pub fn pressure_from_raw(raw: u32) -> f32 {
    (raw as f32 / 1024.0).clamp(0.0, 1.0)
}

/// Win32 틸트(-90~90도) → 도. `POINTER_PEN_INFO.tiltX/tiltY`의 계약 그대로다.
pub fn tilt_from_raw(raw: i32) -> f32 {
    (raw as f32).clamp(-90.0, 90.0)
}

/// Win32 회전(0~359도, 시계 방향) → 도. `POINTER_PEN_INFO.rotation`의 계약 그대로다.
///
/// 문서가 범위를 0~359로 못박으므로 **한 바퀴로 접는다**(360을 보고하는 장치가 있어도 각도다).
pub fn rotation_from_raw(raw: u32) -> f32 {
    (raw % 360) as f32
}

/// 표면이 호스트에 포인터를 알리는 통로 — 자기 메시지 큐로 옮긴다.
///
/// `fn`이 아니라 `Rc<dyn Fn>`인 이유: ④-UI의 빌더가 **호스트의 `LocalSender`를 캡처**한다
/// (elm `<Raw>`는 슬롯·props를 볼 수 없으므로 캡처가 유일한 통로다 — 예제 08).
pub type InputSink = Rc<dyn Fn(Phase, f64, f64)>;

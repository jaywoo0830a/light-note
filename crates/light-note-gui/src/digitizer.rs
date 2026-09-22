//! ①-하드웨어의 **Win32 절반** — `WM_POINTER`를 읽어 펜 프레임(장치·필압·틸트)을 만든다.
//!
//! ## 왜 Win32인가 (WinUI만으로는 안 되는 이유)
//! `PointerEventInfo`에는 좌표와 버튼만 있다 — **장치도, pointer id도, 압력도 없다**.
//! 그래서 이 모듈은 앱 창의 프로시저를 **서브클래스**해 `WM_POINTERDOWN/UPDATE/UP`을
//! **관찰만** 하고 `DefSubclassProc`으로 그대로 흘려보낸다. XAML의 입력·캡처·좌표 변환은
//! 손대지 않는다(④-UI의 포인터 이벤트는 그대로 온다).
//!
//! ```text
//! WM_POINTERDOWN / UPDATE / UP        wParam 하위 워드 = pointer id
//!   └ GetPointerType(id) → PT_PEN | PT_TOUCH | PT_MOUSE
//!       └ PT_PEN이면 GetPointerPenInfo(id)
//!            penMask · pressure(0~1024) · tiltX/tiltY(-90~90도)
//!            └ LATEST(스레드 로컬) ← ①이 WinUI 이벤트 **순간**에 읽는다
//! ```
//!
//! ## 왜 RealTimeStylus(`rtscom.h`)가 아닌가
//! RTS는 Windows XP Tablet PC 시절의 WISP 경로다: COM 싱크 플러그인(`IStylusSyncPlugin`)을
//! 구현해야 하고, 창에 붙이면 **입력을 독점**해서 XAML이 포인터를 못 본다. `WM_POINTER`는
//! Windows 8+의 표준 경로이고 관찰만 하면 되며, 압력·틸트는 `GetPointerPenInfo`가 같은 것을
//! 준다(여러 점이 뭉쳐 오면 `GetPointerPenInfoHistory`가 중간 점까지 준다 — 지금은 최신 한 점).
//!
//! ## 필기 자격은 **여기**가 정한다
//! 펜이면 [`Device::Pen`], 터치면 [`Device::Touch`], 아무것도 못 봤으면 프레임이 없다 =
//! 마우스다([`crate::input::Sample::device`]). ②가 그 장치를 보고 **펜만** 잉크로 만든다
//! ([`crate::tool::CanvasTool::accepts`]).
//!
//! ## 한계 (정직하게)
//! - **UI 스레드 전용**이다: 프레임은 스레드 로컬이고 잠금이 없다(읽는 쪽도 UI 스레드다).
//! - **시간 결합**: WinUI 이벤트는 `WM_POINTER` 메시지 **직후** 같은 스레드에서 온다. 그래서
//!   "이벤트 순간의 최신 프레임"이 그 이벤트의 프레임이다. 낡은 프레임은 ①이 버린다.
//! - **창을 못 찾으면 아무것도 못 본다**(리액터가 HWND를 공개하지 않아 찾아야 한다) —
//!   그 사실을 [`state`]로 남기고 상태바가 "왜 안 그려지는지"를 말한다.
//! - **마우스는 여기로 오지 않는다**(`WM_MOUSE*`) — 그래서 마우스로는 **필기할 수 없다**.
//!   원격 데스크톱·가상 머신처럼 디지타이저가 없는 환경도 같다(사고가 아니라 정책이다).

use std::cell::Cell;

use crate::input::{Device, PointerFrame};

/// 창 서브클래스 설치 상태 — **필기가 안 되는 이유**를 화면이 말할 수 있게 남긴다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookState {
    /// 아직 시도하지 않았다(창이 없을 수 있다 — 다음 기회에 다시 시도한다).
    Idle,
    /// 창을 찾아 서브클래스를 걸었다.
    Hooking,
    /// 창을 못 찾았다 — `WM_POINTER`를 볼 수 없다.
    NoWindow,
    /// 서브클래스 설치가 거부됐다.
    Failed,
}

impl HookState {
    /// 펜 프레임을 받을 수 있는 상태인가.
    pub const fn is_hooking(self) -> bool {
        matches!(self, HookState::Hooking)
    }

    /// 상태바 문구 — 설치가 됐으면 빈 문자열이다.
    pub const fn reason(self) -> &'static str {
        match self {
            HookState::Hooking => "",
            HookState::Idle | HookState::NoWindow => {
                "App window not found — pen input cannot be read"
            }
            HookState::Failed => "Window subclass install was refused — pen input cannot be read",
        }
    }
}

thread_local! {
    /// UI 스레드의 최신 프레임 — 서브클래스가 쓰고 ①이 읽는다(같은 스레드라 잠금이 없다).
    static LATEST: Cell<Option<PointerFrame>> = const { Cell::new(None) };
    /// `PT_PEN`을 한 번이라도 봤는가 — 상태바가 "펜이 안 잡힌다"를 구분할 수 있게.
    static SEEN_PEN: Cell<bool> = const { Cell::new(false) };
    /// 설치 상태.
    static STATE: Cell<HookState> = const { Cell::new(HookState::Idle) };
}

/// 설치 상태 — 설치가 안 됐으면 **필기가 안 되는 이유**가 여기 있다.
pub fn state() -> HookState {
    STATE.get()
}

/// 최근 프레임 — `None`이면 아직 펜·터치 입력을 못 봤다는 뜻이다(마우스일 수 있다).
pub fn latest() -> Option<PointerFrame> {
    LATEST.get()
}

/// 디지타이저(펜)를 한 번이라도 봤는가.
pub fn seen_pen() -> bool {
    SEEN_PEN.get()
}

/// 앱 창을 찾아 서브클래스를 건다 — **여러 번 불러도 한 번만** 걸린다(`view()`/`update()`가 매번 부른다).
pub fn install() {
    if state().is_hooking() {
        return;
    }
    #[cfg(windows)]
    win32::install();
}

#[cfg(windows)]
mod win32 {
    use std::time::Instant;

    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows::Win32::UI::Input::Pointer::{GetPointerPenInfo, GetPointerType, POINTER_PEN_INFO};
    use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GetClientRect, IsWindowVisible, PEN_MASK_PRESSURE, PEN_MASK_TILT_X,
        PEN_MASK_TILT_Y, POINTER_INPUT_TYPE, PT_PEN, PT_TOUCH, WM_POINTERDOWN, WM_POINTERUP,
        WM_POINTERUPDATE,
    };

    use super::{Device, HookState, LATEST, SEEN_PEN, STATE};
    use crate::input::{self, PointerFrame};

    /// comctl32 서브클래스 식별자 — 한 창에 여러 서브클래스가 붙으므로 우리 것에 이름을 준다.
    const SUBCLASS_ID: usize = 0x4C4E_5054; // "LNPT"
    /// pointer id는 `wParam`의 **하위 워드**다(`GET_POINTERID_WPARAM`).
    const POINTER_ID_MASK: usize = 0xFFFF;

    /// 창을 찾아 서브클래스를 건다(실패해도 이유만 남기고 앱은 계속 돈다).
    pub(super) fn install() {
        let Some(hwnd) = app_window() else {
            STATE.set(HookState::NoWindow);
            return;
        };
        let hooked = unsafe { SetWindowSubclass(hwnd, Some(pointer_proc), SUBCLASS_ID, 0) };
        STATE.set(if hooked.0 != 0 {
            HookState::Hooking
        } else {
            HookState::Failed
        });
    }

    /// 이 스레드의 앱 창 — 리액터는 HWND를 공개하지 않으므로 **찾는다**.
    ///
    /// ① 이 스레드의 활성 창이 있으면 그것(대부분 이 경우다),
    /// ② 없으면 이 스레드의 최상위 창 중 **가장 큰 보이는 창**.
    /// 앱은 창이 하나뿐이라 이 둘로 충분하다.
    fn app_window() -> Option<HWND> {
        let active = unsafe { GetActiveWindow() };
        if !active.is_invalid() {
            return Some(active);
        }
        let mut best: Option<(HWND, i64)> = None;
        unsafe {
            let _ = EnumThreadWindows(
                GetCurrentThreadId(),
                Some(collect_largest),
                LPARAM((&raw mut best) as isize),
            );
        }
        best.map(|(hwnd, _)| hwnd)
    }

    /// 열거 콜백 — **가장 큰 보이는 창**을 고른다(창이 하나면 그것이 곧 앱 창이다).
    unsafe extern "system" fn collect_largest(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let best = unsafe { &mut *(lparam.0 as *mut Option<(HWND, i64)>) };
        let mut rect = RECT::default();
        let visible = unsafe { IsWindowVisible(hwnd) }.0 != 0;
        if visible && unsafe { GetClientRect(hwnd, &mut rect) }.is_ok() {
            let area = i64::from(rect.right - rect.left) * i64::from(rect.bottom - rect.top);
            if best.is_none_or(|(_, best_area)| area > best_area) {
                *best = Some((hwnd, area));
            }
        }
        BOOL(1) // 계속 열거한다.
    }

    /// 창 프로시저 — `WM_POINTER*`를 **읽고 그대로 넘긴다**(입력을 소비하지 않는다).
    unsafe extern "system" fn pointer_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if matches!(message, WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP) {
            record((wparam.0 & POINTER_ID_MASK) as u32);
        }
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    /// pointer id 하나를 프레임으로 — **여기가 하드웨어와 모델의 경계**다.
    fn record(id: u32) {
        let mut kind = POINTER_INPUT_TYPE::default();
        let device = match unsafe { GetPointerType(id, &mut kind) } {
            Ok(()) if kind == PT_PEN => Device::Pen,
            Ok(()) if kind == PT_TOUCH => Device::Touch,
            _ => Device::Mouse,
        };
        let (pressure, tilt) = if device == Device::Pen {
            SEEN_PEN.set(true);
            pen_info(id)
        } else {
            (None, None)
        };
        LATEST.set(Some(PointerFrame::new(
            device,
            pressure,
            tilt,
            Instant::now(),
        )));
    }

    /// `POINTER_PEN_INFO` → (압력, 틸트) — **장치가 보고한 것만** `Some`이다(`penMask`).
    fn pen_info(id: u32) -> (Option<f32>, Option<(f32, f32)>) {
        let mut info = POINTER_PEN_INFO::default();
        if unsafe { GetPointerPenInfo(id, &mut info) }.is_err() {
            return (None, None);
        }
        let pressure = (info.penMask & PEN_MASK_PRESSURE != 0)
            .then(|| input::pressure_from_raw(info.pressure));
        let tilt_mask = PEN_MASK_TILT_X | PEN_MASK_TILT_Y;
        let tilt = (info.penMask & tilt_mask == tilt_mask).then(|| {
            (
                input::tilt_from_raw(info.tiltX),
                input::tilt_from_raw(info.tiltY),
            )
        });
        (pressure, tilt)
    }
}

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
//! ## `POINTER_PEN_INFO`를 어디까지 쓰는가 (공식 계약)
//! | 필드 | 쓰는가 |
//! |---|---|
//! | `pressure`(0~1024) | ✅ `PEN_MASK_PRESSURE`일 때만 → 0.0~1.0([`crate::input::pressure_from_raw`]) |
//! | `tiltX`/`tiltY`(−90~+90) | ✅ 두 축이 **둘 다** 보고됐을 때만 `Some` |
//! | `penFlags` | ✅ `PEN_FLAG_INVERTED`(뒤집힘 → **지운다**) · `PEN_FLAG_ERASER`(지우개 끝의 유무) |
//! | `rotation`(0~359) | ⚠️ `PEN_MASK_ROTATION`일 때만 읽어 **진단에만** 보여준다(잉크 모양에는 안 쓴다) |
//! | `pointerInfo.pointerFlags` | ✅ `POINTER_FLAG_INCONTACT` — 호버 갱신은 **프레임이 아니다** |
//! | `pointerInfo.historyCount` | ❌ 안 쓴다 — 아래 한계 참고 |
//!
//! ## OTD 같은 드라이버와 함께 쓸 때
//! 원시 입력 장치 목록(`GetRawInputDeviceList`, HID 사용 페이지 `0x0D`)을 읽어 **Windows가 아는
//! 디지타이저**를 표에 보여준다 — 이름이 곧 **어느 드라이버가 펜을 내보내는가**다(예: VMulti의
//! `VirtualHID`, VID `0x00FF`/PID `0xBACC`). 그리고 창마다 **마우스 메시지 수**도 센다: 펜을
//! **마우스로** 내보내는 출력 모드(예: OpenTabletDriver의 기본 Absolute/Relative Mode =
//! `SendInput`)는 `WM_POINTER`를 **하나도** 만들지 않으므로, 그 경우 표는 `pen 0 · mouse N`이라고
//! 정확히 말한다(그때는 그 드라이버에서 Windows Ink 계열 출력 모드를 골라야 한다).
//!
//! ## 한계 (정직하게)
//! - **점이 뭉쳐 오면 최신 하나만** 쓴다: 문서는 "처리가 못 따라가면 메시지가 합쳐지니
//!   `GetPointerPenInfoHistory`를 쓰라"고 말한다. 지금은 합쳐진 갱신에서 **마지막 점**만 취하므로
//!   아주 빠른 획에서 점 간격이 넓어진다 — 고치려면 히스토리를 **여러 표본**으로 흘려야 하고,
//!   그러면 "이벤트 하나 = 표본 하나"인 ①의 계약이 바뀐다(다음 단계).
//! - **`PEN_FLAG_BARREL`(배럴 버튼)은 안 쓴다** — 보조 버튼에 동작을 붙이지 않았다.
//! - **UI 스레드 전용**이다: 프레임은 스레드 로컬이고 잠금이 없다(읽는 쪽도 UI 스레드다).
//! - **시간 결합**: WinUI 이벤트는 `WM_POINTER` 메시지 **직후** 같은 스레드에서 온다. 그래서
//!   "이벤트 순간의 최신 프레임"이 그 이벤트의 프레임이다. 낡은 프레임은 ①이 버린다.
//! - **창을 못 찾으면 아무것도 못 본다**(리액터가 HWND를 공개하지 않아 찾아야 한다) —
//!   그 사실을 [`state`]로 남기고 상태바가 "왜 안 그려지는지"를 말한다.
//! - **마우스는 여기로 오지 않는다**(`WM_MOUSE*`) — 그래서 마우스로는 **필기할 수 없다**.
//!   원격 데스크톱·가상 머신처럼 디지타이저가 없는 환경도 같다(사고가 아니라 정책이다).
//!
//! ## 안 될 때 **어디를 보는가** (개발자 도구)
//! 필기는 세 관문을 다 지나야 성립한다: ①시스템에 펜이 있는가 ②우리가 **메시지를 받는 창**에
//! 훅을 걸었는가 ③그 창으로 펜 메시지가 오는가. 하나라도 막히면 잉크가 안 나오는데, 화면에는
//! 그 이유가 없다 — 그래서 이 모듈이 [`Digest`]를 열어 두고 툴바의 진단 도구가 그대로 보여준다.
//!
//! 관문 ②가 함정이다: WinUI 3의 창은 **하나가 아니다**. 최상위 앱 창 안에 콘텐츠를 담는
//! **자식 창**(site bridge)이 따로 있고 포인터는 자식으로 온다. 그래서 [`rescan`]은 최상위 창의
//! 자식까지 **전부** 걸고, 창마다 몇 개가 도착했는지([`Probe::messages`])를 센다 —
//! 어느 창에 걸어야 했는가가 숫자로 드러난다.

use std::cell::{Cell, RefCell};

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
    /// 모든 창을 합친 `WM_POINTER*` 메시지 수 — **아직 아무것도 안 왔는가**의 근거다.
    static MESSAGES: Cell<u32> = const { Cell::new(0) };
    /// 찾은 창들 — 창 프로시저가 자기 자리를 **인덱스로** 찾는다(참조 데이터).
    static PROBES: RefCell<Vec<Slot>> = const { RefCell::new(Vec::new()) };
    /// 시스템 디지타이저 — 창과 무관하므로 한 번 읽으면 된다.
    static DIGITIZER: Cell<Digitizer> = const {
        Cell::new(Digitizer {
            present: false,
            integrated_pen: false,
            external_pen: false,
            integrated_touch: false,
            ready: false,
        })
    };
}

/// 설치 상태 — 설치가 안 됐으면 **필기가 안 되는 이유**가 여기 있다.
pub fn state() -> HookState {
    STATE.get()
}

/// 최근 프레임 — `None`이면 아직 펜·터치 입력을 못 봤다는 뜻이다(마우스일 수 있다).
pub fn latest() -> Option<PointerFrame> {
    LATEST.get()
}

/// 디지타이저(펜)를 한 번이라도 봤는가 — **호버만 해도 참**이다(접촉은 프레임의 자격이다).
pub fn seen_pen() -> bool {
    SEEN_PEN.get()
}

/// 상태 띠 배지의 **문장** — 펜 입력이 왜 잉크가 안 되는지 한 줄로 말한다.
///
/// 배지는 사용자가 **가장 먼저** 읽는 문장이다. 그래서 "펜이 안 잡힌다"로 끝내지 않고 **왜**를
/// 말한다 — 관문 순서대로: ① 훅이 안 걸렸으면 그 이유, ② 펜을 봤으면 그것, ③ 못 봤으면 그 이유
/// (포인터가 오는데 펜이 아닌가 / 마우스로 오는가 / 아무것도 안 오는가).
///
/// 순수 함수라 화면 없이 검증한다(정적값은 인자로 온다).
pub fn input_badge(state: HookState, seen_pen: bool, messages: u32, mouse: u32) -> String {
    if !state.is_hooking() {
        return state.reason().to_string();
    }
    if seen_pen {
        return "Pen — digitizer active".to_string();
    }
    if messages > 0 {
        // 포인터 메시지는 오는데 `PT_PEN`이 아니다 — 터치이거나 마우스 포인터다.
        "Pen only — pointer input is not a pen".to_string()
    } else if mouse > 0 {
        // `WM_POINTER`가 **하나도** 없고 마우스만 온다 = 펜을 마우스로 내보내는 드라이버다.
        "Pen only — input arrives as mouse (check the tablet driver)".to_string()
    } else {
        "Pen only — no pen detected yet".to_string()
    }
}

// ── 진단 (개발자 도구가 읽는다) ────────────────────────────────────

/// 시스템이 보고하는 **디지타이저 종류** — `GetSystemMetrics(SM_DIGITIZER)`의 플래그.
///
/// 이 값이 **먼저**다: 펜이 없는 기계라면 훅이 아무리 잘 걸려도 필기는 안 된다. 진단 도구가
/// 첫 줄에서 하드웨어가 문제인지 우리가 문제인지를 가른다.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Digitizer {
    /// 태블릿 입력이 하나라도 있는가.
    pub present: bool,
    /// 내장 펜(노트북에 붙은 펜).
    pub integrated_pen: bool,
    /// 외장 펜(드로잉 패드).
    pub external_pen: bool,
    /// 손가락 터치.
    pub integrated_touch: bool,
    /// 드라이버가 준비됐는가.
    pub ready: bool,
}

impl Digitizer {
    /// 펜이 하나라도 있는가 — **필기가 가능한 기계인가**의 답.
    pub const fn has_pen(self) -> bool {
        self.integrated_pen || self.external_pen
    }

    /// 진단 줄에 쓸 한 줄(**영어만**) — 펜이 없으면 그 사실이 맨 앞에 온다.
    pub fn summary(self) -> String {
        if !self.present {
            // **거짓 결론을 피한다**: 이 지표(`SM_DIGITIZER`)는 내장 디지타이저 기준이라
            // USB 펜·가상 펜(예: OTD의 VMulti)에서는 **0으로 나온다**(실측: 펜 장치가 등록돼
            // 있는데도 0). 그래서 "펜이 없다"가 아니라 "여기에는 안 잡힌다"까지만 말한다.
            return "0 — no tablet reported by this metric (USB and virtual pens often report 0)"
                .to_string();
        }
        let mut kinds: Vec<&str> = Vec::new();
        if self.integrated_pen {
            kinds.push("integrated pen");
        }
        if self.external_pen {
            kinds.push("external pen");
        }
        if self.integrated_touch {
            kinds.push("touch");
        }
        if kinds.is_empty() {
            kinds.push("tablet input, but no pen");
        }
        let ready = if self.ready {
            ""
        } else {
            " (driver not ready)"
        };
        format!("{}{ready}", kinds.join(" · "))
    }
}

/// Windows가 아는 **디지타이저 장치 하나** — 이름과 용도(원시 입력 장치 목록에서 읽는다).
///
/// 이 목록이 중요한 이유: `WM_POINTER`는 **OS가 펜 장치를 알 때만** 나온다. 그래서 "펜이 안 잡힌다"는
/// 문제는 두 갈래로 갈린다 — ① OS가 펜을 아예 모른다(장치 목록이 비었다), ② OS는 아는데 펜을
/// **마우스로** 내보내는 드라이버다(`WM_POINTER`는 안 나오고 마우스 메시지만 온다).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PenDevice {
    /// 장치 경로에서 사람이 읽는 부분만 남긴 이름(예: `hid vid_00ff&pid_bacc`).
    pub name: String,
    /// 디지타이저 페이지의 용도 — 펜(0x02)과 손가락(0x04·0x05)은 **다른 장치**다.
    pub usage: u16,
}

/// HID 사용 페이지의 용도(usage) 이름 — 디지타이저 페이지(0x0D)의 값들이다.
///
/// 순수 함수다: 표에 어떤 말이 나가는지 화면 없이 검증한다.
pub fn usage_label(usage: u16) -> &'static str {
    match usage {
        0x01 => "digitizer",
        0x02 => "pen",
        0x03 => "light pen",
        0x04 => "touch screen",
        0x05 => "touch pad",
        0x06 => "white board",
        0x07 | 0x08 => "3D digitizer",
        _ => "digitizer device",
    }
}

/// 원시 입력 장치 경로를 **사람이 읽는 이름**으로 줄인다.
///
/// `\\?\hid#vid_00ff&pid_bacc&mi_00#7&5b1c#0000#{4d1e55b2-...}` → `hid vid_00ff&pid_bacc&mi_00`.
/// 앞 두 조각(버스 + VID/PID, 그리고 있으면 `mi_xx` **인터페이스**)만 남긴다: 그게 **어떤 장치인지**를
/// 말하는 부분이고, 나머지(인스턴스 번호·GUID)는 꽂을 때마다 달라진다. `mi_xx`를 남기는 이유는
/// 같은 VID/PID의 장치가 **인터페이스마다 다른 용도**로 나타나기 때문이다(디지타이저 + 마우스 등).
///
/// 순수 함수다 — 줄이는 규칙이 곧 진단 표의 문장이므로 화면 없이 검증한다.
pub fn compact_device_name(path: &str) -> String {
    let body = path.split("#{").next().unwrap_or(path);
    let body = body.trim_start_matches("\\\\?\\");
    let mut parts = body.split('#');
    let head = parts.next().unwrap_or_default();
    match parts.next() {
        Some(ids) if !ids.is_empty() => format!("{head} {ids}"),
        _ => head.to_string(),
    }
}

/// 창 하나의 **관측 기록** — 어느 창으로 입력이 오는가를 눈으로 보기 위한 것.
///
/// WinUI 3의 창은 하나가 아니다: 최상위 앱 창과 그 안의 자식 콘텐츠 창이 따로 있다. 창마다
/// [`Probe::messages`]를 세면 **어디에 걸어야 했는가**가 숫자로 드러난다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// 창 핸들 — 진단 줄에 16진수로 찍는다.
    pub hwnd: isize,
    /// 창 클래스 이름(예: `Microsoft.UI.Content.DesktopChildSiteBridge`).
    pub class: String,
    /// 창 제목(보통 빈 문자열이다).
    pub title: String,
    /// 보이는 창인가.
    pub visible: bool,
    /// 클라이언트 크기 (px).
    pub size: (i32, i32),
    /// 서브클래스가 걸렸는가.
    pub hooked: bool,
    /// 이 창이 받은 `WM_POINTER*` 메시지 수.
    pub messages: u32,
    /// 그중 **펜**이었던 수 — **호버도 센다**(접촉 여부는 프레임의 자격이지 펜의 자격이 아니다).
    pub pen: u32,
    /// 이 창이 받은 **마우스** 메시지 수(`WM_MOUSEMOVE`·`WM_LBUTTONDOWN`·`WM_LBUTTONUP`).
    ///
    /// 펜을 마우스로 내보내는 드라이버는 `WM_POINTER`를 만들지 않는다 — 그때 이 숫자만 움직인다.
    pub mouse: u32,
}

impl Probe {
    /// 진단 줄의 **이름** — 클래스 이름 + 핸들.
    pub fn name(&self) -> String {
        let class = if self.class.is_empty() {
            "(no class name)"
        } else {
            self.class.as_str()
        };
        format!("{class}  {:#x}", self.hwnd)
    }

    /// 진단 줄의 **값** — 크기 · 표시 · 훅 · 도착한 메시지.
    pub fn detail(&self) -> String {
        let (width, height) = self.size;
        let visible = if self.visible { "visible" } else { "hidden" };
        let hooked = if self.hooked { "hooked" } else { "not hooked" };
        format!(
            "{width}×{height} · {visible} · {hooked} · msgs {} · pen {} · mouse {}",
            self.messages, self.pen, self.mouse
        )
    }
}

/// 지금 이 순간의 진단 — 화면이 그대로 올리는 값(문장은 [`Digest::report`]가 만든다).
#[derive(Clone, Debug, PartialEq)]
pub struct Digest {
    /// 훅 설치 상태.
    pub state: HookState,
    /// 펜 프레임을 한 번이라도 받았는가.
    pub seen_pen: bool,
    /// 모든 창을 합친 `WM_POINTER*` 메시지 수.
    pub messages: u32,
    /// 모든 창을 합친 **마우스** 메시지 수 — 펜이 마우스로 오는 경우의 유일한 흔적이다.
    pub mouse: u32,
    /// 찾은 창들(걸린 것과 못 걸린 것 모두).
    pub probes: Vec<Probe>,
    /// 마지막 펜 프레임.
    pub last: Option<PointerFrame>,
    /// 마지막 프레임이 몇 ms 전인가.
    pub last_age_ms: Option<u64>,
    /// 시스템 디지타이저.
    pub digitizer: Digitizer,
    /// Windows가 아는 디지타이저 장치들(원시 입력 장치 목록) — 비어 있으면 OS가 펜을 모른다.
    pub pen_devices: Vec<PenDevice>,
}

impl Digest {
    /// 진단 줄 — `(이름, 값)` 쌍. **영어만** 쓴다(화면 언어 규칙).
    ///
    /// 순수 함수라 WinUI 없이 테스트된다: 이 줄들이 곧 무엇을 보고 판단할 것인가다.
    pub fn report(&self) -> Vec<(String, String)> {
        let mut rows = vec![
            ("System digitizer".to_string(), self.digitizer.summary()),
            ("Hook".to_string(), self.hook_summary()),
            ("Pen frames".to_string(), self.pen_summary()),
            ("WM_POINTER messages".to_string(), self.messages.to_string()),
            ("Mouse messages".to_string(), self.mouse.to_string()),
            ("Windows pen devices".to_string(), self.device_summary()),
            ("Last frame".to_string(), self.frame_summary()),
        ];
        if self.probes.is_empty() {
            rows.push((
                "Windows".to_string(),
                "none found on this thread".to_string(),
            ));
        }
        // 펜이 안 잡히는 기계라면 그 사실이 **실마리**다 — 다만 이 지표는 **결론이 아니다**(아래 참조).
        if !self.digitizer.has_pen() {
            rows.push((
                "Hint".to_string(),
                "the system metric reports no pen — USB and virtual pens often report 0 here; judge \
                 by the device list and the per-window counters instead"
                    .to_string(),
            ));
        }
        // 펜을 봤는데 접촉 프레임이 아직 없다 = **호버 중**이다(접촉 전 업데이트는 프레임이 아니다).
        if self.seen_pen && self.last.is_none() {
            rows.push((
                "Hint".to_string(),
                "the pen is in range but not in contact — touch the surface to draw".to_string(),
            ));
        }
        // 펜 메시지는 없는데 **마우스**만 오고 있다면 그게 결론이다: 펜을 마우스로 내보내는 드라이버다.
        if !self.seen_pen && self.mouse > 0 {
            rows.push((
                "Hint".to_string(),
                "only mouse input arrives — OpenTabletDriver's Absolute/Relative Mode sends SendInput \
                 mouse; choose the Windows Ink plugin or the Windows Pen Pointer output mode"
                    .to_string(),
            ));
        }
        for probe in &self.probes {
            rows.push((probe.name(), probe.detail()));
        }
        rows
    }

    /// 관문 ② — 훅이 어디까지 갔는가.
    fn hook_summary(&self) -> String {
        let hooked = self.probes.iter().filter(|probe| probe.hooked).count();
        match self.state {
            HookState::Hooking => format!("hooked on {hooked} of {} window(s)", self.probes.len()),
            HookState::Idle => "not tried yet".to_string(),
            HookState::NoWindow => "no app window found — still looking".to_string(),
            HookState::Failed => "the window refused the subclass".to_string(),
        }
    }

    /// 관문 ③ — 펜 메시지가 도착했는가(안 왔으면 **어디까지 왔는지**를 말한다).
    fn pen_summary(&self) -> String {
        if self.seen_pen {
            "yes — the digitizer reported a pen".to_string()
        } else if self.messages > 0 {
            "no — messages arrived, none of them a pen".to_string()
        } else {
            "no — no pointer message has arrived at all".to_string()
        }
    }

    /// 마지막 프레임 한 줄 — 장치 · 필압 · 틸트 · 나이.
    /// Windows가 아는 디지타이저들 — 이름이 곧 **어떤 드라이버가 펜을 내보내는가**다.
    fn device_summary(&self) -> String {
        if self.pen_devices.is_empty() {
            return "none — Windows sees no digitizer device".to_string();
        }
        self.pen_devices
            .iter()
            .map(|device| format!("{} · {}", device.name, usage_label(device.usage)))
            .collect::<Vec<_>>()
            .join(" | ")
    }

    fn frame_summary(&self) -> String {
        let Some(frame) = self.last else {
            return "none yet".to_string();
        };
        let mut text = frame.device.label().to_string();
        if let Some(pressure) = frame.pressure {
            text.push_str(&format!(" · pressure {pressure:.2}"));
        }
        if let Some((x, y)) = frame.tilt {
            text.push_str(&format!(" · tilt {x:.0}/{y:.0}"));
        }
        // 펜의 **자세**도 사실이다: 뒤집힘(지우개 끝) · 지우개 끝의 유무 · 회전.
        if frame.inverted {
            text.push_str(" · flipped (erasing)");
        }
        if frame.has_eraser {
            text.push_str(" · eraser tip");
        }
        if let Some(rotation) = frame.rotation {
            text.push_str(&format!(" · rotation {rotation:.0}"));
        }
        if let Some(age) = self.last_age_ms {
            text.push_str(&format!(" · {age} ms ago"));
        }
        text
    }
}

/// 지금 이 순간의 진단 — 개발자 도구가 읽는다(**UI 스레드 전용**).
pub fn digest() -> Digest {
    let last = latest();
    Digest {
        state: state(),
        seen_pen: seen_pen(),
        messages: MESSAGES.get(),
        mouse: mouse(),
        probes: probes(),
        last,
        last_age_ms: last.map(|frame| frame.at.elapsed().as_millis() as u64),
        digitizer: DIGITIZER.get(),
        pen_devices: pen_devices(),
    }
}

/// 창을 **다시 찾아** 아직 안 걸린 창에 서브클래스를 건다 — 개발자 도구의 리스캔.
///
/// 리액터는 HWND를 공개하지 않으므로 창 찾기는 추측이다. 늦게 생긴 창(자식 콘텐츠 창)을 줍기
/// 위해 다시 부를 수 있게 열어 둔다 — 카운터는 **지우지 않는다**(무엇이 왔는지가 정보다).
pub fn rescan() {
    #[cfg(windows)]
    win32::rescan();
}

/// 창 목록의 **복사본** — 스레드 로컬을 오래 붙들지 않는다(창 프로시저가 거기에 더한다).
fn probes() -> Vec<Probe> {
    PROBES.with(|slots| {
        slots
            .borrow()
            .iter()
            .map(|slot| Probe {
                hwnd: slot.hwnd,
                class: slot.class.clone(),
                title: slot.title.clone(),
                visible: slot.visible,
                size: slot.size,
                hooked: slot.hooked,
                messages: slot.messages.get(),
                pen: slot.pen.get(),
                mouse: slot.mouse.get(),
            })
            .collect()
    })
}

/// 모든 창이 받은 **마우스** 메시지의 합 — 창 프로시저가 센 값을 그대로 더한다.
fn mouse() -> u32 {
    PROBES.with(|slots| slots.borrow().iter().map(|slot| slot.mouse.get()).sum())
}

/// Windows가 아는 디지타이저 장치들(원시 입력 장치 목록) — Windows 밖에서는 빈 목록이다.
fn pen_devices() -> Vec<PenDevice> {
    #[cfg(windows)]
    {
        win32::pen_devices()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// 스레드 로컬 원본 — 카운터가 `Cell`이라 **창 프로시저가 빌림 없이** 더한다.
struct Slot {
    hwnd: isize,
    class: String,
    title: String,
    visible: bool,
    size: (i32, i32),
    hooked: bool,
    messages: Cell<u32>,
    pen: Cell<u32>,
    mouse: Cell<u32>,
}

/// 앱 창을 찾아 서브클래스를 건다 — `view()`/`update()`가 매번 부른다(**공짜여야 한다**).
///
/// **펜을 한 번 볼 때까지** 다시 찾는다: WinUI 3의 자식 콘텐츠 창은 앱 창보다 늦게 생길 수 있고,
/// 그 창을 빠뜨리면 훅이 최상위 창에만 걸려 **아무 메시지도 못 본다**(필기가 안 되는 대표 사고다).
/// 펜을 봤으면 그만 찾는다 — 열거와 이름 읽기는 공짜가 아니다.
pub fn install() {
    if seen_pen() {
        return;
    }
    #[cfg(windows)]
    win32::install();
}

#[cfg(windows)]
mod win32 {
    use std::cell::Cell;
    use std::mem::size_of;
    use std::time::Instant;

    use windows::core::BOOL;
    use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::Pointer::{
        GetPointerInfo, GetPointerPenInfo, GetPointerType, POINTER_FLAG_INCONTACT, POINTER_INFO,
        POINTER_PEN_INFO,
    };
    use windows::Win32::UI::Input::{
        GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWINPUTDEVICELIST, RIDI_DEVICEINFO,
        RIDI_DEVICENAME, RID_DEVICE_INFO, RIM_TYPEHID,
    };
    use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, EnumThreadWindows, GetClassNameW, GetClientRect, GetSystemMetrics,
        GetWindowTextW, IsWindowVisible, NID_EXTERNAL_PEN, NID_INTEGRATED_PEN,
        NID_INTEGRATED_TOUCH, NID_READY, PEN_FLAG_ERASER, PEN_FLAG_INVERTED, PEN_MASK_PRESSURE,
        PEN_MASK_ROTATION, PEN_MASK_TILT_X, PEN_MASK_TILT_Y, POINTER_INPUT_TYPE, PT_PEN, PT_TOUCH,
        SM_DIGITIZER, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_POINTERDOWN, WM_POINTERUP,
        WM_POINTERUPDATE,
    };

    use super::{
        Device, Digitizer, HookState, PenDevice, Slot, DIGITIZER, LATEST, MESSAGES, PROBES,
        SEEN_PEN, STATE,
    };
    use crate::input::{self, PointerFrame};

    /// comctl32 서브클래스 식별자 — 한 창에 여러 서브클래스가 붙으므로 우리 것에 이름을 준다.
    const SUBCLASS_ID: usize = 0x4C4E_5054; // "LNPT"
    /// pointer id는 `wParam`의 **하위 워드**다(`GET_POINTERID_WPARAM`).
    const POINTER_ID_MASK: usize = 0xFFFF;
    /// 창 클래스 이름/제목 버퍼 길이 — 256이면 어떤 WinUI 창 이름도 들어간다.
    const CLASS_NAME_MAX: usize = 256;

    /// 창을 찾아 **아직 안 걸린 창 전부**에 서브클래스를 건다(실패해도 이유만 남기고 앱은 돈다).
    pub(super) fn install() {
        rescan();
    }

    /// 다시 찾아 **새로 생긴 창**까지 건다 — 카운터는 지우지 않는다(무엇이 왔는지가 정보다).
    pub(super) fn rescan() {
        // 디지타이저 종류는 창과 무관하다 — 볼 때마다 새로 읽어도 싸다(진단 첫 줄의 근거).
        DIGITIZER.set(read_digitizer());

        let mut hooked = 0usize;
        for hwnd in candidates() {
            if hook(hwnd) {
                hooked += 1;
            }
        }
        if hooked > 0 {
            STATE.set(HookState::Hooking);
        } else if !super::state().is_hooking() {
            // 이번에도 못 걸었다 — 이유를 남긴다(창이 아예 없으면 NoWindow).
            let found = PROBES.with(|slots| !slots.borrow().is_empty());
            STATE.set(if found {
                HookState::Failed
            } else {
                HookState::NoWindow
            });
        }
    }

    /// 창 하나에 서브클래스를 건다 — **이미 걸었으면 건너뛴다**(같은 창에 두 번 걸면 정의되지 않는다).
    ///
    /// 참조 데이터로 **목록의 인덱스**를 넘긴다: 창 프로시저가 자기 카운터를 찾는 유일한 길이다.
    fn hook(hwnd: HWND) -> bool {
        let Some((index, already)) = remember(hwnd) else {
            return false;
        };
        if already {
            return true;
        }
        let hooked = unsafe { SetWindowSubclass(hwnd, Some(pointer_proc), SUBCLASS_ID, index) };
        if hooked.0 == 0 {
            return false;
        }
        PROBES.with(|slots| {
            if let Some(slot) = slots.borrow_mut().get_mut(index) {
                slot.hooked = true;
            }
        });
        true
    }

    /// 창을 목록에 **기억한다** — 처음 보면 넣고, 아는 창이면 크기·표시 여부만 새로 읽는다.
    fn remember(hwnd: HWND) -> Option<(usize, bool)> {
        let (visible, size) = read_shape(hwnd);
        PROBES.with(|slots| {
            let mut slots = slots.borrow_mut();
            if let Some(index) = slots.iter().position(|slot| slot.hwnd == hwnd.0 as isize) {
                let slot = &mut slots[index];
                slot.visible = visible;
                slot.size = size;
                return Some((index, slot.hooked));
            }
            slots.push(Slot {
                hwnd: hwnd.0 as isize,
                class: read_class(hwnd),
                title: read_title(hwnd),
                visible,
                size,
                hooked: false,
                messages: Cell::new(0),
                pen: Cell::new(0),
                mouse: Cell::new(0),
            });
            Some((slots.len() - 1, false))
        })
    }

    /// 후보 창 — 이 스레드의 최상위 창 **+ 그 자식 전부**(자식 콘텐츠 창이 여기 있다).
    ///
    /// 거르지 않는다: **어느 창으로 입력이 오는지는 걸어 봐야 안다**. 그래서 창마다 도착 수를
    /// 세고([`Slot::messages`]), 진단 도구가 그 표를 그대로 보여준다.
    fn candidates() -> Vec<HWND> {
        let mut found: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumThreadWindows(
                GetCurrentThreadId(),
                Some(collect),
                LPARAM((&raw mut found) as isize),
            );
        }
        // 열거 중에 자식을 더하므로 **복사본**을 돌린다(같은 목록을 돌며 늘리면 끝이 없다).
        for top in found.clone() {
            unsafe {
                let _ =
                    EnumChildWindows(Some(top), Some(collect), LPARAM((&raw mut found) as isize));
            }
        }
        found
    }

    /// 열거 콜백 — 모으기만 한다.
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let found = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        found.push(hwnd);
        BOOL(1) // 계속 열거한다.
    }

    /// 창 클래스 이름 — 진단에서 **어느 창인지**를 가르는 이름이다.
    fn read_class(hwnd: HWND) -> String {
        let mut buffer = [0u16; CLASS_NAME_MAX];
        let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
        String::from_utf16_lossy(&buffer[..len.max(0) as usize])
    }

    /// 창 제목(보통 빈 문자열이다) — 있으면 클래스 이름보다 잘 읽힌다.
    fn read_title(hwnd: HWND) -> String {
        let mut buffer = [0u16; CLASS_NAME_MAX];
        let len = unsafe { GetWindowTextW(hwnd, &mut buffer) };
        String::from_utf16_lossy(&buffer[..len.max(0) as usize])
    }

    /// 표시 여부 + 클라이언트 크기 (px).
    fn read_shape(hwnd: HWND) -> (bool, (i32, i32)) {
        let visible = unsafe { IsWindowVisible(hwnd) }.0 != 0;
        let mut rect = RECT::default();
        let size = if unsafe { GetClientRect(hwnd, &mut rect) }.is_ok() {
            (rect.right - rect.left, rect.bottom - rect.top)
        } else {
            (0, 0)
        };
        (visible, size)
    }

    /// 시스템 디지타이저 — `SM_DIGITIZER`의 플래그를 이름 있는 값으로 나눈다.
    fn read_digitizer() -> Digitizer {
        let flags = unsafe { GetSystemMetrics(SM_DIGITIZER) } as u32;
        Digitizer {
            present: flags != 0,
            integrated_pen: flags & NID_INTEGRATED_PEN != 0,
            external_pen: flags & NID_EXTERNAL_PEN != 0,
            integrated_touch: flags & NID_INTEGRATED_TOUCH != 0,
            ready: flags & NID_READY != 0,
        }
    }

    /// HID **사용 페이지**: 디지타이저(0x0D) — 펜은 이 페이지에서만 나온다.
    const USAGE_PAGE_DIGITIZER: u16 = 0x0D;

    /// Windows가 아는 **디지타이저 장치**들 — 원시 입력 장치 목록에서 사용 페이지 `0x0D`만 고른다.
    ///
    /// 여기 이름이 곧 **어떤 드라이버가 펜을 내보내는가**다: OEM 드라이버일 수도, OTD의 VMulti
    /// 가상 디지타이저(`VirtualHID`, VID `0x00FF`/PID `0xBACC`)일 수도 있다. 목록이 비어 있으면
    /// OS가 펜을 아예 모르는 것이고 — 그때는 `WM_POINTER`도 나오지 않는다.
    pub(super) fn pen_devices() -> Vec<PenDevice> {
        let size = size_of::<RAWINPUTDEVICELIST>() as u32;
        let mut count = 0u32;
        if unsafe { GetRawInputDeviceList(None, &mut count, size) } == u32::MAX || count == 0 {
            return Vec::new();
        }
        let mut list = vec![RAWINPUTDEVICELIST::default(); count as usize];
        let listed = unsafe { GetRawInputDeviceList(Some(list.as_mut_ptr()), &mut count, size) };
        if listed == u32::MAX {
            return Vec::new();
        }
        list.truncate((listed as usize).min(list.len()));
        list.iter().filter_map(pen_device).collect()
    }

    /// 한 장치가 **디지타이저**면 이름과 용도를 읽는다 — 아니면 `None`(마우스·키보드는 관심 없다).
    fn pen_device(device: &RAWINPUTDEVICELIST) -> Option<PenDevice> {
        if device.dwType != RIM_TYPEHID {
            return None;
        }
        let mut info = RID_DEVICE_INFO {
            cbSize: size_of::<RID_DEVICE_INFO>() as u32,
            ..Default::default()
        };
        let mut size = size_of::<RID_DEVICE_INFO>() as u32;
        let read = unsafe {
            GetRawInputDeviceInfoW(
                Some(device.hDevice),
                RIDI_DEVICEINFO,
                Some((&raw mut info).cast::<core::ffi::c_void>()),
                &mut size,
            )
        };
        if read == u32::MAX {
            return None;
        }
        // 안전: `dwType`이 HID일 때만 `hid`가 채워진다 — 그 조건을 위에서 확인했다.
        let hid = unsafe { info.Anonymous.hid };
        if hid.usUsagePage != USAGE_PAGE_DIGITIZER {
            return None;
        }
        Some(PenDevice {
            name: device_name(device.hDevice),
            usage: hid.usUsage,
        })
    }

    /// 장치 경로(`\\?\hid#vid_...`)를 읽어 사람이 읽는 이름으로 줄인다.
    fn device_name(handle: HANDLE) -> String {
        // 문서의 계약: `pData == NULL`이면 반환값은 **0**이고, 필요한 크기는 `pcbSize`에 담긴다.
        // 그리고 이 명령(`RIDI_DEVICENAME`)만 그 크기가 **문자 수**다(다른 명령은 바이트 수다).
        let mut size = 0u32;
        let _ = unsafe { GetRawInputDeviceInfoW(Some(handle), RIDI_DEVICENAME, None, &mut size) };
        if size == 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; size as usize + 1];
        let mut size = buffer.len() as u32;
        let read = unsafe {
            GetRawInputDeviceInfoW(
                Some(handle),
                RIDI_DEVICENAME,
                Some(buffer.as_mut_ptr().cast::<core::ffi::c_void>()),
                &mut size,
            )
        };
        if read == u32::MAX {
            return String::new();
        }
        let end = buffer
            .iter()
            .position(|ch| *ch == 0)
            .unwrap_or(buffer.len());
        super::compact_device_name(&String::from_utf16_lossy(&buffer[..end]))
    }

    /// 창 프로시저 — `WM_POINTER*`를 **읽고 그대로 넘긴다**(입력을 소비하지 않는다).
    unsafe extern "system" fn pointer_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        if matches!(message, WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP) {
            count(data);
            let id = (wparam.0 & POINTER_ID_MASK) as u32;
            // **종류가 먼저, 접촉이 다음**이다: 펜을 봤다는 사실은 **호버에도** 참이다(WinUI는
            // 접촉 전에도 `WM_POINTERUPDATE`를 보낸다). 접촉 여부는 *프레임*의 자격이지 펜의
            // 자격이 아니다 — 순서를 뒤집으면 공중에 띄운 펜을 "펜이 없다"고 말하게 된다(거짓).
            let device = pointer_kind(id);
            if device == Device::Pen {
                count_pen(data);
            }
            if message != WM_POINTERUPDATE || in_contact(id) {
                record(id, device);
            }
        } else if matches!(message, WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP) {
            // **펜이 안 오고 마우스만 오는 경우**를 세는 곳: 펜을 `SendInput` 마우스로 내보내는
            // 드라이버(예: OpenTabletDriver의 기본 Absolute/Relative Mode)는 `WM_POINTER`를
            // **하나도** 만들지 않는다. 이 숫자가 없으면 그 경우를 "아무 입력도 없다"로 오해한다.
            count_mouse(data);
        }
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }

    /// 이 창이 **마우스** 메시지를 받았다 — 펜이 마우스로 오는 드라이버의 유일한 흔적이다.
    fn count_mouse(index: usize) {
        PROBES.with(|slots| {
            if let Some(slot) = slots.borrow().get(index) {
                slot.mouse.set(slot.mouse.get().wrapping_add(1));
            }
        });
    }

    /// 이 포인터는 무엇인가 — `GetPointerType`의 계약 그대로(`PT_PEN`/`PT_TOUCH`/그 외는 마우스).
    ///
    /// 못 읽으면 **마우스로 본다**: 모르는 입력에 필기 자격을 주지 않는다.
    fn pointer_kind(id: u32) -> Device {
        let mut kind = POINTER_INPUT_TYPE::default();
        match unsafe { GetPointerType(id, &mut kind) } {
            Ok(()) if kind == PT_PEN => Device::Pen,
            Ok(()) if kind == PT_TOUCH => Device::Touch,
            _ => Device::Mouse,
        }
    }

    /// **펜을 봤다** — 접촉과 무관한 사실이다(공중에 띄운 펜도 펜이다).
    ///
    /// 두 곳에 남긴다: 배지(`SEEN_PEN`)와 **어느 창으로 왔는가**(창마다 세면 훅 자리가 드러난다).
    fn count_pen(index: usize) {
        SEEN_PEN.set(true);
        PROBES.with(|slots| {
            if let Some(slot) = slots.borrow().get(index) {
                slot.pen.set(slot.pen.get().wrapping_add(1));
            }
        });
    }

    /// 접촉 중인가 — `POINTER_INFO.pointerFlags & POINTER_FLAG_INCONTACT`.
    ///
    /// 못 읽으면 **막지 않는다**: 정보가 없다고 입력을 버리는 것이 더 나쁘다.
    fn in_contact(id: u32) -> bool {
        let mut info = POINTER_INFO::default();
        if unsafe { GetPointerInfo(id, &mut info) }.is_err() {
            return true;
        }
        info.pointerFlags.0 & POINTER_FLAG_INCONTACT.0 != 0
    }

    /// 이 창이 메시지를 받았다 — **어느 창으로 오는가**가 진단의 답이다(참조 데이터 = 목록 인덱스).
    fn count(index: usize) {
        MESSAGES.set(MESSAGES.get().wrapping_add(1));
        PROBES.with(|slots| {
            if let Some(slot) = slots.borrow().get(index) {
                slot.messages.set(slot.messages.get().wrapping_add(1));
            }
        });
    }

    /// pointer id 하나를 프레임으로 — **여기가 하드웨어와 모델의 경계**다.
    ///
    /// 종류는 이미 읽혔다([`pointer_kind`] — 호버에서도 펜을 세려면 거기서 읽어야 한다).
    /// 여기서는 **자세**만 읽는다.
    fn record(id: u32, device: Device) {
        let (pressure, tilt, pose) = if device == Device::Pen {
            pen_info(id)
        } else {
            (None, None, Pose::default())
        };
        LATEST.set(Some(
            PointerFrame::new(device, pressure, tilt, Instant::now()).with_pen_pose(
                pose.inverted,
                pose.has_eraser,
                pose.rotation,
            ),
        ));
    }

    /// `penFlags`/`PEN_MASK_ROTATION`을 읽은 **펜의 자세** — 플랫폼 상수는 여기서만 만진다.
    #[derive(Clone, Copy, Default)]
    struct Pose {
        /// 뒤집힘(`PEN_FLAG_INVERTED`) — 지우개 끝으로 쓰는 중.
        inverted: bool,
        /// 장치에 지우개 끝이 있는가(`PEN_FLAG_ERASER`).
        has_eraser: bool,
        /// 회전(0~359도, `PEN_MASK_ROTATION`).
        rotation: Option<f32>,
    }

    /// `POINTER_PEN_INFO` → (압력, 틸트, 자세) — **장치가 보고한 것만** `Some`이다(`penMask`).
    fn pen_info(id: u32) -> (Option<f32>, Option<(f32, f32)>, Pose) {
        let mut info = POINTER_PEN_INFO::default();
        if unsafe { GetPointerPenInfo(id, &mut info) }.is_err() {
            return (None, None, Pose::default());
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
        // `penFlags`는 장치의 **자세**다: 뒤집힘(지우개 끝으로 쓰는 중)과 지우개 끝의 유무.
        let pose = Pose {
            inverted: info.penFlags & PEN_FLAG_INVERTED != 0,
            has_eraser: info.penFlags & PEN_FLAG_ERASER != 0,
            // 0도와 "보고하지 않음"은 값으로 구분할 수 없다 — 무선(mask)으로 가른다.
            rotation: (info.penMask & PEN_MASK_ROTATION != 0)
                .then(|| input::rotation_from_raw(info.rotation)),
        };
        (pressure, tilt, pose)
    }
}

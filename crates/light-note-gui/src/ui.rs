//! elm 화면 — 툴바 / 페이지 목록 / 상태바 / 잉크 표면.
//!
//! ## 이 파일이 지키는 경계
//! - **화면은 플랫폼을 모른다** — `view!` 본문에는 WinUI 타입이 하나도 없다.
//! - WinUI 표면은 `<Raw>`로만 붙는다. 그 클로저는 호스트가 등록한 빌더를 부르고,
//!   재료는 호스트가 [`stage_frame`]/[`stage_view`]로 넘긴다
//!   (`<Raw>` 클로저는 props/슬롯 이름을 볼 수 없다 — 예제 08의 제약).
//! - 조각의 **생김새**는 여기 없다 — [`crate::parts`]가 조각마다 파일 하나로 만들고,
//!   숫자·색은 [`crate::style`]의 토큰에만 있다(예제 21~30).
//! - 그래서 화면은 `frame`/`plan`/`mount!`로 **헤드리스로 전부 검증**된다.
//!
//! ## 언어 규칙
//! **사용자가 읽는 문자열은 전부 영어다**(버튼 라벨·상태 문구·힌트·오류·배지).
//! 라벨의 출처는 한 곳([`Intent::label`]/[`Stage::message`]/[`ViewModel`] 메서드)이고,
//! 화면은 그 값을 그대로 쓴다 — 화면과 테스트가 같은 문자열을 본다.
//!
//! ## 상태 소유
//! 문서·도구·페이지는 **호스트가 소유**하고 [`ViewModel`] 하나로 내려온다(예제 11·20).
//! 화면은 그것을 그리고, 의도는 **콜백 하나**([`Intent`])로 올려보낸다 — 버튼마다
//! 콜백 prop을 두던 예전 방식보다 계약이 하나로 줄었다.

use std::cell::RefCell;
use std::rc::Rc;

use elm_magic_windows_reactor::RawSlot;

use crate::geom::{Scale, Size};
use crate::ink::{Style, Tool};
use crate::shape::LiveInk;

/// 화면이 호스트에 올려보내는 의도 — **하나의 콜백 prop으로 전부 온다.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Intent {
    Pen,
    Highlighter,
    Eraser,
    Thinner,
    Thicker,
    Undo,
    Redo,
    Clear,
    Open,
    ExportPng,
    ExportPdf,
    PageAdd,
    PageRemove,
    PagePrev,
    PageNext,
    ZoomIn,
    ZoomOut,
    Retry,
    /// 단축키 패널 열기/닫기 — 화면이 아니라 **호스트가 상태를 소유**한다.
    ToggleHelp,
    /// 단축키 패널 닫기(Esc).
    CloseHelp,
    /// 페이지 목록에서 고른 페이지 — 툴바 버튼이 아니라 **레일의 줄**이 보낸다.
    GoToPage(usize),
    /// **개발자 도구** 열기/닫기 — 태블릿 입력 진단(툴바의 마지막 버튼).
    ToggleDev,
    /// 개발자 도구의 **다시 연결** — OTD 플러그인을 다시 찾는다(방금 설치했거나 데몬을 다시 띄웠을 때).
    Rescan,
}

impl Intent {
    /// 툴바의 **묶음** — 구분선이 들어갈 자리까지 정의에 담는다(플랫폼 무관).
    ///
    /// 그리는 것은 `parts::toolbar`, 검증은 `tests/ui_plan.rs`가 한다.
    ///
    /// 페이지 조작(`PageAdd`/`PageRemove`)은 **여기 없다** — 목록 옆(레일)이 제자리이고,
    /// 툴바가 12개를 넘으면 좁은 창(≈940 DIP)에서 오른쪽 버튼이 잘린다(실제로 겪었다).
    pub const TOOLBAR: [&'static [Intent]; 5] = [
        &[Intent::Pen, Intent::Highlighter, Intent::Eraser],
        &[Intent::Thinner, Intent::Thicker],
        &[Intent::Undo, Intent::Redo, Intent::Clear],
        &[Intent::Open, Intent::ExportPng, Intent::ExportPdf],
        &[Intent::ToggleHelp],
    ];

    /// 레일의 **묶음** — 페이지 목록 옆이 제자리인 조작들.
    ///
    /// 첫 묶음은 페이지 이동·추가·삭제(목록 바로 아래), 둘째 묶음은 배율이다 —
    /// 배율 줄은 사이에 현재 배율(%)이 끼므로 `parts::rail`이 순서대로 그린다.
    pub const RAIL: [&'static [Intent]; 2] = [
        &[
            Intent::PagePrev,
            Intent::PageNext,
            Intent::PageAdd,
            Intent::PageRemove,
        ],
        &[Intent::ZoomOut, Intent::ZoomIn],
    ];

    /// **개발자 도구**의 의도 — 화면(툴바의 진단 버튼 + 진단 패널)이 보낸다.
    ///
    /// 사용자용 기능이 아니라 **진단**이다: 필기가 안 될 때 어느 관문에서 막혔는지 보여주고,
    /// 창을 다시 찾는 버튼을 준다(`digitizer::Digest`). 툴바의 버튼 줄이 아니라 **미리보기 줄**에
    /// 앉는다: 버튼 줄은 라벨 폭이 이미 한 줄 한계에 가까워서(라벨 12개) 하나를 더 넣으면
    /// 좁은 창에서 잘린다.
    pub const DEV: [Intent; 2] = [Intent::ToggleDev, Intent::Rescan];

    /// 화면(조각)이 보낼 수 있는 의도 **전부** — `CloseHelp`는 Esc 전용,
    /// `GoToPage`는 페이지 목록의 줄 전용(값을 들고 온다)이라 여기 없다.
    ///
    /// 의도를 늘리면 [`Intent::label`]의 전수 match가 컴파일을 멈춘다 — 그때 여기 넣을지,
    /// 툴바/레일/개발자 도구에 넣을지를 정한다(`tests/ui_plan.rs`가 셋을 서로 맞춰 본다).
    pub const CHROME: [Intent; 21] = [
        Intent::Pen,
        Intent::Highlighter,
        Intent::Eraser,
        Intent::Thinner,
        Intent::Thicker,
        Intent::Undo,
        Intent::Redo,
        Intent::Clear,
        Intent::Open,
        Intent::ExportPng,
        Intent::ExportPdf,
        Intent::PageAdd,
        Intent::PageRemove,
        Intent::PagePrev,
        Intent::PageNext,
        Intent::ZoomIn,
        Intent::ZoomOut,
        Intent::Retry,
        Intent::ToggleHelp,
        Intent::ToggleDev,
        Intent::Rescan,
    ];

    /// 버튼 라벨이자 테스트가 읽는 이름 — UI와 테스트가 **같은 문자열**을 쓴다.
    ///
    /// **영어만** 쓴다(화면 언어 규칙). 아이콘 버튼의 라벨은 툴팁 + 자동화 이름으로 간다.
    pub const fn label(self) -> &'static str {
        match self {
            Intent::Pen => "Pen",
            Intent::Highlighter => "Highlighter",
            Intent::Eraser => "Eraser",
            Intent::Thinner => "Thinner",
            Intent::Thicker => "Thicker",
            Intent::Undo => "Undo",
            Intent::Redo => "Redo",
            Intent::Clear => "Clear page",
            Intent::Open => "Open PDF",
            Intent::ExportPng => "Save PNG",
            Intent::ExportPdf => "Save PDF",
            Intent::PageAdd => "Add page",
            Intent::PageRemove => "Remove page",
            Intent::PagePrev => "Previous page",
            Intent::PageNext => "Next page",
            Intent::ZoomIn => "Zoom in",
            Intent::ZoomOut => "Zoom out",
            Intent::Retry => "Retry",
            Intent::ToggleHelp => "Keyboard shortcuts",
            Intent::CloseHelp => "Close shortcuts",
            Intent::ToggleDev => "Diagnostics",
            Intent::Rescan => "Reconnect OTD",
            Intent::GoToPage(_) => "Go to page",
        }
    }
}

/// 잉크 표면 구역의 상태 — bool 두 개 대신 enum 하나로 두고 **빠짐없이 분기**한다.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Stage {
    /// 아직 아무것도 안 그림 — 안내 문구를 보여준다.
    #[default]
    Empty,
    /// PDF를 여는 중.
    Loading,
    /// 문서가 준비됨.
    Ready,
    /// 실패 — 복구 동선(다시 시도)을 같은 자리에 둔다.
    Failed(String),
}

impl Stage {
    pub fn is_empty(&self) -> bool {
        matches!(self, Stage::Empty)
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Stage::Loading)
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Stage::Ready)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Stage::Failed(_))
    }

    /// 실패 문구 — 화면에 그대로 보여준다(**영어만**).
    pub fn message(&self) -> String {
        match self {
            Stage::Failed(message) => message.clone(),
            Stage::Loading => "Opening PDF…".to_string(),
            Stage::Empty => "Blank page".to_string(),
            Stage::Ready => "Ready".to_string(),
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Stage::Failed(message) => Some(message),
            _ => None,
        }
    }
}

/// 화면에 내려보내는 값 — 호스트가 채운다(화면은 진실을 소유하지 않는다).
#[derive(Clone, Debug, PartialEq)]
pub struct ViewModel {
    pub tool: Tool,
    pub width_pt: f32,
    pub zoom: f32,
    pub page: usize,
    pub page_count: usize,
    pub stroke_count: usize,
    /// 지금 라이브 도형 수(안 구운 꼬리 + 진행 중 획).
    pub live_shapes: usize,
    /// 구운 접두사가 반영한 획 수.
    pub baked: usize,
    pub can_undo: bool,
    pub can_redo: bool,
    pub dirty: bool,
    pub title: String,
    pub pdf_name: String,
    pub stage: Stage,
    pub status: String,
    /// 입력 장치 배지 — "무엇으로 그리는 중인가"(호스트가 디지타이저 상태로 채운다).
    ///
    /// 펜이 감지되지 않으면 그 사실이 **화면에 그대로** 보인다: 필기가 안 될 때
    /// 사용자가 원인을 알 수 있는 유일한 단서다(`digitizer::HookState`).
    pub input: String,
    /// 단축키 패널이 열려 있는가 — **호스트가 소유**한다(화면은 그리기만 한다).
    pub help: bool,
    /// **개발자 도구**(디지타이저 진단)가 열려 있는가 — 호스트가 소유한다.
    pub dev: bool,
    /// 진단 줄 — `(이름, 값)` 쌍. 문장은 **호스트가 만들고**(`digitizer::Digest::report`)
    /// 조각은 표로 그리기만 한다: 진단이 화면 언어 규칙을 조각 밖에서 지킨다.
    pub diag: Vec<(String, String)>,
    /// 창의 클라이언트 크기 (DIP) — 잉크 영역이 창 안에 들어오게 하는 **유일한 근거**다.
    ///
    /// elm 트리는 `StackPanel`뿐이라 자식 높이가 묶이지 않는다(종이가 창보다 크면
    /// 크롬이 화면 밖으로 밀린다). 그래서 호스트가 창 크기를 관측해
    /// [`crate::parts::paper::scroll`]의 높이로 내려준다 — 그제야 스크롤이 생긴다.
    pub viewport: (f64, f64),
    pub page_labels: Vec<String>,
}

impl Default for ViewModel {
    fn default() -> Self {
        Self {
            tool: Tool::Pen,
            width_pt: Style::PEN_WIDTH_PT,
            zoom: 100.0,
            page: 0,
            page_count: 1,
            stroke_count: 0,
            live_shapes: 0,
            baked: 0,
            can_undo: false,
            can_redo: false,
            dirty: false,
            title: "Untitled".to_string(),
            pdf_name: String::new(),
            stage: Stage::Empty,
            status: String::new(),
            input: String::new(),
            help: false,
            dev: false,
            diag: Vec::new(),
            viewport: (1280.0, 800.0),
            page_labels: Vec::new(),
        }
    }
}

impl ViewModel {
    pub fn tool_label(&self) -> &'static str {
        self.tool.label()
    }

    pub fn hint(&self) -> &'static str {
        Style::hint(self.tool)
    }

    pub fn zoom_label(&self) -> String {
        format!("{:.0}%", self.zoom)
    }

    /// 파이프라인 상태 한 줄 — 구운 접두사와 안 구운 꼬리의 크기.
    pub fn pipeline_label(&self) -> String {
        format!(
            "Base {} strokes · live {} shapes",
            self.baked, self.live_shapes
        )
    }
}

/// 페이지 목록 라벨 — 사이드바가 쓴다(**영어만**).
pub fn page_label(index: usize, strokes: usize) -> String {
    format!("Page {} · {} strokes", index + 1, strokes)
}

/// 한 프레임에 ④-UI가 그릴 재료. **`Rc`라 복사가 없다**(꼬리는 통째로 공유된다).
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub base: crate::canvas::Base,
    pub tail: Rc<[LiveInk]>,
    pub drawing: Option<Rc<LiveInk>>,
    pub size: Size,
    pub scale: Scale,
    pub baked: usize,
    pub strokes: usize,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            base: crate::canvas::Base::blank(),
            tail: Rc::from(Vec::new()),
            drawing: None,
            size: Size::A4,
            scale: Scale::default(),
            baked: 0,
            strokes: 0,
        }
    }
}

/// `<Raw>`가 값을 채우는 슬롯 — 어댑터의 `Option<View>`.
pub type SurfaceSlot = RawSlot;

/// 호스트가 등록하는 표면 빌더 — [`Frame`]과 [`ViewModel`]을 WinUI 트리로 바꾼다.
///
/// `fn`이 아니라 `Rc<dyn Fn>`인 이유: 빌더가 **호스트의 `LocalSender`를 캡처**해야
/// 포인터 이벤트를 메시지로 올려보낼 수 있다(예제 08). 값이 함께 가는 이유: 잉크 영역의
/// **높이**(`view.viewport`)와 빈 상태 안내가 표면 안에서 필요하기 때문이다(둘 다 표면의
/// 내용이지 호스트의 관심사가 아니다).
pub type SurfaceBuilder = Rc<dyn Fn(&Frame, &ViewModel) -> SurfaceSlot>;

/// 화면이 호스트에 요청하는 **조각** — `<Raw>` 하나가 조각 하나를 그린다.
///
/// 화면은 "무엇이 필요한가"만 알고 "어떻게 보이는가"는 모른다: 생김새는
/// [`crate::parts`]가 조각마다 파일 하나로 만들고, 숫자·색은 [`crate::style`]의
/// 토큰에만 있다. 그래서 **스타일을 바꿔도 이 파일은 손대지 않는다**.
///
/// 버튼도 조각이 만든다 — `<Raw>` 안에서 elm 콜백은 못 쓰지만 **호스트의
/// [`IntentSink`]는 쓸 수 있다**(표면이 포인터를 다루는 것과 같은 방법).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    /// **네이티브 타이틀바** — 문서 제목 · 배경 요약 · 배지(WinUI `TitleBar`).
    TitleBar,
    /// 툴바 — 아이콘 버튼 묶음 + 잉크 미리보기.
    Toolbar,
    /// 좌측 레일 — 페이지 목록 + 페이지/배율 조작.
    Rail,
    /// 상태바 — 상태 문구 + 사실 요약.
    Status,
    /// 여는 중 — 스피너 + 문구.
    Loading,
    /// 실패 — 이유 + 복구 버튼.
    Failure,
    /// 단축키 패널(호스트가 열고 닫는다).
    Shortcuts,
    /// **개발자 도구** — 디지타이저 진단 표 + 리스캔(호스트가 열고 닫는다).
    DevTools,
}

/// 의도를 호스트로 보내는 통로 — 조각이 만든 **버튼이 이것을 캡처**한다.
///
/// `Rc<dyn Fn>`인 이유: 버튼 클로저 여러 개가 같은 통로를 나눠 쓴다(복제는 `Rc` 복사).
pub type IntentSink = Rc<dyn Fn(Intent)>;

/// 호스트가 등록하는 조각 빌더 — [`Part`]와 [`ViewModel`]을 WinUI 트리로 바꾼다.
///
/// [`SurfaceBuilder`]와 같은 이유로 `Rc<dyn Fn>`이다(테스트가 기록용 빌더를 끼운다).
pub type PartBuilder = Rc<dyn Fn(Part, &ViewModel, &IntentSink) -> SurfaceSlot>;

thread_local! {
    /// 이번 발행에서 `<Raw>`가 읽어 갈 재료 (UI 스레드 전용 — `Rc`를 담는다).
    static PENDING_FRAME: RefCell<Option<Frame>> = const { RefCell::new(None) };
    /// 이번 발행에서 조각이 읽어 갈 값 — 호스트가 `view()` 안에서 넣는다.
    static PENDING_VIEW: RefCell<Option<ViewModel>> = const { RefCell::new(None) };
    /// 등록된 표면 빌더.
    static SURFACE_BUILDER: RefCell<Option<SurfaceBuilder>> = const { RefCell::new(None) };
    /// 등록된 조각 빌더.
    static PART_BUILDER: RefCell<Option<PartBuilder>> = const { RefCell::new(None) };
    /// 조각이 만든 버튼이 의도를 보내는 통로.
    static INTENT_SINK: RefCell<Option<IntentSink>> = const { RefCell::new(None) };
}

/// 이번 발행에서 `<Raw>`가 그릴 재료를 넣는다 — 호스트가 `view()` 안에서 부른다.
pub fn stage_frame(frame: Frame) {
    PENDING_FRAME.with(|slot| *slot.borrow_mut() = Some(frame));
}

/// 스테이징된 재료 — **비우지 않는다**(프레임이 여러 번 만들어져도 모두 같은 것을 본다).
pub fn pending_frame() -> Option<Frame> {
    PENDING_FRAME.with(|slot| slot.borrow().clone())
}

/// 발행이 끝나면 비운다(다음 프레임의 재료와 섞이지 않게).
pub fn clear_frame() {
    PENDING_FRAME.with(|slot| *slot.borrow_mut() = None);
}

/// 표면 빌더를 등록한다 — 호스트가 `view()`마다 현재 sender로 다시 부른다.
pub fn set_surface_builder(builder: SurfaceBuilder) {
    SURFACE_BUILDER.with(|slot| *slot.borrow_mut() = Some(builder));
}

/// 등록을 해제한다(테스트 격리).
pub fn clear_surface_builder() {
    SURFACE_BUILDER.with(|slot| *slot.borrow_mut() = None);
}

/// 현재 표면 빌더.
pub fn surface_builder() -> Option<SurfaceBuilder> {
    SURFACE_BUILDER.with(|slot| slot.borrow().clone())
}

/// 조각이 그릴 값을 넣는다 — 호스트가 `view()` 안에서 부른다.
pub fn stage_view(view: ViewModel) {
    PENDING_VIEW.with(|slot| *slot.borrow_mut() = Some(view));
}

/// 스테이징된 값 — **비우지 않는다**(조각이 여러 번 그려도 같은 것을 본다).
pub fn pending_view() -> Option<ViewModel> {
    PENDING_VIEW.with(|slot| slot.borrow().clone())
}

/// 발행이 끝나면 비운다(테스트 격리).
pub fn clear_view() {
    PENDING_VIEW.with(|slot| *slot.borrow_mut() = None);
}

/// 조각 빌더를 등록한다 — 호스트가 `view()`마다 현재 sender로 다시 부른다.
pub fn set_part_builder(builder: PartBuilder) {
    PART_BUILDER.with(|slot| *slot.borrow_mut() = Some(builder));
}

/// 등록을 해제한다(테스트 격리).
pub fn clear_part_builder() {
    PART_BUILDER.with(|slot| *slot.borrow_mut() = None);
}

/// 현재 조각 빌더.
pub fn part_builder() -> Option<PartBuilder> {
    PART_BUILDER.with(|slot| slot.borrow().clone())
}

/// 의도 통로를 등록한다 — 호스트가 `view()`마다 현재 sender로 다시 부른다.
pub fn set_intent_sink(sink: IntentSink) {
    INTENT_SINK.with(|slot| *slot.borrow_mut() = Some(sink));
}

/// 등록을 해제한다(테스트 격리).
pub fn clear_intent_sink() {
    INTENT_SINK.with(|slot| *slot.borrow_mut() = None);
}

/// 현재 의도 통로 — 조각이 만든 버튼이 캡처한다.
pub fn intent_sink() -> Option<IntentSink> {
    INTENT_SINK.with(|slot| slot.borrow().clone())
}

/// `<Raw>` 슬롯 하나를 채운다 — 값·빌더·통로가 **다 셋** 있어야 그린다.
///
/// 없으면 빈 슬롯이다: 아직 준비되지 않았다는 뜻이고(예: 호스트 없이 계획만 검사할 때),
/// WinUI에서는 그 자리가 비어 보인다(거짓 그림을 그리지 않는다).
fn part(part: Part) -> SurfaceSlot {
    match (pending_view(), part_builder(), intent_sink()) {
        (Some(view), Some(builder), Some(sink)) => builder(part, &view, &sink),
        _ => None,
    }
}

elm_magic::view! {
    /// **네이티브 타이틀바** — 문서 제목 · 배경 요약 · 배지.
    ///
    /// 리액터가 이 요소를 창의 타이틀바로 붙인다(캡션 버튼·드래그 영역·Mica가 따라온다) —
    /// 그래서 크롬이 **우리 것이 아니라 WinUI 것**이 되고, 모양은 조각이 정한다.
    pub fn TitleBar() {
        <Raw>|out: &mut SurfaceSlot| {
            *out = part(Part::TitleBar);
        }</Raw>
    }
}

elm_magic::view! {
    /// 툴바 — 아이콘 버튼 묶음 + 잉크 미리보기.
    ///
    /// **버튼도 여기서 만든다**(조각 안에서 `IntentSink`를 캡처한다) — elm 기본 위젯을
    /// 쓰지 않는 이유는 스타일을 줄 통로가 `<Raw>`뿐이기 때문이다.
    pub fn Toolbar() {
        <Raw>|out: &mut SurfaceSlot| {
            *out = part(Part::Toolbar);
        }</Raw>
    }
}

elm_magic::view! {
    /// 좌측 레일 — 페이지 목록 + 페이지/배율 조작.
    pub fn Rail() {
        <Raw>|out: &mut SurfaceSlot| {
            *out = part(Part::Rail);
        }</Raw>
    }
}

elm_magic::view! {
    /// 상태바 — 상태 문구 + 사실 요약.
    pub fn Status() {
        <Raw>|out: &mut SurfaceSlot| {
            *out = part(Part::Status);
        }</Raw>
    }
}

elm_magic::view! {
    /// 여는 중 — 스피너 + 문구.
    pub fn Loading() {
        <Raw>|out: &mut SurfaceSlot| {
            *out = part(Part::Loading);
        }</Raw>
    }
}

elm_magic::view! {
    /// 실패 — 이유(`Stage::Failed`) + 복구 버튼.
    pub fn Failure() {
        <Raw>|out: &mut SurfaceSlot| {
            *out = part(Part::Failure);
        }</Raw>
    }
}

elm_magic::view! {
    /// 단축키 패널 — 호스트가 `view.help`로 열고 닫는다.
    pub fn Shortcuts() {
        <Raw>|out: &mut SurfaceSlot| {
            *out = part(Part::Shortcuts);
        }</Raw>
    }
}

elm_magic::view! {
    /// 개발자 도구 — 디지타이저 진단 표 + 리스캔(호스트가 `view.dev`로 연다).
    pub fn DevTools() {
        <Raw>|out: &mut SurfaceSlot| {
            *out = part(Part::DevTools);
        }</Raw>
    }
}

elm_magic::view! {
    /// 잉크 표면 — 종이 + (빈 상태 안내). **잉크 영역 전체**가 이 조각이다.
    ///
    /// 클로저 토큰은 매크로가 그대로 삽입하므로 props/슬롯 이름을 쓸 수 없다 — 재료(프레임)와
    /// 값(뷰모델)은 호스트가 [`stage_frame`]/[`stage_view`]로 넘긴 곳에서 읽는다(예제 08).
    pub fn InkSurface() {
        <Raw>|out: &mut SurfaceSlot| {
            if let (Some(frame), Some(view), Some(builder)) =
                (pending_frame(), pending_view(), surface_builder())
            {
                *out = builder(&frame, &view);
            }
        }</Raw>
    }
}

elm_magic::view! {
    /// light-note 화면 — 타이틀바 / 툴바 / 레일 / 잉크 표면 / 상태 띠 / 단축키 패널.
    ///
    /// ## 이 본문에 있는 것과 없는 것
    /// - **있는 것**: 구조(어떤 조각이 어디에 있는가)와 분기(단계·단축키 패널).
    /// - **없는 것**: 버튼·색·간격·글자 크기 — 전부 조각([`crate::parts`])과
    ///   토큰([`crate::style`])에 있다. elm 기본 위젯(`<Button>`)도 쓰지 않는다:
    ///   스타일을 줄 통로가 `<Raw>`뿐이기 때문이다(예제 16·26).
    ///
    /// ## 상태
    /// 화면은 상태를 **소유하지 않는다** — 값은 [`ViewModel`]으로 내려오고(호스트가 진실을
    /// 소유) 의도는 `on_intent` 하나로 올라간다. 단축키(F1/Esc)는 elm이 선언하되
    /// **의도로 바꿔** 호스트에 넘긴다(패널의 열림 상태도 호스트가 소유한다).
    pub fn Screen(view: ViewModel, on_intent: fn(Intent)) {
        // 단축키는 elm이 선언한다 — 상태는 호스트가 소유하므로 **의도로 올려보낸다**.
        // (`AcceleratorKey`에 F1/Esc가 없다: 가속기는 Ctrl+±/Ctrl+Enter뿐이다.)
        on_key("F1") { on_intent(Intent::ToggleHelp) }
        on_key("Escape") { on_intent(Intent::CloseHelp) }

        <Col>
            // ── 네이티브 타이틀바 (조각): 제목 + 배지 + 캡션 버튼 ──
            <TitleBar />

            // ── 툴바 (조각): 아이콘 버튼 + 잉크 미리보기 ────────
            <Toolbar />

            // ── 정보 띠 (조각): 상태 문구 + 힌트 + 사실 요약 ────
            // 본문 **위**에 둔다: 아래에 두면 잉크 영역 높이 추정이 틀릴 때 화면 밖으로
            // 밀려 아무도 못 본다(추정이 틀리면 잘리는 것은 본문 아래쪽뿐이어야 한다).
            <Status />

            // ── 개발자 도구 (조각): 디지타이저 진단 — **필기가 안 될 때 여기** ──
            // 정보 띠와 같은 이유로 본문 **위**에 둔다(진단은 필요할 때 보여야 한다).
            <If when={view.dev}>
                <DevTools />
            </If>

            // ── 본문: 레일(조각) + 잉크 표면(조각) ─────────────
            <Row>
                <Rail />
                <Col>
                    <Switch on={view.stage}>
                        <Case when={Stage::Loading}>
                            <Loading />
                        </Case>
                        <Case when={Stage::Empty}>
                            <InkSurface />
                        </Case>
                        <Case when={Stage::Ready}>
                            <InkSurface />
                        </Case>
                        <Case when={Stage::Failed(_)}>
                            <Failure />
                        </Case>
                    </Switch>
                </Col>
            </Row>

            // ── 단축키 패널 (조각): 열림 상태는 호스트가 소유 ────
            <If when={view.help}>
                <Shortcuts />
            </If>
        </Col>
    }
}

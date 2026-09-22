//! elm 화면 — 툴바 / 페이지 목록 / 상태바 / 잉크 표면.
//!
//! ## 이 파일이 지키는 경계
//! - **화면은 플랫폼을 모른다** — `view!` 본문에는 WinUI 타입이 하나도 없다.
//! - WinUI 표면은 `<Raw>` **하나**로만 붙는다. 그 클로저는 호스트가 등록한
//!   [`SurfaceBuilder`]를 부르고, 재료는 호스트가 [`stage_frame`]으로 넘긴다
//!   (`<Raw>` 클로저는 props/슬롯 이름을 볼 수 없다 — 예제 08의 제약).
//! - 그래서 화면은 `frame`/`plan`/`mount!`로 **헤드리스로 전부 검증**된다.
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
}

impl Intent {
    /// 버튼 라벨이자 테스트가 읽는 이름 — UI와 테스트가 **같은 문자열**을 쓴다.
    pub const fn label(self) -> &'static str {
        match self {
            Intent::Pen => "펜",
            Intent::Highlighter => "형광펜",
            Intent::Eraser => "지우개",
            Intent::Thinner => "가늘게",
            Intent::Thicker => "굵게",
            Intent::Undo => "되돌리기",
            Intent::Redo => "다시하기",
            Intent::Clear => "페이지 비우기",
            Intent::Open => "PDF 열기",
            Intent::ExportPng => "PNG 저장",
            Intent::ExportPdf => "PDF 저장",
            Intent::PageAdd => "페이지 추가",
            Intent::PageRemove => "페이지 삭제",
            Intent::PagePrev => "이전 페이지",
            Intent::PageNext => "다음 페이지",
            Intent::ZoomIn => "확대",
            Intent::ZoomOut => "축소",
            Intent::Retry => "다시 시도",
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

    /// 실패 문구 — 화면에 그대로 보여준다.
    pub fn message(&self) -> String {
        match self {
            Stage::Failed(message) => message.clone(),
            Stage::Loading => "PDF를 읽는 중입니다".to_string(),
            Stage::Empty => "빈 페이지".to_string(),
            Stage::Ready => "준비됨".to_string(),
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
            title: "무제".to_string(),
            pdf_name: String::new(),
            stage: Stage::Empty,
            status: String::new(),
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
        format!("베이스 {}획 · 라이브 {}도형", self.baked, self.live_shapes)
    }
}

/// 페이지 목록 라벨 — 사이드바가 쓴다.
pub fn page_label(index: usize, strokes: usize) -> String {
    format!("{}페이지 · 획 {}개", index + 1, strokes)
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

/// 호스트가 등록하는 표면 빌더 — [`Frame`]을 WinUI 트리로 바꾼다.
///
/// `fn`이 아니라 `Rc<dyn Fn>`인 이유: 빌더가 **호스트의 `LocalSender`를 캡처**해야
/// 포인터 이벤트를 메시지로 올려보낼 수 있다(예제 08).
pub type SurfaceBuilder = Rc<dyn Fn(&Frame) -> SurfaceSlot>;

thread_local! {
    /// 이번 발행에서 `<Raw>`가 읽어 갈 재료 (UI 스레드 전용 — `Rc`를 담는다).
    static PENDING_FRAME: RefCell<Option<Frame>> = const { RefCell::new(None) };
    /// 등록된 표면 빌더.
    static SURFACE_BUILDER: RefCell<Option<SurfaceBuilder>> = const { RefCell::new(None) };
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

elm_magic::view! {
    /// 잉크 표면만 담는 작은 컴포넌트.
    ///
    /// `<Raw>`를 **한 곳에만** 두려고 분리했다(빈 상태와 준비 상태가 같은 표면을 쓴다).
    /// 클로저 토큰은 매크로가 그대로 삽입하므로 props/슬롯 이름을 쓸 수 없다 — 재료는
    /// 호스트가 [`stage_frame`]으로 넘긴 곳에서 읽는다(예제 08의 제약).
    pub fn InkSurface() {
        <Raw>|out: &mut SurfaceSlot| {
            if let (Some(frame), Some(builder)) = (pending_frame(), surface_builder()) {
                *out = builder(&frame);
            }
        }</Raw>
    }
}

elm_magic::view! {
    /// light-note 화면 — 툴바 / 페이지 목록 / 잉크 표면 / 상태바.
    ///
    /// 값은 전부 [`ViewModel`] 하나로 내려오고(호스트가 진실을 소유), 의도는
    /// `on_intent` **하나**로 올라간다. 이 규칙 하나로 "WinUI 상태"와 "elm 상태"가
    /// 어긋나지 않는다.
    pub fn Screen(view: ViewModel, on_intent: fn(Intent), help = false) {
        // 단축키는 elm이 선언하고 호스트가 밀어 준다(예제 12).
        on_key("F1") { help = !help }
        on_key("Escape") { help = false }

        <Col>
            // ── 툴바 ────────────────────────────────────────────
            <Row>
                <Button on_click={on_intent(Intent::Pen)}>"펜"</Button>
                <Button on_click={on_intent(Intent::Highlighter)}>"형광펜"</Button>
                <Button on_click={on_intent(Intent::Eraser)}>"지우개"</Button>
                <Divider />
                <Button on_click={on_intent(Intent::Thinner)}>"가늘게"</Button>
                "굵기 {view.width_pt:.1}pt"
                <Button on_click={on_intent(Intent::Thicker)}>"굵게"</Button>
                <Divider />
                <Button on_click={on_intent(Intent::Undo)}>"되돌리기"</Button>
                <Button on_click={on_intent(Intent::Redo)}>"다시하기"</Button>
                <Button on_click={on_intent(Intent::Clear)}>"페이지 비우기"</Button>
                <Divider />
                <Button on_click={on_intent(Intent::Open)}>"PDF 열기"</Button>
                <Button on_click={on_intent(Intent::ExportPng)}>"PNG 저장"</Button>
                <Button on_click={on_intent(Intent::ExportPdf)}>"PDF 저장"</Button>
                <Divider />
                // 단축키 안내는 **elm이 소유한 화면 상태**라 호스트를 거치지 않는다.
                <Button on_click={help = !help}>"단축키"</Button>
            </Row>

            <Row>
                // ── 페이지 사이드바 ──────────────────────────────
                <Col>
                    <Strong>"{view.title}"</Strong>
                    <If when={view.pdf_name.is_empty() == false}>
                        <Text>"배경: {view.pdf_name}"</Text>
                    </If>
                    "페이지 {view.page + 1} / {view.page_count}"
                    <For each={view.page_labels} as={label} key={label}>
                        <Text>"{label}"</Text>
                    </For>
                    <Button on_click={on_intent(Intent::PagePrev)}>"이전 페이지"</Button>
                    <Button on_click={on_intent(Intent::PageNext)}>"다음 페이지"</Button>
                    <Button on_click={on_intent(Intent::PageAdd)}>"페이지 추가"</Button>
                    <Button on_click={on_intent(Intent::PageRemove)}>"페이지 삭제"</Button>
                    <Divider />
                    <Button on_click={on_intent(Intent::ZoomOut)}>"축소"</Button>
                    "배율 {view.zoom_label()}"
                    <Button on_click={on_intent(Intent::ZoomIn)}>"확대"</Button>
                </Col>

                // ── 잉크 표면 ─────────────────────────────────────
                <Col>
                    <Switch on={view.stage}>
                        <Case when={Stage::Loading}>
                            <Row>
                                <Spinner />
                                <Text>"PDF 여는 중…"</Text>
                            </Row>
                        </Case>
                        <Case when={Stage::Empty}>
                            <Col>
                                <Text>"여기에 필기하세요 — 펜으로 그리면 됩니다"</Text>
                                <InkSurface />
                            </Col>
                        </Case>
                        <Case when={Stage::Ready}>
                            <InkSurface />
                        </Case>
                        <Case when={Stage::Failed(message)}>
                            <Col>
                                <Banner kind="error">{message}</Banner>
                                <Button on_click={on_intent(Intent::Retry)}>"다시 시도"</Button>
                            </Col>
                        </Case>
                    </Switch>
                </Col>
            </Row>

            // ── 상태바 ──────────────────────────────────────────
            <Divider />
            <Row>
                "도구: {view.tool_label()}"
                "획 {view.stroke_count}개"
                "{view.pipeline_label()}"
                <If when={view.can_undo}><Text>"되돌릴 수 있음"</Text></If>
                <If when={view.can_redo}><Text>"다시할 수 있음"</Text></If>
                <If when={view.dirty}><Text>"저장 안 됨"</Text></If>
            </Row>
            <Text>"{view.status}"</Text>
            <Text>"{view.hint()}"</Text>

            // ── 단축키 안내 (elm이 소유한 화면 상태) ─────────────
            <If when={help}>
                <Col>
                    <Strong>"단축키"</Strong>
                    <Text>"F1 안내 열기/닫기 · Esc 닫기"</Text>
                    <Text>"Ctrl+더하기 / Ctrl+빼기 확대·축소 (호스트 가속기)"</Text>
                    <Text>"Ctrl+Enter 페이지 추가 (호스트 가속기)"</Text>
                    <Text>"나머지 기능은 툴바 버튼으로"</Text>
                </Col>
            </If>
        </Col>
    }
}

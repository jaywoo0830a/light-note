//! elm-magic 화면 — 툴바 / 페이지 목록 / 상태바 / 잉크 표면.
//!
//! ## 이 파일이 지키는 경계
//! - **화면은 플랫폼을 모른다** — `view!` 본문에는 WinUI 타입이 하나도 없다.
//! - WinUI 표면은 `<Raw>` 하나로만 붙는다. 그 클로저는 **호스트가 등록한
//!   [`SurfaceBuilder`]** 를 부른다(여기서 WinUI를 직접 알지 않는다).
//! - 그래서 이 화면은 리눅스에서도 `mount!`/`frame`/`plan`으로 전부 검증된다.
//!
//! ## 상태 소유
//! 문서·도구·페이지는 **호스트가 소유**하고 props로 내려온다(예제 11·20의 패턴).
//! 화면은 그것을 그리고, 의도(intent)를 콜백 prop으로 올려보낸다.

use crate::doc::Document;
use crate::ink::{StrokeStyle, Tool};
use crate::surface::{InkSurface, SurfaceLine};
use elm_magic::Callback;

/// `<Raw>`가 값을 채우는 슬롯.
///
/// Windows에서는 어댑터의 `RawSlot`(= `Option<windows_reactor::View>`)이고,
/// 다른 플랫폼(헤드리스 테스트)에서는 자리표시자다. **같은 소스가 양쪽에서 컴파일**된다.
#[cfg(windows)]
pub type SurfaceSlot = elm_magic_windows_reactor::RawSlot;

/// 헤드리스(비-Windows) 자리표시자 — 테스트는 `Some(())`로 "빌더가 불렸다"를 확인한다.
#[cfg(not(windows))]
pub type SurfaceSlot = Option<()>;

/// 표면을 그리는 데 필요한 재료 — `<Raw>` 클로저가 호스트에 넘긴다.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceData {
    /// 확정 스트로크를 그린 PNG(투명 배경). 스트로크가 없으면 `None`.
    pub png: Option<Vec<u8>>,
    /// 진행 중인 획의 선분들.
    pub lines: Vec<SurfaceLine>,
    /// 표면 크기(DIP).
    pub width: u16,
    pub height: u16,
    /// 1pt당 픽셀 수.
    pub scale: f32,
}

/// 호스트가 등록하는 표면 빌더 — `SurfaceData`를 WinUI 컨트롤 트리로 바꾼다.
///
/// `fn`이 아니라 `Rc<dyn Fn>`인 이유: 빌더가 **호스트의 `LocalSender`를 캡처**해야
/// 포인터 이벤트를 메시지로 올려보낼 수 있다(예제 08의 "값이 필요하면 캡처한다").
/// 호스트는 매 `view()`에서 현재 sender로 다시 등록한다.
pub type SurfaceBuilder = std::rc::Rc<dyn Fn(&SurfaceData) -> SurfaceSlot>;

thread_local! {
    /// 등록된 표면 빌더 (UI 스레드 전용 — `Rc`를 담아야 하므로 `Mutex` 정적이 아니다).
    static SURFACE_BUILDER: std::cell::RefCell<Option<SurfaceBuilder>> =
        const { std::cell::RefCell::new(None) };
}

/// 표면 빌더를 등록한다 — 호스트가 `view()`마다 현재 sender로 다시 부른다.
///
/// 등록하지 않으면 `<Raw>`는 **아무것도 하지 않는다**(헤드리스 테스트 경로).
pub fn set_surface_builder(builder: SurfaceBuilder) {
    SURFACE_BUILDER.with(|slot| *slot.borrow_mut() = Some(builder));
}

/// 등록을 해제한다(테스트 격리용).
pub fn clear_surface_builder() {
    SURFACE_BUILDER.with(|slot| *slot.borrow_mut() = None);
}

/// 현재 표면 빌더.
pub fn surface_builder() -> Option<SurfaceBuilder> {
    SURFACE_BUILDER.with(|slot| slot.borrow().clone())
}

/// 화면이 구분하는 상태 (예제 19의 3분기 패턴).
///
/// "빈 페이지"와 "아직 안 옴"과 "실패"는 **다른 화면**이다 — 그래서 bool 두 개가
/// 아니라 enum 하나로 두고 `<Switch>`로 빠짐없이 분기한다.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Phase {
    /// 아직 아무것도 안 그림 — 안내 문구를 보여준다.
    #[default]
    Empty,
    /// PDF를 여는 중.
    Loading,
    /// 문서가 준비됨(그림이 있거나 PDF 배경이 있음).
    Ready,
    /// 실패 — **복구 동선(다시 시도)을 같은 자리에** 둔다.
    Failed(String),
}

impl Phase {
    pub fn is_ready(&self) -> bool {
        matches!(self, Phase::Ready)
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Phase::Loading)
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Phase::Failed(message) => Some(message),
            _ => None,
        }
    }
}

thread_local! {
    /// 이번 발행(publish)에서 `<Raw>`가 읽어 갈 표면 재료.
    ///
    /// **왜 전역인가**: `<Raw>`의 클로저 토큰은 매크로가 그대로 삽입하므로
    /// prop/슬롯 이름이 클로저 안에서 해석되지 않는다(사양서 7.3의 제약).
    /// 그래서 호스트가 **발행 직전에** 재료를 넣고 클로저가 꺼내 쓴다.
    /// UI 스레드 하나에서만 쓰이므로 `thread_local`이 맞다.
    static PENDING_SURFACE: std::cell::RefCell<Option<SurfaceData>> =
        const { std::cell::RefCell::new(None) };
}

/// 이번 발행에서 `<Raw>`가 그릴 표면 재료를 넣는다 — 호스트가 `view()` 직전에 부른다.
pub fn stage_surface(data: SurfaceData) {
    PENDING_SURFACE.with(|slot| *slot.borrow_mut() = Some(data));
}

/// 스테이징된 표면 재료를 꺼낸다(`<Raw>`가 호출). 꺼내면 비워진다.
pub fn surface_pending() -> Option<SurfaceData> {
    PENDING_SURFACE.with(|slot| slot.borrow_mut().take())
}

/// 스테이징만 비운다(테스트 격리/발행 취소).
pub fn clear_staged_surface() {
    PENDING_SURFACE.with(|slot| *slot.borrow_mut() = None);
}

elm_magic::view! {
    /// 잉크 표면만 담는 작은 컴포넌트.
    ///
    /// `<Raw>`를 **한 곳에만** 두려고 분리했다(빈 상태와 준비 상태가 같은 표면을 쓴다).
    /// 클로저 토큰은 매크로가 그대로 삽입하므로 props/슬롯 이름을 쓸 수 없다 —
    /// 그래서 재료는 호스트가 [`stage_surface`]로 넘긴 전역에서 읽는다.
    pub fn InkSurfaceView() {
        <Raw>|out: &mut SurfaceSlot| {
            if let Some(data) = surface_pending() {
                if let Some(builder) = surface_builder() {
                    *out = builder(&data);
                }
            }
        }</Raw>
    }
}

elm_magic::view! {
    /// light-note 화면 — 툴바 / 페이지 목록 / 잉크 표면 / 상태바.
    ///
    /// 값은 전부 **props**(호스트가 진실을 소유)이고, 화면은 의도만 콜백으로
    /// 올려보낸다. 이 규칙 하나로 "WinUI 상태"와 "elm 상태"가 어긋나지 않는다.
    ///
    /// `pub`인 이유: 호스트(WinUI)와 테스트가 **다른 크레이트**에서 이 화면을 쓴다.
    pub fn NoteApp(
        // ── 도구/문서 (호스트 소유) ──
        tool: Tool,
        width_pt: f32,
        zoom: f32,
        page: usize,
        page_count: usize,
        stroke_count: usize,
        can_undo: bool,
        can_redo: bool,
        dirty: bool,
        doc_title: String,
        pdf_name: String,
        has_pdf: bool,
        // 화면 상태 (예제 19) — bool 두 개 대신 enum 하나.
        phase: Phase,
        status: String,
        page_labels: Vec<String>,
        // ── 잉크 표면 표시용 (재료는 호스트가 stage_surface로 넘긴다) ──
        surface_width: u16,
        surface_height: u16,
        surface_lines: usize,
        // ── 의도 (elm → 호스트) ──
        // **어댑터 제약**: 콜백 prop 호출은 매크로가 `move` 클로저로 감싸므로
        // 한 번의 렌더에서 **딱 한 번**만 부를 수 있다. 그래서 버튼마다 prop을
        // 하나씩 둔다(같은 prop을 두 버튼에 쓰면 "use of moved value").
        on_pen: fn(),
        on_highlighter: fn(),
        on_eraser: fn(),
        on_thinner: fn(),
        on_thicker: fn(),
        on_undo: fn(),
        on_redo: fn(),
        on_clear: fn(),
        on_open: fn(),
        on_export_png: fn(),
        on_export_pdf: fn(),
        on_page_add: fn(),
        on_page_remove: fn(),
        on_page_prev: fn(),
        on_page_next: fn(),
        on_zoom_in: fn(),
        on_zoom_out: fn(),
        // 실패 화면의 복구 동선(예제 19: "다시 시도").
        on_retry: fn(),
        // 단축키 안내 패널 — elm이 소유하는 화면 상태(호스트는 모른다).
        help = false,
    ) {
        // 단축키는 elm이 선언하고 호스트가 밀어 준다(예제 12).
        // **한계**: 어댑터 0.8.6은 호스트가 `ElmView` 인스턴스를 잡을 수 없어
        // `drive::dispatch_key`를 직접 부를 수 없다 → 같은 패널을 **툴바 버튼**으로도
        // 열 수 있게 했다(`help = !help`). 호스트는 Reactor가 지원하는 가속기
        // (Ctrl+더하기/빼기/Enter)만 붙인다 — README의 "알아둘 한계" 참고.
        on_key("F1") { help = !help }
        on_key("Escape") { help = false }

        <Col>
            // ── 툴바 ────────────────────────────────────────────
            <Row>
                <Button on_click={on_pen()}>"펜"</Button>
                <Button on_click={on_highlighter()}>"형광펜"</Button>
                <Button on_click={on_eraser()}>"지우개"</Button>
                <Divider />
                <Button on_click={on_thinner()}>"가늘게"</Button>
                "굵기 {width_pt:.1}pt"
                <Button on_click={on_thicker()}>"굵게"</Button>
                <Divider />
                <Button on_click={on_undo()}>"되돌리기"</Button>
                <Button on_click={on_redo()}>"다시하기"</Button>
                <Button on_click={on_clear()}>"페이지 비우기"</Button>
                <Divider />
                <Button on_click={on_open()}>"PDF 열기"</Button>
                <Button on_click={on_export_png()}>"PNG 저장"</Button>
                <Button on_click={on_export_pdf()}>"PDF 저장"</Button>
                <Divider />
                // 단축키 안내는 **elm이 소유한 화면 상태**라 호스트를 거치지 않는다.
                <Button on_click={help = !help}>"단축키"</Button>
            </Row>

            <Row>
                // ── 페이지 사이드바 ──────────────────────────────
                <Col>
                    <Strong>"{doc_title}"</Strong>
                    <If when={has_pdf}>
                        <Text>"배경: {pdf_name}"</Text>
                    </If>
                    "페이지 {page + 1} / {page_count}"
                    <For each={page_labels} as={label} key={label}>
                        <Text>"{label}"</Text>
                    </For>
                    <Button on_click={on_page_prev()}>"이전 페이지"</Button>
                    <Button on_click={on_page_next()}>"다음 페이지"</Button>
                    <Button on_click={on_page_add()}>"페이지 추가"</Button>
                    <Button on_click={on_page_remove()}>"페이지 삭제"</Button>
                </Col>

                // ── 잉크 표면 (WinUI: <Raw> 하나) ────────────────
                <Col>
                    <Row>
                        <Button on_click={on_zoom_out()}>"축소"</Button>
                        "확대 {zoom:.0}%"
                        <Button on_click={on_zoom_in()}>"확대"</Button>
                        "선분 {surface_lines}개"
                    </Row>
                    // 상태는 enum 하나로 두고 <Switch>로 빠짐없이 분기한다(예제 19).
                    // "빈 페이지"와 "여는 중"과 "실패"는 다른 화면이다.
                    <Switch on={phase}>
                        <Case when={Phase::Loading}>
                            <Row>
                                <Spinner />
                                <Text>"PDF 여는 중…"</Text>
                            </Row>
                        </Case>
                        <Case when={Phase::Empty}>
                            <Col>
                                <Text>"여기에 필기하세요 — 펜으로 그리면 됩니다"</Text>
                                <InkSurfaceView />
                            </Col>
                        </Case>
                        <Case when={Phase::Ready}>
                            <InkSurfaceView />
                        </Case>
                        <Case when={Phase::Failed(message)}>
                            <Col>
                                <Banner kind="error">{message}</Banner>
                                <Button on_click={on_retry()}>"다시 시도"</Button>
                            </Col>
                        </Case>
                    </Switch>
                </Col>
            </Row>

            // ── 상태바 ──────────────────────────────────────────
            <Divider />
            <Row>
                "도구: {tool.label()}"
                "스트로크 {stroke_count}개"
                <If when={can_undo}><Text>"되돌릴 수 있음"</Text></If>
                <If when={can_redo}><Text>"다시할 수 있음"</Text></If>
                <If when={dirty}><Text>"저장 안 됨"</Text></If>
            </Row>
            <Text>"{status}"</Text>
            <Text>"{tool.hint()}"</Text>

            // ── 단축키 안내 (elm이 소유한 화면 상태) ─────────────
            <If when={help}>
                <Col>
                    <Strong>"단축키"</Strong>
                    <Text>"F1 안내 열기/닫기 · Esc 닫기"</Text>
                    <Text>"Ctrl+더하기 / Ctrl+빼기 확대·축소 (호스트 가속기)"</Text>
                    <Text>"Ctrl+Enter 페이지 추가 (호스트 가속기)"</Text>
                    <Text>"나머지 기능은 툴바 버튼으로 — 호스트가 키를 라우팅하면 더 붙는다"</Text>
                </Col>
            </If>
        </Col>
    }
}

/// 페이지 목록 라벨 — 사이드바와 상태바가 같은 문자열을 쓴다.
pub fn page_label(index: usize, page_count: usize, strokes: usize) -> String {
    format!("{} / {}페이지 · 스트로크 {}개", index + 1, page_count, strokes)
}

/// 화면에 내려보낼 값 — 호스트가 채워 props로 만든다.
#[derive(Clone, Debug, PartialEq)]
pub struct NoteViewModel {
    pub tool: Tool,
    pub width_pt: f32,
    pub zoom: f32,
    pub page: usize,
    pub page_count: usize,
    pub stroke_count: usize,
    pub can_undo: bool,
    pub can_redo: bool,
    pub dirty: bool,
    pub doc_title: String,
    pub pdf_name: String,
    pub has_pdf: bool,
    /// 화면 상태 (예제 19) — bool 두 개 대신 enum 하나.
    pub phase: Phase,
    pub status: String,
    pub page_labels: Vec<String>,
    pub surface_width: u16,
    pub surface_height: u16,
    pub surface_lines: usize,
}

impl Default for NoteViewModel {
    fn default() -> Self {
        Self {
            tool: Tool::Pen,
            width_pt: StrokeStyle::PEN_WIDTH_PT,
            zoom: 100.0,
            page: 0,
            page_count: 1,
            stroke_count: 0,
            can_undo: false,
            can_redo: false,
            dirty: false,
            doc_title: "무제".to_string(),
            pdf_name: String::new(),
            has_pdf: false,
            phase: Phase::Empty,
            status: String::new(),
            page_labels: Vec::new(),
            surface_width: 0,
            surface_height: 0,
            surface_lines: 0,
        }
    }
}

impl NoteViewModel {
    /// 문서에서 화면 값을 만든다(표면 크기/선분 수는 [`Self::with_surface`]로).
    pub fn from_document(document: &Document, tool: Tool, width_pt: f32, zoom: f32) -> Self {
        Self {
            tool,
            width_pt,
            zoom,
            page: document.active_index(),
            page_count: document.page_count(),
            stroke_count: document.active_page().stroke_count(),
            can_undo: document.can_undo(),
            can_redo: document.can_redo(),
            dirty: document.is_dirty(),
            doc_title: document.title().to_string(),
            status: document.status_line(),
            phase: Self::phase_for(document),
            page_labels: document
                .pages()
                .iter()
                .enumerate()
                .map(|(index, page)| page_label(index, document.page_count(), page.stroke_count()))
                .collect(),
            ..Self::default()
        }
    }

    /// PDF 배경 정보를 붙인다.
    pub fn with_pdf(mut self, name: Option<&str>) -> Self {
        match name {
            Some(name) => {
                self.pdf_name = name.to_string();
                self.has_pdf = true;
            }
            None => {
                self.pdf_name.clear();
                self.has_pdf = false;
            }
        }
        self
    }

    /// 표면 표시 값을 붙인다(재료 자체는 [`stage_surface`]로 간다).
    pub fn with_surface(mut self, surface: &InkSurface) -> Self {
        self.surface_width = surface.width;
        self.surface_height = surface.height;
        self.surface_lines = surface.line_count();
        self
    }

    /// 로딩/실패 표시를 바꾼다.
    pub fn with_phase(mut self, phase: Phase) -> Self {
        self.phase = phase;
        self
    }

    /// 문서에서 "빈 페이지 / 준비됨"을 스스로 판단한다 — 화면이 상태를 계산하지 않는다.
    pub fn phase_for(document: &Document) -> Phase {
        let page = document.active_page();
        if page.stroke_count() > 0 || page.has_pdf_background() {
            Phase::Ready
        } else {
            Phase::Empty
        }
    }
}

/// 의도(intent) 묶음 — 화면의 콜백 prop에 그대로 들어간다.
///
/// 호스트는 [`NoteIntents::each`]로 자기 메시지 큐에 연결하고, 테스트는 이름을
/// 기록해 "어느 버튼이 무슨 의도를 냈는지" 검증한다.
#[derive(Clone, Default)]
pub struct NoteIntents {
    pub pen: Callback<()>,
    pub highlighter: Callback<()>,
    pub eraser: Callback<()>,
    pub thinner: Callback<()>,
    pub thicker: Callback<()>,
    pub undo: Callback<()>,
    pub redo: Callback<()>,
    pub clear: Callback<()>,
    pub open: Callback<()>,
    pub export_png: Callback<()>,
    pub export_pdf: Callback<()>,
    pub page_add: Callback<()>,
    pub page_remove: Callback<()>,
    pub page_prev: Callback<()>,
    pub page_next: Callback<()>,
    pub zoom_in: Callback<()>,
    pub zoom_out: Callback<()>,
    pub retry: Callback<()>,
}

impl NoteIntents {
    /// 모든 의도를 한 함수로 받는다 — 인자는 의도 이름이다.
    pub fn each<F>(f: F) -> Self
    where
        F: Fn(&'static str) + 'static,
    {
        let f = std::rc::Rc::new(f);
        let mut intents = Self::default();
        macro_rules! wire {
            ($($field:ident => $name:literal),* $(,)?) => {
                $(
                    intents.$field = {
                        let f = std::rc::Rc::clone(&f);
                        Callback::new(move |_arena: &mut elm_magic::Arena, _: ()| f($name))
                    };
                )*
            };
        }
        wire! {
            pen => "pen",
            highlighter => "highlighter",
            eraser => "eraser",
            thinner => "thinner",
            thicker => "thicker",
            undo => "undo",
            redo => "redo",
            clear => "clear",
            open => "open",
            export_png => "export_png",
            export_pdf => "export_pdf",
            page_add => "page_add",
            page_remove => "page_remove",
            page_prev => "page_prev",
            page_next => "page_next",
            zoom_in => "zoom_in",
            zoom_out => "zoom_out",
            retry => "retry",
        }
        intents
    }

    /// 이름을 순서대로 기록한다(테스트용).
    pub fn recording(sink: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>) -> Self {
        Self::each(move |name| sink.borrow_mut().push(name))
    }
}

impl NoteAppProps {
    /// 화면 props를 만든다 — **필드가 늘어도 이 함수만 고치면 된다**.
    pub fn from_view_model(view: &NoteViewModel, intents: &NoteIntents) -> Self {
        Self {
            tool: Some(view.tool),
            width_pt: Some(view.width_pt),
            zoom: Some(view.zoom),
            page: Some(view.page),
            page_count: Some(view.page_count),
            stroke_count: Some(view.stroke_count),
            can_undo: Some(view.can_undo),
            can_redo: Some(view.can_redo),
            dirty: Some(view.dirty),
            doc_title: Some(view.doc_title.clone()),
            pdf_name: Some(view.pdf_name.clone()),
            has_pdf: Some(view.has_pdf),
            phase: Some(view.phase.clone()),
            status: Some(view.status.clone()),
            page_labels: Some(view.page_labels.clone()),
            surface_width: Some(view.surface_width),
            surface_height: Some(view.surface_height),
            surface_lines: Some(view.surface_lines),
            on_pen: Some(intents.pen.clone()),
            on_highlighter: Some(intents.highlighter.clone()),
            on_eraser: Some(intents.eraser.clone()),
            on_thinner: Some(intents.thinner.clone()),
            on_thicker: Some(intents.thicker.clone()),
            on_undo: Some(intents.undo.clone()),
            on_redo: Some(intents.redo.clone()),
            on_clear: Some(intents.clear.clone()),
            on_open: Some(intents.open.clone()),
            on_export_png: Some(intents.export_png.clone()),
            on_export_pdf: Some(intents.export_pdf.clone()),
            on_page_add: Some(intents.page_add.clone()),
            on_page_remove: Some(intents.page_remove.clone()),
            on_page_prev: Some(intents.page_prev.clone()),
            on_page_next: Some(intents.page_next.clone()),
            on_zoom_in: Some(intents.zoom_in.clone()),
            on_zoom_out: Some(intents.zoom_out.clone()),
            on_retry: Some(intents.retry.clone()),
            ..Default::default()
        }
    }
}

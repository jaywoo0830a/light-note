//! 셸(호스트) — **4단계를 순서대로 부르는 유일한 곳**.
//!
//! ```text
//! Pointer(phase,x,y,frame) → ① input::sample_with → ② tool.press/drag/lift → ③ refresh → ④ 발행
//! Intent(intent)      → ② 도구/문서 명령   → ③ canvas.edit / go_to_page / step_zoom → ④ 발행
//! Baked(result)       → (④-워커 결과)      → ③ canvas.accept(디코드 신호 대기)
//! Decoded(index)      → (ImageOpened)     → ③ canvas.promote → ④ 발행
//! ```
//!
//! ## 필기 정책 — **디지타이저(펜)만** 잉크가 된다
//! `Pointer`의 프레임은 ①-하드웨어([`digitizer`])가 Win32 `WM_POINTER`에서 읽은 것이다.
//! 시작(`Pressed`)과 진행(`Moved`)은 **펜일 때만** ②로 넘어가고, 손가락·마우스는 무시한다.
//! **무시하지 취소하지 않는다** — 손바닥이 닿았다고 진행 중인 펜 획을 버리면 필기가 안 된다.
//! 끝(`Released`/`Canceled`)은 장치와 무관하게 전달한다(커밋할 제스처는 펜이 시작한 것뿐이다).
//!
//! ## UI 스레드에서 하는 일 (R1 — 페이지 크기에 비례하는 일은 없다)
//! - 포인터 표본 하나: ② O(1) + ③ 진행 중 획 하나 갱신 + ④ 도형 만들기(예산 안).
//! - **래스터+인코딩은 `spawn_background`에서만** 돈다([`render::bake`]).
//! - 디코드 신호를 **기다리지 않는다**: 기다리는 동안에도 화면은 옛 베이스 + 라이브 꼬리로
//!   완전하다(R3) → 타이머도, 잠만 자는 스레드도, 세대 장부도 없다.
//!
//! ## 왜 `#[store]`를 안 쓰나
//! 문서의 진실은 **호스트**에 있다(포인터가 곧 편집이고, 그 편집이 ③에 있어야 한다).
//! elm은 화면이고, 호스트는 파이프라인을 소유한다 — 예제 20의 "셸 하나 + elm 루트 하나".
//!
//! ## 스레드 경계
//! [`HostMessage`]는 `Send`여야 한다(`spawn_background` 요구). 그래서 **PDF 문서를 메시지에
//! 넣지 않는다** — 워커가 필요하면 바이트에서 다시 파싱한다.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use elm_magic::Callback;
use elm_magic_windows_reactor::{ElmInput, ElmView};
use windows_reactor::{
    Border, ChildrenControl, Component, ComponentContext, ContentControl, DragDropAction,
    DragDropOperation, DragDropPolicy, DragKind, DroppedData, Grid, View, ViewContext,
    WindowBackdrop, WindowConstraints, WindowSize, WindowTheme, WindowVisuals,
};

use crate::canvas::{BakeResult, Canvas};
use crate::digitizer;
use crate::export::{self, EXPORT_SCALE};
use crate::files::{self, OpenedFile};
use crate::geom::Size;
use crate::ink::Tool;
use crate::input::{self, Phase};
use crate::parts;
use crate::pdf::PdfDocument;
use crate::render;
use crate::style::TOKENS;
use crate::tool::CanvasTool;
use crate::ui::{self, Frame, Intent, Screen, ScreenProps, Stage, ViewModel};

/// 호스트 메시지 — `spawn_background`가 돌려주므로 **`Send`**여야 한다.
#[derive(Clone, Debug)]
pub enum HostMessage {
    /// ① 입력 — 표면 DIP 좌표 + 위상 + **하드웨어 프레임**(펜이면 장치·필압·틸트).
    Pointer(Phase, f64, f64, Option<input::PointerFrame>),
    /// elm 화면의 의도.
    Intent(Intent),
    /// ④-워커가 구운 베이스.
    Baked(BakeResult),
    /// 화면 밖 그림의 디코드 완료(`ImageOpened`) — 앞자리로 올려도 좋다.
    Decoded(usize),
    /// 창 크기가 바뀌었다 (DIP) — 잉크 영역의 높이를 다시 잡는다.
    Resized(f64, f64),
    /// 끌어온 것이 **창 위에 들어왔다**(공식 `drag-drop` 패턴) — 종류를 그대로 들고 온다.
    DragEnter(DragKind),
    /// 끌어온 것이 나갔거나 놓였다 — 안내를 원래대로 돌린다.
    DragLeave,
    /// 놓였다 — PDF면 연다(`DroppedData::StorageItems`의 경로를 본다).
    Dropped(DroppedData),
    /// 백그라운드가 읽은 PDF 파일.
    Opened(Result<OpenedFile, String>),
    /// 백그라운드 내보내기 결과(저장된 경로).
    Exported(Result<PathBuf, String>),
}

/// 문서·도구·표면을 소유하는 셸.
pub struct Shell {
    /// ③ 캔버스(문서 + 구운 접두사 + 꼬리).
    canvas: Canvas,
    /// ② 도구.
    tool: CanvasTool,
    /// 배경 PDF(페이지마다 배경을 깐다).
    pdf: Option<PdfDocument>,
    /// 원본 PDF 바이트 — 워커가 내보낼 때 다시 파싱한다(`Send`).
    pdf_bytes: Option<Vec<u8>>,
    stage: Stage,
    status: String,
    /// 단축키 패널이 열려 있는가 — **호스트가 소유**한다(조각이 그리기만 한다).
    help: bool,
    /// **개발자 도구**(디지타이저 진단)가 열려 있는가 — 이것도 호스트가 소유한다.
    dev: bool,
    /// WinUI 표면이 받은 포인터 이벤트 수 — **훅과 UI 경로를 가르는 숫자**다.
    ///
    /// 디지타이저 훅이 죽어 있어도 표면에는 이벤트가 온다. 이 수가 0이면 문제는 훅이 아니라
    /// UI 경로다(진단 표의 첫 줄이 이 값을 말한다).
    ui_events: Rc<Cell<u64>>,
    /// 창의 클라이언트 크기 (DIP) — 잉크 영역 높이의 근거(관측이 오기 전에는 기본값).
    viewport: (f64, f64),
    /// 끌어놓기 안내를 띄우기 **전의** 상태 문구 — 나가면 이 값으로 되돌린다.
    status_before_drop: Option<String>,
    /// ④-워커에 가 있는 요청 id — **R2: 한 번에 하나**([`Canvas::needs_bake`]).
    baking: Option<u64>,
}

impl Component for Shell {
    type Input = ();
    type Message = HostMessage;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
        let mut shell = Self {
            canvas: Canvas::new(Size::A4),
            tool: CanvasTool::new(),
            pdf: None,
            pdf_bytes: None,
            stage: Stage::Empty,
            status: "New note — draw with a pen (digitizer)".to_string(),
            help: false,
            dev: false,
            ui_events: Rc::new(Cell::new(0)),
            viewport: (1280.0, 800.0),
            status_before_drop: None,
            baking: None,
        };
        shell.refresh(context);
        shell
    }

    fn update(&mut self, message: HostMessage, context: &ComponentContext<Self>) {
        // 창이 첫 발행보다 늦게 생길 수 있다 — 메시지마다 한 번 더 시도한다(이미 걸렸으면 공짜다).
        digitizer::install();
        match message {
            HostMessage::Pointer(phase, x, y, frame) => self.pointer(phase, x, y, frame),
            HostMessage::Intent(intent) => self.intent(intent, context),
            HostMessage::Baked(result) => self.adopt_bake(result),
            HostMessage::Decoded(_) => {
                // 화면 밖 그림이 준비됐다 — 좌표만 맞바꾼다(디코드 없음).
                self.canvas.promote();
            }
            HostMessage::Resized(width, height) => {
                // 창 크기가 바뀌었다 — 잉크 영역의 높이만 다시 잡는다(다음 발행에서).
                self.viewport = (width, height);
            }
            HostMessage::DragEnter(kind) => self.drop_hover(kind),
            HostMessage::DragLeave => self.drop_leave(),
            HostMessage::Dropped(data) => self.dropped(data, context),
            HostMessage::Opened(Ok(file)) => self.adopt_pdf(file),
            HostMessage::Opened(Err(error)) => {
                self.status = error.clone();
                self.stage = Stage::Failed(error);
            }
            HostMessage::Exported(Ok(path)) => self.status = format!("Saved: {}", path.display()),
            HostMessage::Exported(Err(error)) => {
                self.status = error.clone();
                self.stage = Stage::Failed(error);
            }
        }
        // 어떤 메시지든 ③을 최신으로 맞추고, 필요하면 ④-워커에 한 건 맡긴다.
        self.refresh(context);
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        // ①-하드웨어: Win32 `WM_POINTER` 훅을 건다(창이 아직 없으면 다음 프레임에 다시 시도한다).
        digitizer::install();

        // 창 제목은 호스트가 정한다(elm은 플랫폼을 모른다 — 예제 20).
        context.window_title(format!("light-note — {}", self.canvas.doc().title()));

        // **창 자체의 디자인**: 테마는 시스템, 배경은 Mica, 크기 하한은 크롬이 무너지지 않는
        // 값, 처음 크기는 한 줄 툴바가 서는 값. (리액터는 **값이 바뀔 때만** 적용하므로
        // 사용자가 창을 줄이거나 늘리는 것과 싸우지 않는다 — `native/winui`의 diff 확인.)
        //
        // 타이틀바는 여기서 만들지 않는다: 조각(`parts::titlebar`)이 WinUI `TitleBar`를 만들면
        // 리액터가 `SetExtendsContentIntoTitleBar(true)` + `SetTitleBar(...)` +
        // `PreferredHeightOption`까지 붙인다 — 크롬이 **네이티브 것**이 된다.
        context.window_visuals(
            WindowVisuals::new()
                .theme(WindowTheme::System)
                .backdrop(WindowBackdrop::Mica)
                .client_size(TOKENS.default_window_w, TOKENS.default_window_h)
                .constraints(WindowConstraints {
                    min_width: Some(TOKENS.min_window_w),
                    min_height: Some(TOKENS.min_window_h),
                    max_width: None,
                    max_height: None,
                }),
        );

        // **창 크기 관측** — 잉크 영역이 창 안에 들어오게 하는 근거다(elm 트리는 StackPanel뿐이라
        // 자식 높이가 묶이지 않는다). 관측은 큐로 오므로 다음 발행에서 반영된다.
        let sender = context.sender();
        let resize_sender = sender.clone();
        context.on_window_size(move |size: WindowSize| {
            let _ = resize_sender.send(HostMessage::Resized(size.width, size.height));
        });

        // ① 포인터 → 메시지 큐. 표면 빌더가 이 싱크를 **캡처**한다(예제 08).
        let pointer_sender = sender.clone();
        let ui_events = Rc::clone(&self.ui_events);
        let sink: input::InputSink = Rc::new(move |phase, x, y| {
            // 진단: WinUI 경로가 살아 있는가 — 훅이 죽어도 이 수는 는다.
            ui_events.set(ui_events.get().wrapping_add(1));
            // 이벤트 **순간**의 펜 프레임을 붙인다 — 큐를 거친 뒤에 읽으면 이미 낡았다.
            let frame = digitizer::latest();
            let _ = pointer_sender.send(HostMessage::Pointer(phase, x, y, frame));
        });

        // ④ 디코드 완료 → 앞자리 교체. **이 신호 뒤에만** 화면이 바뀐다(빈 프레임 없음).
        let decoded_sender = sender.clone();
        let decoded: render::ImageSink = Rc::new(move |index| {
            let _ = decoded_sender.send(HostMessage::Decoded(index));
        });

        // 가속기(Reactor가 지원하는 키만) → 같은 큐. **루트**에 붙인다(아래): 툴바 버튼을
        // 누르면 포커스가 표면 밖으로 가므로, 표면에만 붙이면 그때 Ctrl+±가 죽는다.
        let accelerator_sender = sender.clone();
        let accelerators = render::accelerators(move |intent| {
            let _ = accelerator_sender.send(HostMessage::Intent(intent));
        });
        ui::set_surface_builder(Rc::new(move |frame: &Frame, view: &ViewModel| {
            render::surface(frame, &sink, &decoded, view)
        }));

        // 조각 빌더 — `<Raw>` 슬롯(타이틀바/툴바/레일/상태 띠/안내/단축키)이 같은 함수를 부른다.
        // 조각의 생김새는 `parts`에만 있으므로, 스타일을 바꿔도 이 파일은 그대로다.
        ui::set_part_builder(Rc::new(parts::build));

        // **의도 통로** — 조각이 만든 버튼이 여기로 의도를 보낸다. elm 콜백과 같은 큐다.
        let part_sender = sender.clone();
        ui::set_intent_sink(Rc::new(move |intent: Intent| {
            let _ = part_sender.send(HostMessage::Intent(intent));
        }));

        // 발행 직전에 ④-UI 재료를 넣는다 — `<Raw>`가 꺼내 간다(슬롯을 못 보므로).
        let view = self.view_model();
        ui::stage_frame(self.canvas.frame());
        ui::stage_view(view.clone());

        // elm → 호스트: 의도 **하나**로 온다(예제 11의 콜백 prop).
        let intent_sender = sender.clone();
        let on_intent = Callback::new(move |_arena, intent: Intent| {
            let _ = intent_sender.send(HostMessage::Intent(intent));
        });

        let screen = View::component::<ElmView<Screen>>(ElmInput::new(ScreenProps {
            view: Some(view),
            on_intent: Some(on_intent),
            ..Default::default()
        }));

        // ── 루트: **끌어놓기**(공식 `drag-drop` 패턴) + 가속기 ────────────────
        // `Border`가 `drop_policy`/`on_drop`을 가진다(리액터의 `Border` 속성 목록) — 그래서
        // **창 전체**가 놓기 대상이 된다(툴바·레일 위에 떨어뜨려도 열린다). 가속기는 안쪽
        // `Grid`에 그대로 남는다: `Border`에는 `key_accelerators`가 없다.
        let enter_sender = sender.clone();
        let over_sender = sender.clone();
        let leave_sender = sender.clone();
        let drop_sender = sender.clone();
        Border::new()
            .drop_policy(DragDropPolicy::new().storage_items(
                DragDropAction::new(DragDropOperation::Copy).caption("Drop to open the PDF"),
            ))
            .on_drag_enter(move |kind: DragKind| {
                let _ = enter_sender.send(HostMessage::DragEnter(kind));
            })
            // 들어오는 순간과 움직이는 순간이 따로 온다 — 둘 다 같은 안내로 모은다.
            .on_drag_over(move |kind: DragKind| {
                let _ = over_sender.send(HostMessage::DragEnter(kind));
            })
            .on_drag_leave(move || {
                let _ = leave_sender.send(HostMessage::DragLeave);
            })
            .on_drop(move |data: DroppedData| {
                let _ = drop_sender.send(HostMessage::Dropped(data));
            })
            .content(
                Grid::new()
                    .key_accelerators(accelerators)
                    .children((screen,)),
            )
    }
}

impl Shell {
    // ── ① → ② (포인터) ──────────────────────────────────────────────

    /// 포인터 한 점 — ①이 pt로 바꾸고 ②가 잉크로 만든다.
    fn pointer(&mut self, phase: Phase, x: f64, y: f64, frame: Option<input::PointerFrame>) {
        let sample = input::sample_with(phase, x, y, self.canvas.scale(), frame);

        // **필기 정책**: 시작과 진행은 디지타이저(펜)만 한다. 손가락·마우스는 무시한다 —
        // 취소하지 않는 이유는 손바닥이 닿았다고 진행 중인 펜 획을 버리면 안 되기 때문이다.
        // 끝(`is_end`)은 장치와 무관하게 전달한다: 커밋할 제스처는 펜이 시작한 것뿐이다.
        if !sample.phase.is_end() && !CanvasTool::accepts(sample.device()) {
            self.reject(sample);
            return;
        }

        match sample.phase {
            Phase::Pressed => {
                self.tool.press_with(
                    self.canvas.doc_mut(),
                    sample.at,
                    sample.now,
                    sample.pressure(),
                );
                self.status = Self::pen_status(sample);
            }
            Phase::Moved => {
                if self.tool.is_active() {
                    self.tool.drag_with(
                        self.canvas.doc_mut(),
                        sample.at,
                        sample.now,
                        sample.pressure(),
                    );
                }
            }
            Phase::Released => match self.tool.lift(self.canvas.doc_mut()) {
                // 획 하나가 늘었다 — **접두사는 그대로 쓸 수 있다**(그 획은 꼬리가 그린다).
                Some(crate::doc::Edit::AddStroke { .. }) => {
                    self.status = "Stroke added".to_string();
                }
                // 지운 결과는 접두사에 반영돼야 한다 — 다시 굽는다.
                Some(edit) => {
                    self.canvas.invalidate();
                    self.status =
                        format!("{} — {} stroke(s)", edit.label(), edit.affected_strokes());
                }
                None => {}
            },
            Phase::Canceled => {
                // 진행 중 획은 어디에도 남지 않고, 지운 것은 되돌아온다 → 접두사는 그대로 옳다.
                if self.tool.cancel(self.canvas.doc_mut()) {
                    self.status = "Stroke canceled".to_string();
                }
            }
        }
    }

    /// 필기가 아닌 입력 — **아무것도 만들지 않고** 이유를 상태바에 남긴다.
    ///
    /// 진행(`Moved`)마다 쓰면 상태바가 시끄러우므로 **시작(`Pressed`)에서만** 말한다.
    fn reject(&mut self, sample: input::Sample) {
        if sample.phase != Phase::Pressed {
            return;
        }
        let state = digitizer::state();
        self.status = if !state.is_hooking() {
            // 훅이 안 걸렸으면 **그것이** 필기가 안 되는 이유다 — 정직하게 말한다.
            state.reason().to_string()
        } else if digitizer::seen_pen() {
            format!(
                "{} input — this app only inks with a pen (digitizer)",
                sample.device().label()
            )
        } else {
            format!(
                "{} input — this app only inks with a pen (digitizer), and no pen has been detected yet",
                sample.device().label()
            )
        };
    }

    /// 펜으로 시작한 획의 한 줄 — **필압이 오는지**를 화면에서 확인할 수 있게.
    fn pen_status(sample: input::Sample) -> String {
        match (sample.pressure(), sample.tilt()) {
            (Some(_), Some(_)) => "Pen — pressure and tilt".to_string(),
            (Some(_), None) => "Pen — pressure (no tilt)".to_string(),
            _ => "Pen — no pressure reported (width from speed)".to_string(),
        }
    }

    /// 입력 배지 — "무엇으로 그리는 중인가"를 머리글에 **항상** 띄운다.
    ///
    /// 필기가 안 될 때 사용자가 원인을 알 수 있는 유일한 단서라서, 상태 문구(마지막으로
    /// 한 일)와 **따로** 둔다: 훅이 안 걸렸는지 / 펜이 아직 안 왔는지 / 펜이 오는지.
    fn input_badge(&self) -> String {
        let state = digitizer::state();
        if !state.is_hooking() {
            // 훅 자체가 안 걸렸다 — 이유를 그대로 보여준다.
            return state.reason().to_string();
        }
        if digitizer::seen_pen() {
            "Pen — digitizer active".to_string()
        } else {
            "Pen only — no pen detected yet".to_string()
        }
    }

    // ── ② 명령 (elm 의도) ───────────────────────────────────────────

    fn intent(&mut self, intent: Intent, context: &ComponentContext<Self>) {
        // 문서를 고치는 명령 앞에서는 진행 중 제스처를 먼저 정리한다.
        self.tool.cancel(self.canvas.doc_mut());
        match intent {
            Intent::Pen => self.select_tool(Tool::Pen),
            Intent::Highlighter => self.select_tool(Tool::Highlighter),
            Intent::Eraser => self.select_tool(Tool::Eraser),
            Intent::Thinner => {
                self.tool.resize(false);
                self.status = self.width_status();
            }
            Intent::Thicker => {
                self.tool.resize(true);
                self.status = self.width_status();
            }
            Intent::Undo => {
                let edit = self.canvas.edit(|doc| doc.undo());
                self.status = match edit {
                    Some(edit) => format!("Undo — {}", edit.label()),
                    None => "Nothing to undo".to_string(),
                };
            }
            Intent::Redo => {
                let edit = self.canvas.edit(|doc| doc.redo());
                self.status = match edit {
                    Some(edit) => format!("Redo — {}", edit.label()),
                    None => "Nothing to redo".to_string(),
                };
            }
            Intent::Clear => {
                let cleared = self.canvas.edit(|doc| doc.clear_active_page());
                self.status = if cleared {
                    "Page cleared (undoable)".to_string()
                } else {
                    "Nothing to clear".to_string()
                };
            }
            Intent::Open => self.open_pdf(context),
            Intent::ExportPng => self.export_png(context),
            Intent::ExportPdf => self.export_pdf(context),
            Intent::PageAdd => {
                let size = self.canvas.page_size();
                self.canvas.edit(|doc| {
                    doc.add_page_after_active(size);
                });
                self.status = self.canvas.doc().status_line();
            }
            Intent::PageRemove => {
                let index = self.canvas.doc().active_index();
                let removed = self.canvas.edit(|doc| doc.remove_page(index));
                self.status = if removed {
                    self.canvas.doc().status_line()
                } else {
                    "The last page cannot be removed".to_string()
                };
            }
            Intent::PagePrev => self.step_page(-1),
            Intent::PageNext => self.step_page(1),
            Intent::ZoomIn => self.step_zoom(true),
            Intent::ZoomOut => self.step_zoom(false),
            Intent::ToggleHelp => self.help = !self.help,
            Intent::CloseHelp => self.help = false,
            // **개발자 도구** — 여는 것만 한다(표는 다음 발행에서 채워진다: `view_model`).
            Intent::ToggleDev => self.dev = !self.dev,
            Intent::Rescan => {
                // 창을 **다시 찾는다**: 늦게 생긴 자식 창(콘텐츠 창)을 줍는 길이다.
                digitizer::rescan();
                self.status = "Rescanned windows for pen input".to_string();
            }
            Intent::GoToPage(index) => {
                // 레일에서 고른 페이지 — 페이지 이동은 접두사를 무효로 만든다(③이 안다).
                if self.canvas.go_to_page(index) {
                    self.status = self.canvas.doc().status_line();
                }
            }
            Intent::Retry => {
                self.stage = Stage::Empty;
                self.status = "Starting over".to_string();
            }
        }
    }

    fn select_tool(&mut self, tool: Tool) {
        self.tool.select(tool);
        self.status = self.tool.hint().to_string();
    }

    fn width_status(&self) -> String {
        format!("Width {:.1} pt", self.tool.state().width_pt)
    }

    fn step_page(&mut self, delta: isize) {
        // 페이지 이동은 접두사를 무효로 만든다(다른 페이지의 그림이다).
        if self.canvas.step_page(delta) {
            self.status = self.canvas.doc().status_line();
        }
    }

    fn step_zoom(&mut self, closer: bool) {
        if self.canvas.step_zoom(closer) {
            self.status = format!("Zoom {:.0}%", self.canvas.zoom());
        }
    }

    // ── ③ → ④ (한 프레임) ───────────────────────────────────────────

    /// ③ 꼬리를 맞추고, 필요하면 ④-워커에 **한 건** 맡긴다. UI 스레드는 여기서 멈추지 않는다.
    fn refresh(&mut self, context: &ComponentContext<Self>) {
        self.canvas.refresh(self.tool.drawing());
        if !self.canvas.needs_bake(self.baking.is_some()) {
            return;
        }
        let request = self.canvas.bake_request();
        if request.is_empty() {
            // 구울 잉크가 없다 — 워커를 부르지 않는다(빈 페이지에 1.13Mpx를 만들지 않는다).
            self.adopt_bake(request.empty_result());
            return;
        }
        self.baking = Some(request.id);
        let _ = context.spawn_background(move |_cancel| HostMessage::Baked(render::bake(&request)));
    }

    /// ④-워커 결과를 ③에 반영한다 — 낡은 응답이면 조용히 버린다.
    ///
    /// `accept`가 `Some(true)`를 돌려주면 새 그림이 **화면 밖 자리**에 들어간 것이고,
    /// 앞자리 교체는 `ImageOpened`([`HostMessage::Decoded`])를 받고서 한다.
    fn adopt_bake(&mut self, result: BakeResult) {
        if self.baking == Some(result.id) {
            self.baking = None;
        }
        let _ = self.canvas.accept(result);
    }

    /// elm 화면에 내려보낼 값 — 화면은 이 하나만 읽는다.
    fn view_model(&self) -> ViewModel {
        let doc = self.canvas.doc();
        ViewModel {
            tool: self.tool.tool(),
            width_pt: self.tool.state().width_pt,
            zoom: self.canvas.zoom(),
            page: doc.active_index(),
            page_count: doc.page_count(),
            stroke_count: doc.strokes().len(),
            live_shapes: self.canvas.live_shapes(),
            baked: self.canvas.baked_count(),
            can_undo: doc.can_undo(),
            can_redo: doc.can_redo(),
            dirty: doc.is_dirty(),
            title: doc.title().to_string(),
            pdf_name: self
                .pdf
                .as_ref()
                .map(PdfDocument::file_name)
                .unwrap_or_default(),
            stage: self.stage.clone(),
            status: self.status.clone(),
            input: self.input_badge(),
            help: self.help,
            dev: self.dev,
            diag: self.diagnostics(),
            viewport: self.viewport,
            page_labels: doc
                .pages()
                .iter()
                .enumerate()
                .map(|(index, page)| ui::page_label(index, page.stroke_count()))
                .collect(),
        }
    }

    /// 개발자 도구에 올릴 **진단 표** — 문장은 여기서 만든다(조각은 표만 그린다).
    ///
    /// 첫 줄은 WinUI 쪽 사실이다: 디지타이저 훅이 죽어 있어도 **표면에는 이벤트가 온다** —
    /// 그 수가 0이면 문제는 훅이 아니라 UI 경로다(둘을 가르는 유일한 근거).
    fn diagnostics(&self) -> Vec<(String, String)> {
        let digest = digitizer::digest();
        let last = digest.last.map_or("none", |frame| frame.device.label());
        let mut rows = vec![(
            "WinUI pointer events".to_string(),
            format!("{} (last device frame: {last})", self.ui_events.get()),
        )];
        rows.extend(digest.report());
        rows
    }
}

/// 파일 입출력 — 파이프라인 밖이지만 셸이 소유한다(대화상자는 UI 스레드, IO는 워커).
impl Shell {
    /// 끌어온 것이 들어왔다 — **상태 띠가 무엇을 하면 되는지** 말한다(종류별로 다르게).
    ///
    /// 문구를 띄우기 전의 상태를 [`Shell::status_before_drop`]에 남긴다: 나가면 되돌려야
    /// 하기 때문이다(놓지 않고 나가는 것이 정상 경로다).
    fn drop_hover(&mut self, kind: DragKind) {
        if self.status_before_drop.is_none() {
            self.status_before_drop = Some(self.status.clone());
        }
        self.status = match kind {
            DragKind::StorageItems => "Drop a PDF to open it".to_string(),
            DragKind::Text => "Drop a PDF file, not text".to_string(),
            DragKind::Unsupported => "Only PDF files can be opened".to_string(),
        };
    }

    /// 끌어온 것이 나갔다 — 안내를 원래 문구로 되돌린다.
    fn drop_leave(&mut self) {
        if let Some(previous) = self.status_before_drop.take() {
            self.status = previous;
        }
    }

    /// 놓였다 — **PDF 하나면 연다**(공식 `drag-drop` 샘플과 같은 흐름: 경로를 꺼내 워커에 맡긴다).
    ///
    /// 고르는 일은 순수 함수([`files::first_pdf`])가 한다: 화면 없이 테스트되는 자리다.
    fn dropped(&mut self, data: DroppedData, context: &ComponentContext<Self>) {
        self.drop_leave();
        let paths = match data {
            DroppedData::StorageItems(items) => {
                items.into_iter().map(|item| item.path).collect::<Vec<_>>()
            }
            // 텍스트·알 수 없는 형식은 열 것이 없다 — **왜 안 되는지**를 말한다.
            _ => {
                self.status = "Only PDF files can be opened".to_string();
                return;
            }
        };
        let Some(path) = files::first_pdf(paths) else {
            self.status = "Only PDF files can be opened".to_string();
            return;
        };
        self.stage = Stage::Loading;
        self.status = format!("Opening {}…", path.display());
        let _ = context.spawn_background(move |_cancel| HostMessage::Opened(files::read_pdf(path)));
    }

    /// PDF 열기 — 대화상자(UI 스레드) → 읽기(워커) → 파싱(지연 파싱이라 싸다).
    fn open_pdf(&mut self, context: &ComponentContext<Self>) {
        let Some(path) = files::pick_pdf() else {
            self.status = "Open canceled".to_string();
            return;
        };
        self.stage = Stage::Loading;
        self.status = format!("Opening {}…", path.display());
        let _ = context.spawn_background(move |_cancel| HostMessage::Opened(files::read_pdf(path)));
    }

    /// PNG 내보내기 — 페이지마다 파일 하나. 렌더는 워커에서.
    fn export_png(&mut self, context: &ComponentContext<Self>) {
        let Some(directory) = files::pick_folder("Choose a folder for the PNG export") else {
            self.status = "Save canceled".to_string();
            return;
        };
        let doc = self.canvas.doc().clone();
        let pdf_bytes = self.pdf_bytes.clone();
        let stem = self.export_stem();
        self.status = "Exporting PNG…".to_string();
        let _ = context.spawn_background(move |_cancel| {
            // 배경 PDF는 **여기서** 다시 파싱한다(문서는 스레드 경계를 넘기지 않는다).
            let source = pdf_bytes.and_then(|bytes| PdfDocument::from_bytes(bytes).ok());
            let result =
                export::export_document_png(&doc, source.as_ref(), &directory, &stem, EXPORT_SCALE)
                    .map(|paths| paths.first().cloned().unwrap_or(directory))
                    .map_err(|error| error.to_string());
            HostMessage::Exported(result)
        });
    }

    /// PDF 내보내기 — 잉크는 벡터, 배경 페이지는 이미지 XObject.
    fn export_pdf(&mut self, context: &ComponentContext<Self>) {
        let suggested = format!("{}.pdf", self.export_stem());
        let Some(path) = files::pick_save("pdf", "PDF document", &suggested) else {
            self.status = "Save canceled".to_string();
            return;
        };
        let doc = self.canvas.doc().clone();
        let pdf_bytes = self.pdf_bytes.clone();
        self.status = "Exporting PDF…".to_string();
        let _ = context.spawn_background(move |_cancel| {
            let source = pdf_bytes.and_then(|bytes| PdfDocument::from_bytes(bytes).ok());
            let result =
                export::export_pdf(&doc, source.as_ref(), &path).map_err(|error| error.to_string());
            HostMessage::Exported(result)
        });
    }

    /// 내보낼 파일 이름의 기본값.
    fn export_stem(&self) -> String {
        let title = self.canvas.doc().title().trim();
        let stem = title.strip_suffix(".pdf").unwrap_or(title);
        if stem.is_empty() {
            "light-note".to_string()
        } else {
            stem.to_string()
        }
    }

    /// 워커가 읽어 온 PDF를 문서로 만든다 — 새 페이지 목록 + 배경 + 접두사 무효화.
    fn adopt_pdf(&mut self, file: OpenedFile) {
        let name = file.file_name();
        let stem = file.stem();
        match PdfDocument::from_bytes(file.bytes.clone()) {
            Ok(pdf) => {
                let sizes = pdf.page_sizes();
                let count = sizes.len();
                let zoom = self.canvas.zoom();
                self.canvas = Canvas::from_pdf(sizes, stem, zoom);
                self.stage = Stage::Ready;
                self.status = format!("{name} — {count} page(s)");
                self.pdf = Some(pdf);
                self.pdf_bytes = Some(file.bytes);
            }
            Err(error) => {
                self.status = error.to_string();
                self.stage = Stage::Failed(error.to_string());
            }
        }
    }
}

/// 앱 시작 — 예제 20의 골격.
///
/// `#[store] struct App`을 만들지 않는다: 이 모듈에 `struct App`이 생기면
/// `windows_reactor::App`과 충돌한다(E0255) — 그래서 경로로 쓴다.
pub fn run() {
    windows_reactor::App::run_component::<Shell>(()).expect("Reactor 실행 실패");
}

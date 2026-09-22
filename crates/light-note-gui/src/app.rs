//! 셸(호스트) — **4단계를 순서대로 부르는 유일한 곳**.
//!
//! ```text
//! Pointer(phase,x,y)  → ① input::sample   → ② tool.press/drag/lift   → ③ canvas.refresh → ④ 발행
//! Intent(intent)      → ② 도구/문서 명령   → ③ canvas.edit / go_to_page / step_zoom → ④ 발행
//! Baked(result)       → (④-워커 결과)      → ③ canvas.accept(디코드 신호 대기)
//! Decoded(index)      → (ImageOpened)     → ③ canvas.promote → ④ 발행
//! ```
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

use std::path::PathBuf;
use std::rc::Rc;

use elm_magic::Callback;
use elm_magic_windows_reactor::{ElmInput, ElmView};
use windows_reactor::{Component, ComponentContext, View, ViewContext};

use crate::canvas::{BakeResult, Canvas};
use crate::export::{self, EXPORT_SCALE};
use crate::files::{self, OpenedFile};
use crate::geom::Size;
use crate::ink::Tool;
use crate::input::{self, Phase};
use crate::pdf::PdfDocument;
use crate::render;
use crate::tool::CanvasTool;
use crate::ui::{self, Frame, Intent, Screen, ScreenProps, Stage, ViewModel};

/// 호스트 메시지 — `spawn_background`가 돌려주므로 **`Send`**여야 한다.
#[derive(Clone, Debug)]
pub enum HostMessage {
    /// ① 입력 — 표면 DIP 좌표 + 위상.
    Pointer(Phase, f64, f64),
    /// elm 화면의 의도.
    Intent(Intent),
    /// ④-워커가 구운 베이스.
    Baked(BakeResult),
    /// 화면 밖 그림의 디코드 완료(`ImageOpened`) — 앞자리로 올려도 좋다.
    Decoded(usize),
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
            status: "새 노트 — 펜으로 그려 보세요".to_string(),
            baking: None,
        };
        shell.refresh(context);
        shell
    }

    fn update(&mut self, message: HostMessage, context: &ComponentContext<Self>) {
        match message {
            HostMessage::Pointer(phase, x, y) => self.pointer(phase, x, y),
            HostMessage::Intent(intent) => self.intent(intent, context),
            HostMessage::Baked(result) => self.adopt_bake(result),
            HostMessage::Decoded(_) => {
                // 화면 밖 그림이 준비됐다 — 좌표만 맞바꾼다(디코드 없음).
                self.canvas.promote();
            }
            HostMessage::Opened(Ok(file)) => self.adopt_pdf(file),
            HostMessage::Opened(Err(error)) => {
                self.status = error.clone();
                self.stage = Stage::Failed(error);
            }
            HostMessage::Exported(Ok(path)) => self.status = format!("저장: {}", path.display()),
            HostMessage::Exported(Err(error)) => {
                self.status = error.clone();
                self.stage = Stage::Failed(error);
            }
        }
        // 어떤 메시지든 ③을 최신으로 맞추고, 필요하면 ④-워커에 한 건 맡긴다.
        self.refresh(context);
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        // 창 제목은 호스트가 정한다(elm은 플랫폼을 모른다 — 예제 20).
        context.window_title(format!("light-note — {}", self.canvas.doc().title()));

        let sender = context.sender();

        // ① 포인터 → 메시지 큐. 표면 빌더가 이 싱크를 **캡처**한다(예제 08).
        let pointer_sender = sender.clone();
        let sink: input::InputSink = Rc::new(move |phase, x, y| {
            let _ = pointer_sender.send(HostMessage::Pointer(phase, x, y));
        });

        // ④ 디코드 완료 → 앞자리 교체. **이 신호 뒤에만** 화면이 바뀐다(빈 프레임 없음).
        let decoded_sender = sender.clone();
        let decoded: render::ImageSink = Rc::new(move |index| {
            let _ = decoded_sender.send(HostMessage::Decoded(index));
        });

        // 가속기(Reactor가 지원하는 키만) → 같은 큐.
        let accelerator_sender = sender.clone();
        let accelerators = render::accelerators(move |intent| {
            let _ = accelerator_sender.send(HostMessage::Intent(intent));
        });
        ui::set_surface_builder(Rc::new(move |frame: &Frame| {
            render::surface(frame, &sink, &decoded, accelerators.clone())
        }));

        // 발행 직전에 ④-UI 재료를 넣는다 — `<Raw>`가 꺼내 간다(슬롯을 못 보므로).
        ui::stage_frame(self.canvas.frame());

        // elm → 호스트: 의도 **하나**로 온다(예제 11의 콜백 prop).
        let intent_sender = sender.clone();
        let on_intent = Callback::new(move |_arena, intent: Intent| {
            let _ = intent_sender.send(HostMessage::Intent(intent));
        });

        View::component::<ElmView<Screen>>(ElmInput::new(ScreenProps {
            view: Some(self.view_model()),
            on_intent: Some(on_intent),
            ..Default::default()
        }))
    }
}

impl Shell {
    // ── ① → ② (포인터) ──────────────────────────────────────────────

    /// 포인터 한 점 — ①이 pt로 바꾸고 ②가 잉크로 만든다.
    fn pointer(&mut self, phase: Phase, x: f64, y: f64) {
        let sample = input::sample(phase, x, y, self.canvas.scale());
        match sample.phase {
            Phase::Pressed => self
                .tool
                .press(self.canvas.doc_mut(), sample.at, sample.now),
            Phase::Moved => {
                if self.tool.is_active() {
                    self.tool.drag(self.canvas.doc_mut(), sample.at, sample.now);
                }
            }
            Phase::Released => match self.tool.lift(self.canvas.doc_mut()) {
                // 획 하나가 늘었다 — **접두사는 그대로 쓸 수 있다**(그 획은 꼬리가 그린다).
                Some(crate::doc::Edit::AddStroke { .. }) => {
                    self.status = "획을 추가했습니다".to_string();
                }
                // 지운 결과는 접두사에 반영돼야 한다 — 다시 굽는다.
                Some(edit) => {
                    self.canvas.invalidate();
                    self.status = format!("{} — {}개", edit.label(), edit.affected_strokes());
                }
                None => {}
            },
            Phase::Canceled => {
                // 진행 중 획은 어디에도 남지 않고, 지운 것은 되돌아온다 → 접두사는 그대로 옳다.
                if self.tool.cancel(self.canvas.doc_mut()) {
                    self.status = "획을 취소했습니다".to_string();
                }
            }
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
                    Some(edit) => format!("{} — 되돌렸습니다", edit.label()),
                    None => "되돌릴 편집이 없습니다".to_string(),
                };
            }
            Intent::Redo => {
                let edit = self.canvas.edit(|doc| doc.redo());
                self.status = match edit {
                    Some(edit) => format!("{} — 다시 적용", edit.label()),
                    None => "다시 적용할 편집이 없습니다".to_string(),
                };
            }
            Intent::Clear => {
                let cleared = self.canvas.edit(|doc| doc.clear_active_page());
                self.status = if cleared {
                    "페이지를 비웠습니다 (되돌리기 가능)".to_string()
                } else {
                    "비울 획이 없습니다".to_string()
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
                    "마지막 페이지는 지울 수 없습니다".to_string()
                };
            }
            Intent::PagePrev => self.step_page(-1),
            Intent::PageNext => self.step_page(1),
            Intent::ZoomIn => self.step_zoom(true),
            Intent::ZoomOut => self.step_zoom(false),
            Intent::Retry => {
                self.stage = Stage::Empty;
                self.status = "다시 시작합니다".to_string();
            }
        }
    }

    fn select_tool(&mut self, tool: Tool) {
        self.tool.select(tool);
        self.status = self.tool.hint().to_string();
    }

    fn width_status(&self) -> String {
        format!("굵기 {:.1}pt", self.tool.state().width_pt)
    }

    fn step_page(&mut self, delta: isize) {
        // 페이지 이동은 접두사를 무효로 만든다(다른 페이지의 그림이다).
        if self.canvas.step_page(delta) {
            self.status = self.canvas.doc().status_line();
        }
    }

    fn step_zoom(&mut self, closer: bool) {
        if self.canvas.step_zoom(closer) {
            self.status = format!("배율 {:.0}%", self.canvas.zoom());
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
            page_labels: doc
                .pages()
                .iter()
                .enumerate()
                .map(|(index, page)| ui::page_label(index, page.stroke_count()))
                .collect(),
        }
    }
}

/// 파일 입출력 — 파이프라인 밖이지만 셸이 소유한다(대화상자는 UI 스레드, IO는 워커).
impl Shell {
    /// PDF 열기 — 대화상자(UI 스레드) → 읽기(워커) → 파싱(지연 파싱이라 싸다).
    fn open_pdf(&mut self, context: &ComponentContext<Self>) {
        let Some(path) = files::pick_pdf() else {
            self.status = "열기를 취소했습니다".to_string();
            return;
        };
        self.stage = Stage::Loading;
        self.status = format!("{} 여는 중…", path.display());
        let _ = context.spawn_background(move |_cancel| HostMessage::Opened(files::read_pdf(path)));
    }

    /// PNG 내보내기 — 페이지마다 파일 하나. 렌더는 워커에서.
    fn export_png(&mut self, context: &ComponentContext<Self>) {
        let Some(directory) = files::pick_folder("PNG 저장 폴더") else {
            self.status = "저장을 취소했습니다".to_string();
            return;
        };
        let doc = self.canvas.doc().clone();
        let pdf_bytes = self.pdf_bytes.clone();
        let stem = self.export_stem();
        self.status = "PNG 내보내는 중…".to_string();
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
        let Some(path) = files::pick_save("pdf", "PDF 문서", &suggested) else {
            self.status = "저장을 취소했습니다".to_string();
            return;
        };
        let doc = self.canvas.doc().clone();
        let pdf_bytes = self.pdf_bytes.clone();
        self.status = "PDF 내보내는 중…".to_string();
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
                self.status = format!("{name} — {count}페이지");
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

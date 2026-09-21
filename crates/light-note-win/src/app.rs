//! WinUI 호스트(셸) — 문서·PDF·도구·표면을 소유하고 elm 화면에 props로 내려보낸다.
//!
//! ## 역할 분담 (예제 20의 골격)
//! - **셸(이 파일)**: 창 제목, 문서/PDF/도구/줌, 표면 재료, 파일 대화상자, 가속기.
//! - **elm 화면**(`light_note_core::ui`): 툴바/사이드바/상태바 + `<Raw>` 표면 슬롯.
//! - 두 방향 계약(예제 11):
//!   - 호스트 → elm: **props**(`NoteAppProps::from_view_model`)
//!   - elm → 호스트: **콜백 prop** → `LocalSender` → `HostMessage::Intent`
//!
//! ## 왜 `#[store]`를 안 쓰나
//! 문서의 진실은 **호스트**에 있다(WinUI 포인터가 곧 편집이기 때문). 예제 20은 elm 쪽
//! 진실을 `#[store]`로 묶으라고 하지만, 여기서는 포인터→문서→표면이 한 스레드에서
//! 이어져야 해서 호스트가 소유하는 편이 단순하고 빠르다.
//!
//! ## 스레드 규칙
//! - `HostMessage`는 `Send`다(`spawn_background` 요구 — 예제 11). 그래서 **PDF 문서를
//!   메시지에 넣지 않는다**(`PdfDocument`는 스레드 경계를 넘기지 않는다).
//! - 백그라운드: 파일 읽기/쓰기/내보내기. UI 스레드: 파싱(지연)과 렌더.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use elm_magic_windows_reactor::{ElmInput, ElmView};
use light_note_core::doc::Document;
use light_note_core::export::{self, EXPORT_SCALE};
use light_note_core::geom::{Point, Size};
use light_note_core::ink::{pressure_from_speed, InkPoint, StrokeStyle, Tool};
use light_note_core::pdf::PdfDocument;
use light_note_core::raster::{page_pixel_size, ViewTransform};
use light_note_core::surface::{live_lines, InkSurface};
use light_note_core::ui::{
    self, NoteApp, NoteAppProps, NoteIntents, NoteViewModel, Phase, SurfaceData,
};
use windows_reactor::{Component, ComponentContext, View, ViewContext};

use crate::files;
use crate::surface::{self, PointerPhase, PointerSink};

/// 호스트 메시지 — `spawn_background`가 돌려주므로 **`Send`**여야 한다.
#[derive(Clone, Debug)]
pub enum HostMessage {
    /// elm 화면의 의도(콜백 prop 이름).
    Intent(&'static str),
    /// 표면 포인터 — 표면 DIP 좌표.
    Pointer(PointerPhase, f64, f64),
    /// 백그라운드에서 읽은 PDF 파일.
    Opened(Result<files::OpenedFile, String>),
    /// 백그라운드 내보내기 결과(저장된 경로).
    Exported(Result<PathBuf, String>),
}

/// 문서·도구·표면을 소유하는 셸.
pub struct Shell {
    document: Document,
    /// 배경 PDF(있으면 페이지마다 배경을 깐다).
    pdf: Option<PdfDocument>,
    /// 원본 PDF 바이트 — 백그라운드 내보내기가 다시 파싱한다(`Send`).
    pdf_bytes: Option<Vec<u8>>,
    tool: Tool,
    width_pt: f32,
    zoom: f32,
    phase: Phase,
    /// 표면 재료(정적 PNG + 라이브 선분). 발행 직전에 `<Raw>`로 넘긴다.
    surface: InkSurface,
    /// 정적 레이어를 다시 만들어야 하는가(획 커밋/되돌리기/페이지 이동/줌).
    surface_dirty: bool,
    /// 포인터 드래그가 진행 중인가.
    drawing: bool,
    /// 속도 기반 압력 계산용 마지막 샘플.
    last_sample: Option<(Point, Instant)>,
    /// 상태바에 띄울 마지막 안내.
    status: String,
}

impl Shell {
    /// 1pt당 DIP — Windows 11의 150% 배율을 기본으로 두고 줌을 곱한다.
    fn scale(&self) -> f32 {
        ViewTransform::DEFAULT_SCALE * self.zoom / 100.0
    }

    /// 현재 도구 + 굵기 → 스타일(색과 반투명은 도구가 정한다).
    fn style(&self) -> StrokeStyle {
        let base = StrokeStyle::for_tool(self.tool);
        StrokeStyle::new(base.color, self.width_pt)
    }

    fn pdf_name(&self) -> Option<String> {
        self.pdf.as_ref().map(PdfDocument::file_name)
    }

    /// 표면을 최신으로 만든다 — **정적 레이어는 필요할 때만**, 라이브는 매번.
    ///
    /// 이 한 줄이 "잉크가 많아도 입력이 끊기지 않는다"의 근거다: 드래그 중에는
    /// 확정 PNG를 다시 만들지 않고 선분 몇 개만 갈아 끼운다.
    fn refresh_surface(&mut self) {
        let size = self.document.active_page().size();
        let scale = self.scale();
        let (width, height) = page_pixel_size(size, scale);

        if self.surface_dirty
            || self.surface.width != width
            || self.surface.height != height
            || (self.surface.scale - scale).abs() > 1e-4
        {
            self.surface =
                InkSurface::build(size, scale, self.document.committed_strokes(), None);
            self.surface_dirty = false;
        }

        self.surface.lines = self
            .document
            .live_stroke()
            .map(|stroke| live_lines(stroke, scale))
            .unwrap_or_default();
    }

    /// 포인터 한 점 — 모델에 반영한다.
    ///
    /// WinUI 포인터에는 압력이 없으므로 **속도로 굵기 변화를 만든다**
    /// ([`pressure_from_speed`]). 펜 태블릿 압력이 필요하면 어댑터가
    /// `PointerEventInfo`를 확장해야 한다(문서화된 한계).
    fn pointer(&mut self, phase: PointerPhase, x: f64, y: f64) {
        let transform = ViewTransform::new(self.scale());
        let point = transform.to_page(x as f32, y as f32);
        let now = Instant::now();
        let pressure = match self.last_sample {
            Some((previous, at)) => {
                let elapsed_ms = now.duration_since(at).as_secs_f32() * 1000.0;
                let speed = if elapsed_ms > 0.0 {
                    point.distance(previous) / elapsed_ms
                } else {
                    0.0
                };
                pressure_from_speed(speed)
            }
            None => 1.0,
        };
        self.last_sample = Some((point, now));

        match phase {
            PointerPhase::Pressed => {
                self.document.begin(
                    self.tool,
                    self.style(),
                    InkPoint::with_pressure(point.x, point.y, pressure),
                );
                self.drawing = true;
            }
            PointerPhase::Moved if self.drawing => {
                self.document
                    .extend(InkPoint::with_pressure(point.x, point.y, pressure));
            }
            PointerPhase::Released if self.drawing => {
                self.document.finish();
                self.drawing = false;
                self.surface_dirty = true;
                self.status = self.document.status_line();
            }
            PointerPhase::Canceled if self.drawing => {
                self.document.cancel();
                self.drawing = false;
                self.surface_dirty = true;
                self.status = "획을 취소했습니다".to_string();
            }
            _ => {}
        }
    }

    /// elm 화면의 의도 하나를 처리한다 (콜백 prop 이름).
    fn intent(&mut self, name: &'static str, context: &ComponentContext<Self>) {
        match name {
            "pen" => {
                self.tool = Tool::Pen;
                self.status = Tool::Pen.hint().to_string();
            }
            "highlighter" => {
                self.tool = Tool::Highlighter;
                self.status = Tool::Highlighter.hint().to_string();
            }
            "eraser" => {
                self.tool = Tool::Eraser;
                self.status = Tool::Eraser.hint().to_string();
            }
            "thinner" => self.width_pt = (self.width_pt - 0.5).max(StrokeStyle::MIN_WIDTH_PT),
            "thicker" => self.width_pt = (self.width_pt + 0.5).min(StrokeStyle::MAX_WIDTH_PT),
            "undo" => {
                self.status = match self.document.undo() {
                    Some(edit) => format!("{} — 되돌렸습니다", edit.label()),
                    None => "되돌릴 편집이 없습니다".to_string(),
                };
                self.surface_dirty = true;
            }
            "redo" => {
                self.status = match self.document.redo() {
                    Some(edit) => format!("{} — 다시 적용", edit.label()),
                    None => "다시 적용할 편집이 없습니다".to_string(),
                };
                self.surface_dirty = true;
            }
            "clear" => {
                self.status = if self.document.clear_active_page() {
                    "페이지를 비웠습니다 (되돌리기 가능)".to_string()
                } else {
                    "비울 스트로크가 없습니다".to_string()
                };
                self.surface_dirty = true;
            }
            "open" => self.open_pdf(context),
            "export_png" => self.export_png(context),
            "export_pdf" => self.export_pdf(context),
            "page_add" => {
                let size = self.document.active_page().size();
                self.document.add_page_after_active(size);
                self.status = self.document.status_line();
                self.surface_dirty = true;
            }
            "page_remove" => {
                let index = self.document.active_index();
                self.status = if self.document.remove_page(index) {
                    self.document.status_line()
                } else {
                    "마지막 페이지는 지울 수 없습니다".to_string()
                };
                self.surface_dirty = true;
            }
            "page_prev" => {
                if self.document.step_page(-1) {
                    self.surface_dirty = true;
                    self.status = self.document.status_line();
                }
            }
            "page_next" => {
                if self.document.step_page(1) {
                    self.surface_dirty = true;
                    self.status = self.document.status_line();
                }
            }
            "zoom_in" => {
                self.zoom = (self.zoom + 25.0).min(400.0);
                self.surface_dirty = true;
                self.status = format!("확대 {:.0}%", self.zoom);
            }
            "zoom_out" => {
                self.zoom = (self.zoom - 25.0).max(25.0);
                self.surface_dirty = true;
                self.status = format!("확대 {:.0}%", self.zoom);
            }
            "retry" => {
                self.phase = Phase::Empty;
                self.status = "다시 시작합니다".to_string();
            }
            other => self.status = format!("알 수 없는 의도: {other}"),
        }
    }

    /// PDF 열기 — 대화상자(UI 스레드) → 읽기(백그라운드) → 파싱(지연 파싱이라 싸다).
    fn open_pdf(&mut self, context: &ComponentContext<Self>) {
        let Some(path) = files::pick_pdf() else {
            self.status = "열기를 취소했습니다".to_string();
            return;
        };
        self.phase = Phase::Loading;
        self.status = format!("{} 여는 중…", path.display());
        let _ = context.spawn_background(move |_cancel| HostMessage::Opened(files::read_pdf(path)));
    }

    /// PNG 내보내기 — 페이지마다 파일 하나. 렌더는 백그라운드에서.
    fn export_png(&mut self, context: &ComponentContext<Self>) {
        let Some(directory) = files::pick_folder("PNG 저장 폴더") else {
            self.status = "저장을 취소했습니다".to_string();
            return;
        };
        let document = self.document.clone();
        let pdf_bytes = self.pdf_bytes.clone();
        let stem = self.export_stem();
        self.status = "PNG 내보내는 중…".to_string();
        let _ = context.spawn_background(move |_cancel| {
            // 배경 PDF는 **여기서** 다시 파싱한다(스레드 경계를 넘기지 않는다).
            let source = pdf_bytes.and_then(|bytes| PdfDocument::from_bytes(bytes).ok());
            let result = export::export_document_png(
                &document,
                source.as_ref(),
                &directory,
                &stem,
                EXPORT_SCALE,
            )
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
        let document = self.document.clone();
        let pdf_bytes = self.pdf_bytes.clone();
        self.status = "PDF 내보내는 중…".to_string();
        let _ = context.spawn_background(move |_cancel| {
            let source = pdf_bytes.and_then(|bytes| PdfDocument::from_bytes(bytes).ok());
            let result = export::export_pdf(&document, source.as_ref(), &path)
                .map_err(|error| error.to_string());
            HostMessage::Exported(result)
        });
    }

    /// 내보낼 파일 이름의 기본값.
    fn export_stem(&self) -> String {
        let title = self.document.title().trim();
        let stem = title.strip_suffix(".pdf").unwrap_or(title);
        if stem.is_empty() {
            "light-note".to_string()
        } else {
            stem.to_string()
        }
    }

    /// 백그라운드가 읽어 온 PDF를 문서로 만든다.
    fn adopt_pdf(&mut self, file: files::OpenedFile) {
        let name = file.file_name();
        let stem = file.stem();
        match PdfDocument::from_bytes(file.bytes.clone()) {
            Ok(pdf) => {
                let sizes = pdf.page_sizes();
                let count = sizes.len();
                self.document = Document::from_pdf_pages(sizes, stem);
                self.pdf = Some(pdf);
                self.pdf_bytes = Some(file.bytes);
                self.phase = Phase::Ready;
                self.status = format!("{name} — {count}페이지");
            }
            Err(error) => {
                self.status = error.to_string();
                self.phase = Phase::Failed(error.to_string());
            }
        }
        self.surface_dirty = true;
    }
}

impl Component for Shell {
    type Input = ();
    type Message = HostMessage;

    fn create(_input: &(), _context: &ComponentContext<Self>) -> Self {
        let mut shell = Self {
            document: Document::blank(Size::A4),
            pdf: None,
            pdf_bytes: None,
            tool: Tool::Pen,
            width_pt: StrokeStyle::PEN_WIDTH_PT,
            zoom: 100.0,
            phase: Phase::Empty,
            surface: InkSurface::build(Size::A4, ViewTransform::DEFAULT_SCALE, &[], None),
            surface_dirty: true,
            drawing: false,
            last_sample: None,
            status: "새 노트 — 펜으로 그려 보세요".to_string(),
        };
        shell.refresh_surface();
        shell
    }

    fn update(&mut self, message: HostMessage, context: &ComponentContext<Self>) {
        match message {
            HostMessage::Intent(name) => self.intent(name, context),
            HostMessage::Pointer(phase, x, y) => self.pointer(phase, x, y),
            HostMessage::Opened(Ok(file)) => self.adopt_pdf(file),
            HostMessage::Opened(Err(error)) => {
                self.status = error.clone();
                self.phase = Phase::Failed(error);
            }
            HostMessage::Exported(Ok(path)) => self.status = format!("저장: {}", path.display()),
            HostMessage::Exported(Err(error)) => {
                self.status = error.clone();
                self.phase = Phase::Failed(error);
            }
        }
        // 어떤 메시지든 표면을 최신으로 맞춘다(정적은 필요할 때만).
        self.refresh_surface();
    }

    fn view(&self, _input: &(), context: &mut ViewContext<Self>) -> View {
        // 창 제목은 호스트가 정한다 (elm은 플랫폼을 모른다 — 예제 20).
        context.window_title(format!("light-note — {}", self.document.title()));

        let sender = context.sender();

        // ① 포인터 → 메시지 큐. 표면 빌더가 이 싱크를 **캡처**한다(예제 08).
        let pointer_sender = sender.clone();
        let sink: PointerSink = Rc::new(move |phase, x, y| {
            let _ = pointer_sender.send(HostMessage::Pointer(phase, x, y));
        });

        // ② 가속기(Reactor가 지원하는 키만) → 같은 큐.
        let accelerator_sender = sender.clone();
        let accelerators = surface::default_accelerators(move |intent| {
            let _ = accelerator_sender.send(HostMessage::Intent(intent));
        });
        ui::set_surface_builder(Rc::new(move |data: &SurfaceData| {
            surface::build(data, &sink, accelerators.clone())
        }));

        // ③ 발행 직전에 표면 재료를 넣는다 — `<Raw>`가 꺼내 간다(슬롯을 못 보므로).
        ui::stage_surface(SurfaceData {
            png: self.surface.static_png.clone(),
            lines: self.surface.lines.clone(),
            width: self.surface.width,
            height: self.surface.height,
            scale: self.surface.scale,
        });

        // ④ 호스트 → elm: props / elm → 호스트: 콜백 prop (예제 11).
        let view_model =
            NoteViewModel::from_document(&self.document, self.tool, self.width_pt, self.zoom)
                .with_pdf(self.pdf_name().as_deref())
                .with_surface(&self.surface)
                .with_phase(self.phase.clone());

        let intent_sender = sender.clone();
        let intents = NoteIntents::each(move |name| {
            let _ = intent_sender.send(HostMessage::Intent(name));
        });

        View::component::<ElmView<NoteApp>>(ElmInput::new(NoteAppProps::from_view_model(
            &view_model,
            &intents,
        )))
    }
}

/// 앱 시작 — 예제 20의 골격.
///
/// `#[store] struct App`을 만들지 않는다: 이 모듈에 `struct App`이 생기면
/// `windows_reactor::App`과 충돌한다(E0255) — 그래서 경로로 쓴다.
pub fn run() {
    windows_reactor::App::run_component::<Shell>(()).expect("Reactor 실행 실패");
}

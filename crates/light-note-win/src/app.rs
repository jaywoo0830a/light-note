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
//! - 백그라운드: 파일 읽기/쓰기/내보내기, **정적 레이어 래스터 + PNG 인코딩**.
//!   UI 스레드: elm 프레임과 라이브 도형(도형 몇 개). 페이지 전체를 다시 그리는 일은
//!   포인터 메시지 안에서 하지 않는다 — 실측 A4 한 장 = 13ms(release)/277ms(dev).
//!   그 사이에 확정된 획은 라이브 도형으로 계속 그린다([`pending_ink`]).
//! - **플리커 금지(구조적)**: 정적 레이어는 두 장이고([`StaticLayers`]), 새 그림은
//!   보이지 않는 뒤 레이어에 올라간 뒤 **디코드 완료 신호**(`ImageOpened` →
//!   [`HostMessage::LayerReady`])를 받고서야 앞뒤가 바뀐다. 어댑터가 소스를 비우고
//!   비동기로 디코드하는 사이 빈 프레임이 화면에 노출될 수 있는 경로가 없다.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use elm_magic_windows_reactor::{ElmInput, ElmView};
use light_note_core::doc::Document;
use light_note_core::export::{self, EXPORT_SCALE};
use light_note_core::geom::{Point, Size};
use light_note_core::history::Edit;
use light_note_core::ink::{pressure_from_speed, InkPoint, Stroke, StrokeStyle, Tool};
use light_note_core::pdf::PdfDocument;
use light_note_core::raster::{page_pixel_size, ViewTransform};
use light_note_core::surface::{has_ink, pending_ink, static_layer, InkSurface};
use light_note_core::ui::{
    self, NoteApp, NoteAppProps, NoteIntents, NoteViewModel, Phase, SurfaceData,
};
use windows_reactor::{Component, ComponentContext, View, ViewContext};

use crate::files;
use crate::surface::{self, LayerSink, PointerPhase, PointerSink};

/// 디코드 완료 신호를 기다리는 시간 — 신호가 끝내 오지 않으면 이만큼 뒤에 승격한다(안전망).
const LAYER_SETTLE: Duration = Duration::from_millis(150);

/// 승격을 허용하는 최소 경과 — 이보다 최근에 스테이징됐다면 아직 디코드 중일 수 있다
/// (성급히 승격하면 갓 비워진 소스가 화면에 드러난다 = 깜빡임).
const LAYER_SETTLE_GRACE: Duration = Duration::from_millis(120);

/// 호스트 메시지 — `spawn_background`가 돌려주므로 **`Send`**여야 한다.
#[derive(Clone, Debug)]
pub enum HostMessage {
    /// elm 화면의 의도(콜백 prop 이름).
    Intent(&'static str),
    /// 표면 포인터 — 표면 DIP 좌표.
    Pointer(PointerPhase, f64, f64),
    /// 백그라운드에서 만든 정적 레이어 — `generation`은 낡은 결과를 버리는 기준,
    /// `count`는 그 PNG가 반영한 확정 스트로크 수다.
    StaticLayer {
        generation: u64,
        count: usize,
        png: Option<Arc<[u8]>>,
    },
    /// 뒤 레이어의 **디코드가 끝났다**(`ImageOpened`) — 이제 앞뒤를 바꿔도 안전하다.
    LayerReady(usize),
    /// 안전망: 디코드 신호 없이 충분히 기다렸다 — 승격해도 안전하다.
    LayerSettled,
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
    /// 표면 재료(정적 PNG + 라이브 도형). 발행 직전에 `<Raw>`로 넘긴다.
    surface: InkSurface,
    /// 정적 레이어를 다시 만들어야 하는가(획 커밋/되돌리기/페이지 이동/줌).
    ///
    /// 이 플래그가 서 있으면 표면을 갱신하는 김에 **백그라운드**에 맡긴다
    /// ([`Shell::request_static`]) — UI 스레드는 래스터/인코딩을 기다리지 않는다.
    surface_dirty: bool,
    /// 진행 중인 정적 렌더의 세대(없으면 `None`) — 한 번에 하나만 돌린다.
    static_inflight: Option<u64>,
    /// 세대 카운터 — 낡은 렌더 결과를 버리는 기준(되돌리기/지우개/페이지 이동/줌).
    static_generation: u64,
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

    /// 표면을 최신으로 만든다 — **정적은 백그라운드, 라이브는 즉시**.
    ///
    /// 이 구분이 "획을 끝내도 화면이 멈추지 않는다"의 근거다:
    /// - 정적 레이어(A4 = 13ms release / 277ms dev)는 워커가 만들고 결과를 메시지로 받는다.
    /// - 라이브 레이어는 도형 몇 개만 갈아 끼운다(600점 = 0.1ms).
    /// - PNG가 도착하기 전에 확정된 획은 꼬리로 남아 라이브 레이어가 그린다.
    fn refresh_surface(&mut self, context: &ComponentContext<Self>) {
        let size = self.document.active_page().size();
        let scale = self.scale();
        let (width, height) = page_pixel_size(size, scale);

        let geometry_changed = self.surface.width != width
            || self.surface.height != height
            || (self.surface.scale - scale).abs() > 1e-4;
        if geometry_changed {
            self.surface.width = width;
            self.surface.height = height;
            self.surface.scale = scale;
            // 크기/배율이 바뀌었다 — 화면의 PNG는 새 크기로 그려야 한다. **지우지는 않는다**:
            // 새 그림이 디코드될 때까지 옛 그림이 그대로 보이는 편이 빈 페이지가 번쩍이는
            // 것보다 낫다(플리커 없음). 새 그림은 승격될 때 이 크기로 나타난다.
            self.mark_static_dirty();
        }

        if self.surface_dirty && self.static_inflight.is_none() {
            self.surface_dirty = false;
            self.request_static(context, size);
        }

        // 꼬리의 기준은 **화면에 보이는 레이어**가 반영한 획 수다 — 아직 승격되지 않은
        // 획은 라이브 도형으로 계속 그린다(그래서 PNG가 도착하기 전에도 잉크가 보인다).
        // 라이브 도형은 래스터와 **같은 기하**라, 승격 순간에도 잉크 모양이 바뀌지 않는다.
        self.surface.ink = pending_ink(
            self.document.committed_strokes(),
            self.surface.layers.visible_count(),
            self.document.live_stroke(),
            scale,
        );
    }

    /// 정적 레이어를 **백그라운드에** 맡긴다 — UI 스레드는 여기서 기다리지 않는다.
    ///
    /// 스트로크 목록을 **스냅샷으로 복사**해 넘기고(`Stroke`는 `Send`), 결과는
    /// [`HostMessage::StaticLayer`]로 돌아온다. 그 사이 문서가 또 바뀌면 세대가 달라져
    /// 낡은 결과는 버려진다([`Shell::mark_static_dirty`]).
    fn request_static(&mut self, context: &ComponentContext<Self>, size: Size) {
        let committed = self.document.committed_strokes();
        if !has_ink(committed) {
            // 그릴 잉크가 없다 — 래스터도 워커도 필요 없다(빈 레이어를 바로 스테이징).
            self.stage_static(context, None, committed.len());
            return;
        }

        let strokes: Vec<Stroke> = committed.to_vec();
        let count = strokes.len();
        let scale = self.surface.scale;
        self.static_generation += 1;
        let generation = self.static_generation;
        self.static_inflight = Some(generation);

        let _ = context.spawn_background(move |_cancel| HostMessage::StaticLayer {
            generation,
            count,
            png: static_layer(&strokes, size, scale),
        });
    }

    /// 백그라운드가 만든 PNG를 받는다 — **최신 세대만** 받는다.
    ///
    /// 새 그림은 **보이지 않는 뒤 레이어**에 올라간다. 화면은 그대로다 — 뒤 레이어의
    /// 디코드가 끝나 `ImageOpened`가 오면 [`Shell::layer_decoded`]가 앞뒤를 바꾼다.
    /// 그 사이 방금 확정한 획은 라이브 도형(꼬리)이 그리고 있으므로 **빈 틈도, 깜빡임도 없다**.
    fn adopt_static(
        &mut self,
        context: &ComponentContext<Self>,
        generation: u64,
        count: usize,
        png: Option<Arc<[u8]>>,
    ) {
        if self.static_inflight != Some(generation) {
            return; // 그 사이 되돌리기/지우개/페이지 이동이 있었다 — 낡은 그림이다
        }
        self.static_inflight = None;
        self.stage_static(
            context,
            png,
            count.min(self.document.committed_strokes().len()),
        );
    }

    /// 그림을 뒤 레이어에 올린다 — 이미 같은 그림이 디코드돼 있으면 기다리지 않고 바꾼다.
    ///
    /// 아니면 **디코드 완료 신호를 기다린다**(`ImageOpened` → [`HostMessage::LayerReady`]).
    /// 그 신호가 끝내 오지 않는 경우(창 밖 요소를 WinUI가 게으르게 다룰 때)를 대비해
    /// 안전망도 함께 건다 — 잠깐 기다렸다가, 그 사이 새 스테이징이 없었으면 승격한다.
    /// 어느 쪽이든 기다리는 동안 화면은 옛 레이어 + 라이브 꼬리라 **비지 않는다**.
    fn stage_static(
        &mut self,
        context: &ComponentContext<Self>,
        png: Option<Arc<[u8]>>,
        count: usize,
    ) {
        if self.surface.layers.stage(png, count) {
            self.surface.layers.promote_staged();
            return;
        }
        let _ = context.spawn_background(move |_cancel| {
            std::thread::sleep(LAYER_SETTLE);
            HostMessage::LayerSettled
        });
    }

    /// 뒤 레이어의 디코드가 끝났다(`ImageOpened`) → 앞뒤를 바꾼다.
    ///
    /// **이 신호가 있어야만 화면이 바뀐다** — 소스가 갓 바뀐(그래서 잠시 비어 있는)
    /// 레이어가 화면에 노출되는 일이 구조적으로 없다.
    fn layer_decoded(&mut self, index: usize) {
        self.surface.layers.promote(index);
    }

    /// 정적 레이어가 **낡았다**고 표시한다(내용/페이지/배율이 바뀌었다).
    ///
    /// 화면의 레이어는 **그대로 둔다**: 새 그림이 디코드되어 승격될 때까지 옛 그림이
    /// 보인다(빈 페이지가 번쩍이는 것보다 낫다 — 플리커 금지). 진행 중이던 렌더 결과는
    /// 세대를 올려 버린다(되돌린 스트로크가 되살아나지 않는다).
    fn mark_static_dirty(&mut self) {
        self.surface_dirty = true;
        self.static_generation += 1;
        self.static_inflight = None;
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
                self.drawing = false;
                match self.document.finish() {
                    // 스트로크 하나가 **늘었다** — 기존 PNG는 그대로 쓸 수 있다.
                    // 새 획은 PNG가 도착할 때까지 라이브 레이어가 꼬리로 그린다.
                    Some(Edit::AddStroke { .. }) => self.surface_dirty = true,
                    // 지운 결과는 PNG에 반영돼야 한다(화면의 PNG는 옛 상태다).
                    Some(_) => self.mark_static_dirty(),
                    None => {}
                }
                self.status = self.document.status_line();
            }
            PointerPhase::Canceled if self.drawing => {
                self.drawing = false;
                self.document.cancel();
                // 취소는 드래그 **직전 상태**로 되돌린다 — 화면의 레이어가 이미 그 상태면
                // 다시 그릴 필요가 없다(포인터 캡처 유실에서 화면이 멈추지 않는다).
                if self.document.committed_strokes().len() != self.surface.layers.visible_count() {
                    self.mark_static_dirty();
                }
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
                self.mark_static_dirty();
            }
            "redo" => {
                self.status = match self.document.redo() {
                    Some(edit) => format!("{} — 다시 적용", edit.label()),
                    None => "다시 적용할 편집이 없습니다".to_string(),
                };
                self.mark_static_dirty();
            }
            "clear" => {
                self.status = if self.document.clear_active_page() {
                    "페이지를 비웠습니다 (되돌리기 가능)".to_string()
                } else {
                    "비울 스트로크가 없습니다".to_string()
                };
                self.mark_static_dirty();
            }
            "open" => self.open_pdf(context),
            "export_png" => self.export_png(context),
            "export_pdf" => self.export_pdf(context),
            "page_add" => {
                let size = self.document.active_page().size();
                self.document.add_page_after_active(size);
                self.status = self.document.status_line();
                self.mark_static_dirty();
            }
            "page_remove" => {
                let index = self.document.active_index();
                self.status = if self.document.remove_page(index) {
                    self.document.status_line()
                } else {
                    "마지막 페이지는 지울 수 없습니다".to_string()
                };
                self.mark_static_dirty();
            }
            "page_prev" => {
                if self.document.step_page(-1) {
                    self.mark_static_dirty();
                    self.status = self.document.status_line();
                }
            }
            "page_next" => {
                if self.document.step_page(1) {
                    self.mark_static_dirty();
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
        // 새 문서다 — 화면의 레이어는 새 그림이 승격될 때 바뀐다(깜빡이지 않는다).
        self.mark_static_dirty();
    }
}

impl Component for Shell {
    type Input = ();
    type Message = HostMessage;

    fn create(_input: &(), context: &ComponentContext<Self>) -> Self {
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
            static_inflight: None,
            static_generation: 0,
            drawing: false,
            last_sample: None,
            status: "새 노트 — 펜으로 그려 보세요".to_string(),
        };
        shell.refresh_surface(context);
        shell
    }

    fn update(&mut self, message: HostMessage, context: &ComponentContext<Self>) {
        match message {
            HostMessage::Intent(name) => self.intent(name, context),
            HostMessage::Pointer(phase, x, y) => self.pointer(phase, x, y),
            HostMessage::StaticLayer {
                generation,
                count,
                png,
            } => self.adopt_static(context, generation, count, png),
            HostMessage::LayerReady(index) => self.layer_decoded(index),
            HostMessage::LayerSettled => {
                // 안전망 — 충분히 기다렸고 그 사이 새 스테이징이 없었을 때만 바꾼다.
                self.surface.layers.promote_if_settled(LAYER_SETTLE_GRACE);
            }
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
        // 어떤 메시지든 표면을 최신으로 맞춘다 — 정적은 필요할 때 백그라운드에 맡기고,
        // 라이브(진행 중인 획 + 아직 PNG에 없는 꼬리)는 여기서 바로 만든다.
        self.refresh_surface(context);
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

        // ② 디코드 완료 → 레이어 교체. **이 신호 뒤에만** 화면이 바뀐다(빈 프레임 없음).
        let layer_sender = sender.clone();
        let layers: LayerSink = Rc::new(move |index| {
            let _ = layer_sender.send(HostMessage::LayerReady(index));
        });

        // ③ 가속기(Reactor가 지원하는 키만) → 같은 큐.
        let accelerator_sender = sender.clone();
        let accelerators = surface::default_accelerators(move |intent| {
            let _ = accelerator_sender.send(HostMessage::Intent(intent));
        });
        ui::set_surface_builder(Rc::new(move |data: &SurfaceData| {
            surface::build(data, &sink, &layers, accelerators.clone())
        }));

        // ④ 발행 직전에 표면 재료를 넣는다 — `<Raw>`가 꺼내 간다(슬롯을 못 보므로).
        ui::stage_surface(SurfaceData::from_surface(&self.surface));

        // ⑤ 호스트 → elm: props / elm → 호스트: 콜백 prop (예제 11).
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

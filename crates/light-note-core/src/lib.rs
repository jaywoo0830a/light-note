//! # light-note-core — 플랫폼 독립 코어
//!
//! 윈도우 11에 최적화된 드로잉패드 필기 앱의 **엔진**이다. UI(WinUI 3)와
//! 플랫폼 코드는 [`crate::ui`]의 화면 정의와 얇은 호스트에만 있고, 나머지는
//! 전부 여기서 돌아가며 **리눅스에서도 테스트된다**.
//!
//! ## 층
//! | 모듈 | 하는 일 | 플랫폼 |
//! |---|---|---|
//! | [`geom`] | 좌표계(pt, 좌상단 원점)와 경계 상자 | 어디서나 |
//! | [`ink`] | 도구/스타일/압력 샘플/스트로크 | 어디서나 |
//! | [`doc`] | 페이지·문서·진행 중인 획 | 어디서나 |
//! | [`history`] | 편집 단위(드래그 = Undo 1회) | 어디서나 |
//! | [`raster`] | 스트로크 → 픽셀(hayro의 vello_cpu) | 어디서나 |
//! | [`pdf`] | **hayro**로 PDF 열기/페이지 크기/래스터화 | 어디서나 |
//! | [`export`] | PNG / PDF 내보내기 | 어디서나 |
//! | [`surface`] | WinUI 표면이 무엇을 그릴지(정적 PNG + 라이브 도형) | 어디서나 |
//! | [`ui`] | elm-magic 화면(`view!`) + 어댑터 계획(plan) 계약 | 어디서나 |
//!
//! ## 데이터 흐름
//! ```text
//! WinUI 포인터 ──▶ host(light-note-win) ──▶ Document::begin/extend/finish
//!                                              │
//!                        elm-magic 화면 ◀──────┘ props(도구/페이지/상태)
//!                                              │
//!                    <Raw> 표면 ◀── InkSurface(정적 PNG + 라이브 도형)
//! ```
//!
//! ## 테스트 중심 설계
//! 표면이 "무엇을 그릴지"까지 코어가 계산하므로, WinUI 코드는 번역만 한다.
//! 그래서 `cargo test -p light-note-core`가 UI 계약까지 검증한다.
//!
//! (문서는 모듈 상단과 공개 API에 붙어 있다 — `missing_docs` 린트는 끄고
//! "왜 이 API가 필요한가"를 설명하는 주석을 우선한다.)

pub mod doc;
pub mod export;
pub mod geom;
pub mod history;
pub mod ink;
pub mod pdf;
pub mod raster;
pub mod surface;
pub mod ui;

pub use doc::{Document, ERASER_RADIUS_PT, Page};
pub use geom::{Bounds, Point, Size};
pub use history::{Edit, History};
pub use ink::{InkPoint, Rgba, Stroke, StrokeStyle, Tool};
pub use pdf::{PdfDocument, PdfError};
pub use raster::{
    ViewTransform, composite_into, ink_coverage, page_pixel_size, pixel_at, render_ink,
    render_page, render_page_on_white,
};
pub use surface::{InkSurface, SurfaceLine};

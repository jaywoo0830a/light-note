//! PDF 읽기 — **hayro**로 페이지를 열고 배경 래스터를 만든다.
//!
//! 계약:
//! - 원본 PDF는 **읽기 전용**이다(light-note는 원본을 절대 고치지 않는다).
//!   필기 결과는 [`crate::export`]가 새 PDF/PNG로 저장한다.
//! - 페이지 크기는 pt이고 좌표는 모델과 같이 **좌상단 원점**으로 돌려준다
//!   (PDF 내부의 좌하단 원점은 여기서 흡수한다).
//!
//! 파싱은 지연(xref/trailer만 먼저)이라 UI 스레드에서 열어도 싸다 — 무거운 래스터만
//! ④-워커로 넘긴다.

use std::path::{Path, PathBuf};

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::peniko::color::{AlphaColor, Srgb};
use hayro::vello_cpu::Pixmap;
use hayro::{render, RenderCache, RenderSettings};

use crate::geom::Size;

/// PDF를 열지 못한 이유 — UI가 문장 그대로 보여줄 수 있게 한다.
#[derive(Clone, Debug, PartialEq)]
pub enum PdfError {
    /// 파일을 읽지 못했다.
    Io(String),
    /// PDF로 해석하지 못했다(손상/암호화).
    Parse(String),
    /// 페이지가 하나도 없다.
    NoPages,
    /// 페이지 인덱스가 범위를 벗어났다.
    PageOutOfRange(usize),
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PdfError::Io(message) => write!(formatter, "Could not read the file: {message}"),
            PdfError::Parse(message) => write!(formatter, "Could not parse the PDF: {message}"),
            PdfError::NoPages => write!(formatter, "The PDF has no pages"),
            PdfError::PageOutOfRange(index) => {
                write!(formatter, "Page {} does not exist", index + 1)
            }
        }
    }
}

impl std::error::Error for PdfError {}

/// 열린 PDF 문서 + 페이지 크기 캐시.
///
/// `hayro_syntax::Pdf`가 바이트를 소유하므로 문서를 그대로 들고 있어도 안전하다
/// (자기 참조 문제가 없다). 다만 **스레드 경계는 넘기지 않는다** — 필요하면 워커가
/// 바이트에서 다시 파싱한다.
pub struct PdfDocument {
    pdf: Pdf,
    path: Option<PathBuf>,
    sizes: Vec<Size>,
}

impl PdfDocument {
    /// 파일에서 연다.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PdfError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|error| PdfError::Io(error.to_string()))?;
        let mut document = Self::from_bytes(bytes)?;
        document.path = Some(path.to_path_buf());
        Ok(document)
    }

    /// 메모리 바이트에서 연다.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, PdfError> {
        let pdf = Pdf::new(bytes).map_err(|error| PdfError::Parse(format!("{error:?}")))?;
        let sizes: Vec<Size> = pdf
            .pages()
            .iter()
            .map(|page| {
                let (width, height) = page.render_dimensions();
                Size::new(width, height)
            })
            .collect();
        if sizes.is_empty() {
            return Err(PdfError::NoPages);
        }
        Ok(Self {
            pdf,
            path: None,
            sizes,
        })
    }

    /// 원본 경로(메모리에서 열었으면 `None`).
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 파일 이름 — 창 제목/상태바에 쓴다.
    pub fn file_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "in-memory PDF".to_string())
    }

    pub fn page_count(&self) -> usize {
        self.sizes.len()
    }

    /// 페이지 크기(pt).
    pub fn page_size(&self, index: usize) -> Option<Size> {
        self.sizes.get(index).copied()
    }

    pub fn page_sizes(&self) -> Vec<Size> {
        self.sizes.clone()
    }

    /// 페이지를 픽셀로 래스터화한다 — 필기 배경이자 PDF 내보내기용 이미지.
    ///
    /// 배경은 **흰 종이**로 렌더한다(투명 배경 위에 필기하면 PDF 뷰어마다 결과가 다르다).
    pub fn render_page(&self, index: usize, scale: f32) -> Result<Pixmap, PdfError> {
        if index >= self.sizes.len() {
            return Err(PdfError::PageOutOfRange(index));
        }
        let cache = RenderCache::new();
        let settings = InterpreterSettings::default();
        let page = &self.pdf.pages()[index];
        Ok(render(
            page,
            &cache,
            &settings,
            &RenderSettings {
                x_scale: scale,
                y_scale: scale,
                width: None,
                height: None,
                bg_color: AlphaColor::<Srgb>::from_rgba8(255, 255, 255, 255),
            },
        ))
    }

    /// 배경 인덱스가 `None`이면 건너뛴다(빈 페이지 편의).
    pub fn render_optional(&self, index: Option<usize>, scale: f32) -> Option<Pixmap> {
        index.and_then(|index| self.render_page(index, scale).ok())
    }
}

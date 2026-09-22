//! 파일 대화상자와 파일 IO — 윈도우 11의 `IFileDialog`(rfd)를 그대로 쓴다.
//!
//! ## 스레드 규칙 (중요)
//! - **대화상자는 UI 스레드**에서 띄운다 — 모달 창이 우리 창에 붙어야 하기 때문이다.
//! - **읽기/쓰기/내보내기는 백그라운드**에서 한다(`ComponentContext::spawn_background`).
//!   그래서 여기서 만드는 값은 전부 `Send`다(경로·바이트).

use std::path::PathBuf;

/// 연 파일 — 경로 + 바이트 (둘 다 `Send`).
#[derive(Clone, Debug)]
pub struct OpenedFile {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

impl OpenedFile {
    /// 창 제목/상태바에 쓸 이름.
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "PDF".to_string())
    }

    /// 저장 파일 이름의 기본값(확장자 제거).
    pub fn stem(&self) -> String {
        self.path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "light-note".to_string())
    }
}

/// PDF 열기 대화상자 — **UI 스레드**에서 부른다. 취소하면 `None`.
pub fn pick_pdf() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("PDF 열기")
        .add_filter("PDF 문서", &["pdf"])
        .pick_file()
}

/// 경로에서 바이트를 읽는다 — **백그라운드**에서 부른다.
pub fn read_pdf(path: PathBuf) -> Result<OpenedFile, String> {
    let bytes = std::fs::read(&path).map_err(|error| format!("파일을 읽지 못했습니다: {error}"))?;
    Ok(OpenedFile { path, bytes })
}

/// 저장 경로를 묻는다 — **UI 스레드**에서 부른다.
pub fn pick_save(extension: &str, filter: &str, suggested: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("저장")
        .add_filter(filter, &[extension])
        .set_file_name(suggested)
        .save_file()
}

/// 폴더를 묻는다(PNG 여러 장 내보내기) — **UI 스레드**에서 부른다.
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    rfd::FileDialog::new().set_title(title).pick_folder()
}

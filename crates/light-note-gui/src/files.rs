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
        .set_title("Open PDF")
        .add_filter("PDF document", &["pdf"])
        .pick_file()
}

/// 경로에서 바이트를 읽는다 — **백그라운드**에서 부른다.
pub fn read_pdf(path: PathBuf) -> Result<OpenedFile, String> {
    let bytes =
        std::fs::read(&path).map_err(|error| format!("Could not read the file: {error}"))?;
    Ok(OpenedFile { path, bytes })
}

/// 끌어놓은 경로들에서 **첫 PDF**를 고른다 — 파일을 읽기 전에 거른다(순수 함수).
///
/// 확장자만 보고(대소문자 무시) **읽지 않는다**: 놓인 파일이 PDF가 아니면 여기서 끝나고,
/// 상태 띠가 "왜 안 열리는지"를 말한다([`crate::app::Shell`]). 여러 개를 놓으면 첫 PDF 하나만
/// 연다 — 지금은 문서가 하나뿐이라 그 이상은 의미가 없다.
pub fn first_pdf(paths: impl IntoIterator<Item = String>) -> Option<PathBuf> {
    paths
        .into_iter()
        .map(PathBuf::from)
        .find(|path| is_pdf(path))
}

/// PDF 확장자인가 — **대소문자를 가리지 않는다**(`.PDF`도 PDF다).
fn is_pdf(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

/// 저장 경로를 묻는다 — **UI 스레드**에서 부른다.
pub fn pick_save(extension: &str, filter: &str, suggested: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Save")
        .add_filter(filter, &[extension])
        .set_file_name(suggested)
        .save_file()
}

/// 폴더를 묻는다(PNG 여러 장 내보내기) — **UI 스레드**에서 부른다.
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    rfd::FileDialog::new().set_title(title).pick_folder()
}

//! 끌어놓기의 **판단** — 화면 없이 검증한다.
//!
//! `drag-drop`은 공식 windows-rs 패턴이지만 판단 자체(어떤 경로를 열 것인가)는 순수 함수다.
//! 그래서 WinUI 없이 여기서 못 박는다: 창을 띄우지 않고도 회귀가 잡힌다.

use std::path::PathBuf;

use light_note_gui::files::first_pdf;

/// 호스트가 `DroppedData::StorageItems`에서 모으는 모양 그대로(경로 문자열 목록).
fn paths(list: &[&str]) -> Vec<String> {
    list.iter().map(|path| (*path).to_string()).collect()
}

#[test]
fn the_first_pdf_wins() {
    // 여러 개를 놓으면 **첫 PDF 하나**만 연다(문서가 하나뿐이라 그 이상은 의미가 없다).
    let picked = first_pdf(paths(&[
        "C:\\notes\\readme.txt",
        "C:\\notes\\sketch.pdf",
        "C:\\notes\\other.pdf",
    ]));
    assert_eq!(picked, Some(PathBuf::from("C:\\notes\\sketch.pdf")));
}

#[test]
fn the_extension_is_matched_without_case() {
    // `.PDF`도 PDF다 — 대소문자로 갈리면 사용자는 이유를 알 수 없다.
    assert_eq!(
        first_pdf(paths(&["C:\\notes\\BIG.PDF"])),
        Some(PathBuf::from("C:\\notes\\BIG.PDF"))
    );
}

#[test]
fn nothing_else_opens() {
    // PDF가 아니면 **아무것도 열지 않는다**(상태 띠가 이유를 말한다).
    assert_eq!(first_pdf(paths(&[])), None);
    assert_eq!(first_pdf(paths(&["C:\\notes\\a.txt"])), None);
    // 확장자가 없는 파일도, 이름이 `.pdf`뿐인 숨김 파일도 PDF가 아니다.
    assert_eq!(first_pdf(paths(&["C:\\notes\\a"])), None);
    assert_eq!(first_pdf(paths(&["C:\\notes\\.pdf"])), None);
}

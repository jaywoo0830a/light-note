//! 인코딩 계약 — 윈도우에서 **같은 사고를 반복하지 않게** 기계가 지킨다.
//!
//! ## 왜 테스트인가
//! `run-windows.ps1`이 UTF-8(**BOM 없음**)이었고 `powershell`(Windows PowerShell **5.1**)로
//! 돌리자 5.1이 그 파일을 **ANSI(CP949)**로 읽었다. 한글이 깨진 정도가 아니라 문자열 종결자
//! `'` 하나가 멀티바이트쌍으로 먹혀 "문자열에 종결자가 없습니다"가 났고, 뒤따르는 `}`/`)`
//! 누락 오류 4개가 캐스케이드로 쏟아졌다. **파일 내용은 정상이었고 인코딩만** 문제였다 —
//! 그래서 사람의 주의력이 아니라 테스트가 지킨다.
//!
//! ## 규칙 (윈도우 최적화)
//! | 파일 | 인코딩 | 줄바꿈 | 이유 |
//! |---|---|---|---|
//! | PowerShell(`*.ps1/.psm1/.psd1`) | **UTF-8 with BOM** | CRLF | 5.1은 BOM이 없으면 ANSI로 읽는다 |
//! | `.bat`/`.cmd` | **ASCII만** | CRLF | `cmd.exe`는 OEM 코드페이지 — 한글은 `.ps1`로 |
//! | 그 외 텍스트 | UTF-8 **without** BOM | **CRLF**(윈도우) | rustc/cargo/깃은 항상 UTF-8 — BOM은 노이즈 |
//!
//! `.editorconfig`가 에디터의 **저장**을, `.gitattributes`가 **줄바꿈**을 맡고, 여기서는
//! 결과를 검사한다. 이 파일은 `cargo test -p light-note-core`에 포함되고 그 명령이
//! `run-windows.ps1`의 가장 먼저 도는 단계라, 윈도우에서도 리눅스에서도 같은 규칙이 강제된다.
//!
//! ## 한계 (정직하게)
//! 워크스페이스 루트를 못 찾으면(다른 저장소에 vendoring된 사본) 검사를 **건너뛴다** —
//! 남의 트리를 우리 규칙으로 판정하면 안 된다. 새 확장자를 쓰기 시작하면
//! [`is_text_file`]에 넣어야 검사 대상이 된다.

use std::path::{Path, PathBuf};

/// UTF-8 BOM — Windows PowerShell 5.1이 `.ps1`을 UTF-8로 읽게 하는 **유일한 신호**다.
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// UTF-16 BOM — 어디에도 쓰지 않는다(읽는 쪽이 전부 UTF-8이라 이득이 없다).
const UTF16_BOMS: [[u8; 2]; 2] = [[0xFF, 0xFE], [0xFE, 0xFF]];

/// 워크스페이스 루트 — `[workspace]`를 가진 `Cargo.toml`을 위로 올라가며 찾는다.
fn workspace_root() -> Option<PathBuf> {
    let start = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut cursor = Some(start);
    while let Some(dir) = cursor {
        if let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) {
            if text.contains("[workspace]") {
                return Some(dir.to_path_buf());
            }
        }
        cursor = dir.parent();
    }
    None
}

/// 우리가 지키는 텍스트 파일인가 — 확장자가 없는 관례 파일(`.gitignore` 등)도 포함한다.
fn is_text_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if matches!(name, ".gitignore" | ".gitattributes" | ".editorconfig") {
        return true;
    }
    matches!(
        extension(path).as_deref(),
        Some(
            "rs" | "toml"
                | "md"
                | "ps1"
                | "psm1"
                | "psd1"
                | "ps1xml"
                | "bat"
                | "cmd"
                | "txt"
                | "json"
                | "lock"
        )
    )
}

/// 검사 대상 텍스트 파일 전부(`.git`·`target`은 건너뛴다 — 파일이 수만 개다).
fn text_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect(root, &mut files);
    files.sort();
    files
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            // `.git`(저장소 메타데이터)과 `target`(빌드 산출물)은 우리 소스가 아니다.
            if matches!(name.as_str(), ".git" | "target" | "node_modules") {
                continue;
            }
            collect(&path, files);
        } else if is_text_file(&path) {
            files.push(path);
        }
    }
}

/// 확장자(소문자) — `.PS1`도 `.ps1`로 본다.
fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
}

/// PowerShell이 읽는 파일들 — 5.1이 **BOM 없는 것을 ANSI로** 읽는 부류다.
const POWERSHELL_EXTENSIONS: [&str; 4] = ["ps1", "psm1", "psd1", "ps1xml"];

/// 이 파일을 PowerShell이 읽는가.
fn is_powershell(path: &Path) -> bool {
    extension(path).is_some_and(|ext| POWERSHELL_EXTENSIONS.contains(&ext.as_str()))
}

/// 메시지에 쓸 저장소 기준 상대 경로.
fn shown(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// 파일을 읽는다 — 실패는 이 테스트가 할 일을 못 한다는 뜻이라 패닉이 맞다.
fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// 순회가 실제로 저장소를 봤는가 — 이게 없으면 루트를 못 찾았을 때 아래 테스트들이
/// **조용히 통과**한다(가짜 안심).
#[test]
fn the_walk_actually_finds_the_repository() {
    let Some(root) = workspace_root() else {
        return;
    };
    let files = text_files(&root);
    for anchor in [
        "Cargo.toml",
        "Cargo.lock",
        "run-windows.ps1",
        "crates/light-note-core/src/lib.rs",
    ] {
        let wanted = root.join(anchor);
        assert!(
            files.iter().any(|path| path == &wanted),
            "인코딩 순회가 {anchor}를 찾지 못했다 — 테스트가 가짜로 통과할 뻔했다"
        );
    }
}

#[test]
fn every_text_file_is_utf8_without_utf16_markers() {
    let Some(root) = workspace_root() else {
        return;
    };
    for path in text_files(&root) {
        let name = shown(&root, &path);
        let bytes = read(&path);

        assert!(
            !UTF16_BOMS.iter().any(|bom| bytes.starts_with(bom)),
            "{name}: UTF-16 BOM이 있다. rustc/cargo/깃/PowerShell은 전부 UTF-8을 읽는다 — \
             'UTF-8'로 다시 저장하라"
        );
        assert!(
            std::str::from_utf8(&bytes).is_ok(),
            "{name}: UTF-8이 아니다(ANSI/CP949로 저장된 듯). 한글이 깨질 뿐 아니라 \
             Windows PowerShell 5.1은 파싱 자체가 죽는다 — 'UTF-8'로 다시 저장하라"
        );
        // ASCII만 있는 UTF-16은 UTF-8로도 읽히지만 NUL이 남는다 — 그 흔적을 잡는다.
        assert!(
            !bytes.contains(&0),
            "{name}: NUL 바이트가 있다(UTF-16으로 저장된 듯) — 'UTF-8'로 다시 저장하라"
        );
    }
}

#[test]
fn powershell_files_start_with_a_utf8_bom() {
    let Some(root) = workspace_root() else {
        return;
    };
    for path in text_files(&root) {
        if !is_powershell(&path) {
            continue;
        }
        let name = shown(&root, &path);
        let bytes = read(&path);
        assert!(
            bytes.starts_with(&UTF8_BOM),
            "{name}: UTF-8 BOM이 없다. Windows PowerShell **5.1**은 BOM 없는 .ps1을 \
             ANSI(CP949)로 읽어 한글을 깨뜨리고 문자열 종결자까지 먹는다(파싱 오류). \
             고치기: VS Code 우하단 인코딩을 'UTF-8 with BOM'으로 저장하거나 \
             [IO.File]::WriteAllText($경로, $텍스트, (New-Object Text.UTF8Encoding($true)))"
        );
    }
}

#[test]
fn other_text_files_do_not_start_with_a_bom() {
    let Some(root) = workspace_root() else {
        return;
    };
    for path in text_files(&root) {
        if is_powershell(&path) {
            continue; // PowerShell 파일은 BOM이 **필수**다(바로 위 테스트).
        }
        let name = shown(&root, &path);
        let bytes = read(&path);
        assert!(
            !bytes.starts_with(&UTF8_BOM),
            "{name}: BOM이 있다. rustc/cargo/깃은 항상 UTF-8을 알아서 읽으므로 BOM은 \
             이득 없이 diff만 흔들고 include_str!에도 그대로 들어간다 — BOM 없이 저장하라"
        );
    }
}

/// PowerShell 파일의 줄바꿈은 **윈도우에서** CRLF여야 한다.
///
/// 리눅스 체크아웃은 LF일 수 있고(5.1도 LF는 읽는다) 그걸 실패로 만들면 안 된다.
/// 윈도우 워크트리는 `.gitattributes`의 `eol=crlf`가 CRLF를 보장한다.
///
/// **모든 텍스트 파일**에 같은 규칙을 건다: `.editorconfig`(`end_of_line = crlf`)와
/// `.gitattributes`가 그렇게 맞추기로 했고, 새로 만든 파일이 LF로 저장되는 드리프트를
/// 여기서 잡는다(실제로 이 테스트가 그런 파일 두 개를 잡아냈다).
#[cfg(windows)]
#[test]
fn text_files_use_crlf_on_windows() {
    let Some(root) = workspace_root() else {
        return;
    };
    for path in text_files(&root) {
        let name = shown(&root, &path);
        let bytes = read(&path);
        let bare_lf = bytes
            .iter()
            .enumerate()
            .filter(|(index, byte)| **byte == b'\n' && (*index == 0 || bytes[index - 1] != b'\r'))
            .count();
        assert_eq!(
            bare_lf, 0,
            "{name}: CR 없이 오는 LF가 {bare_lf}개 있다 — 윈도우 워크트리는 CRLF다(.editorconfig)"
        );
    }
}

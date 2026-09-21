//! light-note — **윈도우 11 전용** 드로잉패드 필기 앱.
//!
//! - 화면(elm-magic)과 엔진(필기/PDF)은 `light-note-core`에 있고, 리눅스에서도
//!   `cargo test -p light-note-core`로 검증된다.
//! - 이 크레이트는 그 코어를 WinUI 3(`windows-reactor`)에 붙이는 **얇은 호스트**다.
//!   윈도우에서 `cargo run -p light-note-win --release`.

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod files;
#[cfg(windows)]
mod surface;

fn main() {
    #[cfg(windows)]
    app::run();

    #[cfg(not(windows))]
    {
        println!("light-note는 윈도우 11(WinUI 3) 전용입니다.");
        println!("리눅스/맥에서는 `cargo test -p light-note-core`로 코어 계약을 검증하세요.");
    }
}

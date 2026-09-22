//! light-note-gui — 윈도우 11 전용 필기 앱의 진입점.
//!
//! 파이프라인(①입력 ②도구 ③캔버스 ④렌더)은 라이브러리 쪽에 있다([`light_note_gui`]).
//! 이 파일은 **창을 띄우는 일만** 한다 — 그래서 테스트는 WinUI 없이 파이프라인을 돌린다.

fn main() {
    light_note_gui::app::run();
}

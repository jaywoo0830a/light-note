//! light-note-gui — **윈도우 11 전용** 필기 앱. 하나의 크레이트, 하나의 파이프라인(4단계).
//!
//! ```text
//! ① HardwareInput  [UI]   WinUI 포인터 + Win32 WM_POINTER → Sample   (input, digitizer)
//! ② CanvasTool     [UI]   Sample → 획 + 라이브 기하         (tool)
//! ③ Canvas         [UI]   상태만: 획 목록 · 구운 접두사 · 안 구운 꼬리  (canvas)
//! ④ Render         [UI]   Canvas → WinUI 트리(키 diff)      (render)
//!                  [워커] Canvas 스냅샷 → 픽스맵+PNG          ← 페이지 크기 작업은 여기서만
//! ```
//!
//! **디지타이저(펜)만 필기한다**: 드로잉 패드 친화 필기 앱이라 손가락·마우스는 잉크를
//! 만들지 않는다([`tool::CanvasTool::accepts`]). 장치 판정은 Win32 `WM_POINTER`를 읽는
//! [`digitizer`]가 하고(필압·틸트도 거기서 온다), WinUI는 장치를 알려주지 않는다.
//!
//! 규칙은 셋뿐이고, 그 셋이 이 크레이트의 계약이다:
//!
//! - **R1 — UI 단계는 페이지 크기에 비례하는 일을 하지 않는다.**
//!   표본 하나 = O(1)이고, ④-UI가 만드는 도형 수는 [`canvas::LIVE_SHAPE_BUDGET`]으로 묶인다.
//! - **R2 — 픽셀 작업은 ④-워커에서만, 그리고 기다리지 않는다.**
//!   요청은 **최신 하나**만 의미가 있다(latest-wins). UI 스레드는 결과를 기다리며 멈추지 않는다.
//! - **R3 — 화면은 언제나 "구운 접두사 + 안 구운 꼬리"로 완전하다.**
//!   접두사가 낡아도 꼬리가 그 획을 그리므로 화면이 비지 않는다 →
//!   **타이머도, 세대 장부도, 안전망 스레드도 필요 없다.**
//!
//! ## 화면에 올라가는 두 겹
//! - **베이스**(③의 [`canvas::Base`]) — 픽스맵 두 장. 새 그림은 **화면 밖 자리**에 들어가고,
//!   디코드 완료 신호(`ImageOpened`)에 좌표만 맞바꾼다. 어댑터에는 `Image` 소스를 원자적으로
//!   바꾸는 방법이 없으므로, 이 규칙이 "빈 프레임"을 **구조적으로** 막는다.
//! - **꼬리**(③의 라이브 도형) — 아직 안 구운 확정 획 + 진행 중 획. ④-UI가 선분과 둥근 캡으로
//!   그리는데, **래스터와 같은 도형 결정**(②의 [`shape::ink_shape`])을 쓴다 →
//!   획을 확정하는 순간에도 잉크의 모양과 색이 바뀌지 않는다.
//!
//! ## 파이프라인 밖
//! - PDF 읽기([`pdf`]), 내보내기([`export`]), 파일 대화상자([`files`])는 파이프라인과
//!   직교한다. 대화상자는 UI 스레드, 읽기/쓰기/렌더는 워커다.
//! - 셸([`app`])이 4단계를 **순서대로 부르는 유일한 곳**이다(elmos `Component`).

pub mod app;
pub mod canvas;
pub mod digitizer;
pub mod doc;
pub mod export;
pub mod files;
pub mod geom;
pub mod ink;
pub mod input;
pub mod parts;
pub mod pdf;
pub mod render;
pub mod shape;
pub mod style;
pub mod tool;
pub mod ui;

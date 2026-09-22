//! ① HardwareInput — WinUI 포인터 이벤트를 **모델이 아는 값**으로 바꾼다.
//!
//! 이 단계가 아는 것은 딱 셋이다: 어떤 위상인가, 표면 어디인가, 지금이 언제인가.
//! 도구도 잉크도 페이지도 모른다 — 그래서 배율 변환 하나만 책임진다.
//!
//! ```text
//! on_pointer_pressed / moved / released / capture_lost / canceled   (④-UI가 붙인다)
//!        │
//!        ▼  Phase + DIP 좌표
//!   sample(phase, x, y, scale) → Sample { phase, pt }               (여기)
//!        │
//!        ▼  메시지로 UI 스레드에 (② CanvasTool이 받는다)
//! ```
//!
//! 취소는 **두 경로로 온다**: 포인터 캡처 상실(`capture_lost`)과 시스템 취소(`canceled`).
//! 둘 다 [`Phase::Canceled`] 하나로 모은다 — 그래야 ②가 진행 중인 획을 확실히 되돌린다.

use std::rc::Rc;
use std::time::Instant;

use crate::geom::{Pt, Scale};

/// 포인터 위상 — ①과 ②의 계약.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pressed,
    Moved,
    Released,
    Canceled,
}

impl Phase {
    /// 제스처가 끝나는 위상인가(②가 커밋하거나 버린다).
    pub const fn is_end(self) -> bool {
        matches!(self, Phase::Released | Phase::Canceled)
    }
}

/// 정규화된 표본 — 페이지 좌표(pt)와 시각.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub phase: Phase,
    pub at: Pt,
    pub now: Instant,
}

/// WinUI DIP 좌표 → 표본(pt). **변환은 이 함수 하나뿐이다.**
pub fn sample(phase: Phase, x: f64, y: f64, scale: Scale) -> Sample {
    Sample {
        phase,
        at: scale.to_pt(x as f32, y as f32),
        now: Instant::now(),
    }
}

/// 표면이 호스트에 포인터를 알리는 통로 — 자기 메시지 큐로 옮긴다.
///
/// `fn`이 아니라 `Rc<dyn Fn>`인 이유: ④-UI의 빌더가 **호스트의 `LocalSender`를 캡처**한다
/// (elm `<Raw>`는 슬롯·props를 볼 수 없으므로 캡처가 유일한 통로다 — 예제 08).
pub type InputSink = Rc<dyn Fn(Phase, f64, f64)>;

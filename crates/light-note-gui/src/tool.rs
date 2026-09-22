//! ② CanvasTool — ①의 입력을 **잉크 결정**으로 바꾸는 유일한 곳.
//!
//! 도구가 정하는 것: **무엇이 잉크가 되는가**([`CanvasTool::accepts`] — 디지타이저만),
//! 어떤 스타일로 긋는가, 압력(**하드웨어 필압이 있으면 그것, 없으면 속도**), 그리고
//! **드래그가 문서에 언제 반영되는가**.
//!
//! - 그리는 중인 획은 **문서에 들어가지 않는다** — ②가 들고 있다가 손을 떼는 순간
//!   [`Doc::commit_stroke`]로 확정한다. 그래서 페이지에는 언제나 **확정된 획만** 있다.
//! - 지우개는 드래그 동안 페이지를 직접 고친다(즉시 사라지는 게 보여야 한다) —
//!   대신 지운 획들을 모아 두었다가 손을 뗄 때 **편집 하나**로 커밋한다.
//!
//! UI 스레드 비용: **표본 하나당 O(1)**. 페이지 크기에 비례하는 일은 여기서 하지 않는다.

use std::time::Instant;

use crate::doc::{Doc, Edit};
use crate::geom::{Pt, Scale};
use crate::ink::{pressure_from_speed, InkPoint, Stroke, Style, Tool, MIN_SAMPLE_DISTANCE_PT};
use crate::input::Device;
use crate::shape::LiveInk;

/// 진행 중인 제스처 — 확정 전의 편집 하나.
#[derive(Clone, Debug)]
pub enum Gesture {
    /// 그리는 중 — 페이지에 아직 없다.
    Draw { stroke: Stroke },
    /// 지우는 중 — 페이지에서 이미 빠졌고, 되돌릴 재료를 모으고 있다.
    Erase {
        page: usize,
        removed: Vec<(usize, Stroke)>,
    },
}

/// 상태바가 읽는 도구 상태 — UI와 테스트가 같은 값을 본다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolState {
    pub tool: Tool,
    pub width_pt: f32,
    pub eraser_radius_pt: f32,
}

pub struct CanvasTool {
    state: ToolState,
    style: Style,
    gesture: Option<Gesture>,
    /// 직전 표본(속도 → 압력).
    last: Option<(Pt, Instant)>,
}

impl Default for CanvasTool {
    fn default() -> Self {
        let tool = Tool::Pen;
        Self {
            state: ToolState {
                tool,
                width_pt: Style::for_tool(tool).width_pt,
                eraser_radius_pt: Self::ERASER_RADIUS_PT,
            },
            style: Style::for_tool(tool),
            gesture: None,
            last: None,
        }
    }
}

impl CanvasTool {
    /// 지우개 반경(pt) — 드래그 지점에서 이만큼 안의 획을 지운다.
    pub const ERASER_RADIUS_PT: f32 = 14.0;
    /// 굵기 한 단계.
    pub const WIDTH_STEP: f32 = 1.25;

    /// **필기 정책 — 디지타이저(펜)만 잉크가 된다.**
    ///
    /// 이 앱은 드로잉 패드 친화 필기 앱이라 손가락·마우스는 **아무것도 만들지 않는다**
    /// (화면을 만질 수는 있어도 획은 안 생긴다). 장치 판정은 ①이 한다([`Device`]) —
    /// ②는 그 판정을 **정책으로** 적용하는 곳이다.
    pub const fn accepts(device: Device) -> bool {
        matches!(device, Device::Pen)
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> ToolState {
        self.state
    }

    pub fn tool(&self) -> Tool {
        self.state.tool
    }

    pub fn style(&self) -> Style {
        self.style
    }

    /// 도구를 바꾼다 — 스타일도 그 도구의 기본값으로 바뀐다.
    pub fn select(&mut self, tool: Tool) {
        self.state.tool = tool;
        self.style = Style::for_tool(tool);
        self.state.width_pt = self.style.width_pt;
    }

    /// 굵기 한 단계 ±.
    pub fn resize(&mut self, wider: bool) {
        let factor = if wider {
            Self::WIDTH_STEP
        } else {
            1.0 / Self::WIDTH_STEP
        };
        self.style = Style::new(self.style.color, self.style.width_pt * factor);
        self.state.width_pt = self.style.width_pt;
    }

    pub fn is_active(&self) -> bool {
        self.gesture.is_some()
    }

    /// 그리는 중인 획 — ③이 라이브 꼬리에 넣는다.
    pub fn drawing(&self) -> Option<&Stroke> {
        match self.gesture.as_ref()? {
            Gesture::Draw { stroke } => Some(stroke),
            Gesture::Erase { .. } => None,
        }
    }

    pub fn hint(&self) -> &'static str {
        Style::hint(self.state.tool)
    }

    // ── 제스처 ──────────────────────────────────────────────────────

    /// 손을 댔다 — 새 획을 시작하거나 지우개 드래그를 시작한다.
    ///
    /// 이전 제스처가 남아 있으면 먼저 버린다(포인터 캡처가 유실됐던 경우).
    /// 하드웨어 필압이 없으면([`None`]) 첫 점은 온전한 폭으로 시작한다.
    pub fn press(&mut self, doc: &mut Doc, at: Pt, now: Instant) {
        self.press_with(doc, at, now, None);
    }

    /// 손을 댔다 — **하드웨어 필압을 들고 오는 경로**(디지타이저).
    ///
    /// `pressure`가 `Some`이면 그것이 이 획의 시작 굵기다: 필압은 **장치가 아는 사실**이고
    /// 속도 추정은 대체값이기 때문이다.
    pub fn press_with(&mut self, doc: &mut Doc, at: Pt, now: Instant, pressure: Option<f32>) {
        self.cancel(doc);
        self.last = Some((at, now));
        let page = doc.active_index();
        if self.state.tool.is_eraser() {
            let removed = doc
                .page_mut(page)
                .map(|page| page.erase_at(at, self.state.eraser_radius_pt))
                .unwrap_or_default();
            self.gesture = Some(Gesture::Erase { page, removed });
            return;
        }
        let first = match pressure {
            Some(pressure) => InkPoint::new(at, pressure),
            None => InkPoint::at(at),
        };
        self.gesture = Some(Gesture::Draw {
            stroke: Stroke::new(self.state.tool, self.style, first),
        });
    }

    /// 손을 움직였다. **무언가 바뀌었으면 `true`**(③이 라이브를 다시 계산한다).
    pub fn drag(&mut self, doc: &mut Doc, at: Pt, now: Instant) -> bool {
        self.drag_with(doc, at, now, None)
    }

    /// 손을 움직였다 — **하드웨어 필압을 들고 오는 경로**(디지타이저).
    pub fn drag_with(
        &mut self,
        doc: &mut Doc,
        at: Pt,
        now: Instant,
        pressure: Option<f32>,
    ) -> bool {
        let pressure = self.pressure(at, now, pressure);
        match self.gesture.as_mut() {
            Some(Gesture::Draw { stroke }) => stroke.push(InkPoint::new(at, pressure)),
            Some(Gesture::Erase { page, removed }) => {
                let page = *page;
                let radius = self.state.eraser_radius_pt;
                let Some(target) = doc.page_mut(page) else {
                    return false;
                };
                let just_removed = target.erase_at(at, radius);
                if just_removed.is_empty() {
                    return false;
                }
                removed.extend(just_removed);
                true
            }
            None => false,
        }
    }

    /// 손을 뗐다 — 편집 하나로 커밋한다(Undo 한 번으로 되돌아간다).
    pub fn lift(&mut self, doc: &mut Doc) -> Option<Edit> {
        self.last = None;
        match self.gesture.take()? {
            Gesture::Draw { stroke } => {
                if stroke.is_empty() {
                    return None;
                }
                let page = doc.active_index();
                let index = doc.commit_stroke(page, stroke.clone());
                Some(Edit::AddStroke {
                    page,
                    index,
                    stroke,
                })
            }
            Gesture::Erase { page, removed } => {
                if doc.commit_erasure(page, removed.clone()) {
                    Some(Edit::RemoveStrokes { page, removed })
                } else {
                    None
                }
            }
        }
    }

    /// 제스처를 버린다 — 그리는 중이던 획은 **어디에도 남지 않고**, 지운 것은 되돌린다.
    pub fn cancel(&mut self, doc: &mut Doc) -> bool {
        self.last = None;
        match self.gesture.take() {
            None => false,
            Some(Gesture::Draw { .. }) => true,
            Some(Gesture::Erase { page, removed }) => {
                doc.restore_erased(page, removed);
                true
            }
        }
    }

    /// 진행 중인 획의 라이브 도형 — ③이 꼬리에 넣는다.
    pub fn live(&self, scale: Scale) -> Option<LiveInk> {
        self.drawing()
            .map(|stroke| crate::shape::live_ink(stroke, scale))
    }

    /// 압력의 **출처는 하나**다: 하드웨어가 보고했으면 그것(디지타이저), 아니면 속도 추정.
    ///
    /// 어느 쪽이든 `last`는 갱신한다 — 펜을 떼고 마우스로 이어 그어도 속도 계산이 튀지 않게.
    fn pressure(&mut self, at: Pt, now: Instant, hardware: Option<f32>) -> f32 {
        let previous = self.last.replace((at, now));
        if let Some(pressure) = hardware {
            return pressure.clamp(0.0, 1.0);
        }
        let Some((previous, then)) = previous else {
            return InkPoint::DEFAULT_PRESSURE;
        };
        let millis = now.saturating_duration_since(then).as_secs_f32() * 1000.0;
        let distance = previous.distance(at);
        if millis <= f32::EPSILON || distance < MIN_SAMPLE_DISTANCE_PT {
            return InkPoint::DEFAULT_PRESSURE;
        }
        pressure_from_speed(distance / millis)
    }
}

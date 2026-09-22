//! ③ Canvas — **상태 하나**. 픽셀은 여기서 만들지 않는다.
//!
//! 화면은 언제나 두 부분의 합이다:
//!
//! ```text
//! 구운 접두사(Base) = 확정 획 [0 .. baked_count)          → 픽스맵 두 장 중 한 장(④-워커가 굽는다)
//! 안 구운 꼬리      = 확정 획 [baked_count .. ) + 진행 중 획 → 라이브 도형(④-UI가 그린다)
//! ```
//!
//! 규칙 셋:
//! - **R1** 이 파일은 순수 상태다 — 페이지 크기에 비례하는 일(래스터·인코딩)을 하지 않는다.
//! - **꼬리는 급하지 않다.** 접두사가 낡아도 화면은 꼬리로 완전하다 → 타이머가 필요 없다.
//! - **베이크는 예산이 정한다**([`LIVE_SHAPE_BUDGET`]): 라이브 도형이 예산을 넘거나 접두사가
//!   무효가 됐을 때(지우개·되돌리기·페이지·줌) **한 번** 요청한다.

use std::rc::Rc;
use std::sync::Arc;

use crate::doc::Doc;
use crate::geom::{Scale, Size};
use crate::ink::Stroke;
use crate::shape::{blank_png, has_ink, live_ink, LiveInk};
use crate::ui::Frame;

/// 라이브 도형 예산 — 이보다 많아지면 베이크를 요청한다(**UI 도형 수의 상한**).
///
/// 예산 안에서는 픽셀 작업이 **전혀 없다**: 획을 그어도 워커를 부르지 않는다.
pub const LIVE_SHAPE_BUDGET: usize = 900;

/// 한 자리의 그림 — **자리마다 자기 출처를 기억한다**.
///
/// 왜 필요한가: 페이지를 넘기면 화면에는 A페이지 그림이 보이는데 화면 밖 자리에는
/// B페이지 그림이 들어 있을 수 있다. 한 벌의 메타데이터로는 이 상황을 표현할 수 없어서
/// 꼬리(안 구운 획) 계산이 어긋난다.
#[derive(Clone, Debug, Default, PartialEq)]
struct Slot {
    png: Option<Arc<[u8]>>,
    page: Option<usize>,
    scale: f32,
    /// 이 그림이 반영한 확정 획 수.
    count: usize,
}

impl Slot {
    fn matches(&self, page: usize, scale: Scale) -> bool {
        self.page == Some(page) && (self.scale - scale.get()).abs() < f32::EPSILON
    }
}

/// 구운 접두사 — 화면에 올린 PNG **두 장**.
///
/// 두 장인 이유: 어댑터에는 `Image`의 소스를 **원자적으로** 바꾸는 방법이 없다(소스를
/// 바꾸면 디코드가 끝날 때까지 비어 보인다). 그래서 새 그림은 **화면 밖 자리**에 넣고
/// (`stage`), 디코드 완료 신호(`ImageOpened`)에 좌표만 맞바꾼다(`promote`) — 화면에
/// 보이는 자리의 소스는 **한 번도 비워지지 않는다.**
#[derive(Clone, Debug, PartialEq)]
pub struct Base {
    slots: [Slot; 2],
    front: usize,
    /// 화면 밖 자리에 **새 그림이 들어가 있고** 그 신호를 기다리는 중인가.
    ///
    /// 이 표시가 없으면 늦게 도착한 옛 그림의 신호에 화면이 뒤로 갈 수 있다.
    staged: bool,
}

impl Default for Base {
    fn default() -> Self {
        Self::blank()
    }
}

impl Base {
    /// 아직 아무것도 굽지 않은 상태(빈 페이지).
    pub fn blank() -> Self {
        Self {
            slots: [Slot::default(), Slot::default()],
            front: 0,
            staged: false,
        }
    }

    /// 화면에 보이는 자리.
    pub fn visible_index(&self) -> usize {
        self.front
    }

    /// 새 그림을 넣을 자리(화면 밖).
    pub fn hidden_index(&self) -> usize {
        1 - self.front
    }

    pub fn png(&self, index: usize) -> Option<&Arc<[u8]>> {
        self.slots[index % 2].png.as_ref()
    }

    pub fn visible_png(&self) -> Option<&Arc<[u8]>> {
        self.slots[self.front].png.as_ref()
    }

    /// 화면에 보이는 그림이 반영한 확정 획 수 = `baked_count`. **꼬리는 여기서 시작한다.**
    pub fn count(&self) -> usize {
        self.slots[self.front].count
    }

    /// 화면에 보이는 그림이 어느 페이지의 것인가.
    pub fn visible_page(&self) -> Option<usize> {
        self.slots[self.front].page
    }

    /// 이 그림을 그대로 쓸 수 있는가(같은 페이지·같은 배율의 그림이면).
    pub fn is_valid_for(&self, page: usize, scale: Scale) -> bool {
        self.slots[self.front].matches(page, scale)
    }

    pub fn has_staged(&self) -> bool {
        self.staged
    }

    /// 새 그림을 **화면 밖 자리**에 넣는다.
    ///
    /// 같은 그림이면 기다릴 이유가 없다 — 디코드 신호 없이 바로 올린다.
    /// 돌려주는 값은 "디코드 신호를 기다려야 하는가".
    pub fn stage(
        &mut self,
        png: Option<Arc<[u8]>>,
        page: usize,
        scale: Scale,
        count: usize,
    ) -> bool {
        // 잉크가 없는 페이지는 소스를 비울 수 없으니 **빈 그림**으로 표현한다.
        let png = png.unwrap_or_else(blank_png);
        if let Some(visible) = self.visible_png() {
            if Arc::ptr_eq(visible, &png) || **visible == *png {
                let visible = self.front;
                self.slots[visible].count = count;
                return false;
            }
        }
        let hidden = self.hidden_index();
        self.slots[hidden] = Slot {
            png: Some(png),
            page: Some(page),
            scale: scale.get(),
            count,
        };
        self.staged = true;
        true
    }

    /// 디코드 완료 — 좌표만 맞바꾼다(동기적, 디코드 없음).
    ///
    /// 다른 페이지의 그림이면 **버린다**: 그 페이지의 그림은 지금 화면에 맞지 않는다.
    /// 버려도 손해가 없다 — 다음 베이크가 같은 자리에 다시 넣는다.
    pub fn promote(&mut self, page: usize) -> bool {
        if !self.staged {
            return false;
        }
        let hidden = self.hidden_index();
        if self.slots[hidden].page != Some(page) || self.slots[hidden].png.is_none() {
            return false;
        }
        self.front = hidden;
        self.staged = false;
        true
    }

    /// 대기 중인 그림을 버린다(페이지 이동 등) — 늦게 온 신호는 무시된다.
    pub fn drop_staged(&mut self) -> bool {
        let had = self.staged;
        self.staged = false;
        had
    }
}

/// ④-워커로 넘기는 스냅샷 — **최신 하나만** 의미가 있다(latest-wins).
///
/// 소유권을 넘긴다: 워커가 이 목록을 마음대로 읽고, UI 스레드는 그 뒤로 손대지 않는다.
#[derive(Clone, Debug)]
pub struct BakeRequest {
    pub id: u64,
    pub page: usize,
    pub size: Size,
    pub scale: Scale,
    pub count: usize,
    pub strokes: Vec<Stroke>,
}

impl BakeRequest {
    /// 구울 잉크가 없으면 워커를 부르지 않는다(빈 페이지에 1.13Mpx를 만들 이유가 없다).
    pub fn is_empty(&self) -> bool {
        !has_ink(&self.strokes)
    }

    /// 잉크 없는 요청의 결과 — 워커 없이 즉시 만들 수 있다.
    pub fn empty_result(&self) -> BakeResult {
        BakeResult {
            id: self.id,
            page: self.page,
            scale: self.scale,
            count: self.count,
            png: None,
        }
    }
}

/// ④-워커가 돌려주는 결과 — **`Send`다**(메시지로 UI 스레드에 온다).
#[derive(Clone, Debug)]
pub struct BakeResult {
    pub id: u64,
    pub page: usize,
    pub scale: Scale,
    pub count: usize,
    pub png: Option<Arc<[u8]>>,
}

/// ③ 캔버스 — 문서 + 구운 접두사 + 꼬리. **순수 상태**이고 픽셀은 만들지 않는다.
pub struct Canvas {
    doc: Doc,
    base: Base,
    scale: Scale,
    zoom: f32,
    /// 구운 접두사 뒤의 확정 획들 — **`Rc`로 공유**한다(프레임마다 복사하지 않는다).
    tail: Rc<[LiveInk]>,
    /// `tail`이 만들어진 시점의 `baked_count`.
    tail_from: usize,
    /// `tail`이 만들어진 시점의 **확정 획 수** — 커밋 하나로도 꼬리가 늘어난다.
    tail_len: usize,
    tail_shapes: usize,
    /// 꼬리를 다시 만들어야 하는가(지우개·되돌리기·페이지 이동 — 획 **집합**이 바뀌었다).
    tail_dirty: bool,
    /// 진행 중 획 — 표본마다 **이 하나만** 다시 만든다.
    drawing: Option<Rc<LiveInk>>,
    /// 접두사가 낡았다(지우개·되돌리기·페이지 비우기) → 다시 구워야 한다.
    stale: bool,
    /// 마지막 베이크 요청 id — 낡은 응답을 버리는 기준.
    last_request: Option<u64>,
    next_id: u64,
}

impl Canvas {
    pub const MIN_ZOOM: f32 = 25.0;
    pub const MAX_ZOOM: f32 = 400.0;
    pub const ZOOM_STEP: f32 = 1.25;

    pub fn new(size: Size) -> Self {
        Self {
            doc: Doc::blank(size),
            base: Base::blank(),
            scale: Scale::from_zoom(100.0),
            zoom: 100.0,
            tail: Rc::from(Vec::new()),
            tail_from: 0,
            tail_len: 0,
            tail_shapes: 0,
            tail_dirty: true,
            drawing: None,
            stale: true,
            last_request: None,
            next_id: 0,
        }
    }

    /// PDF 페이지들로 시작한다 — 배경은 ④가 깔고, 여기는 크기만 안다.
    pub fn from_pdf(
        sizes: impl IntoIterator<Item = Size>,
        title: impl Into<String>,
        zoom: f32,
    ) -> Self {
        let mut canvas = Self::new(Size::A4);
        canvas.doc = Doc::from_pdf(sizes, title);
        canvas.zoom = zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        canvas.scale = Scale::from_zoom(canvas.zoom);
        canvas
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    /// ②의 커밋과 편집 명령이 문서를 고칠 때 쓴다.
    pub fn doc_mut(&mut self) -> &mut Doc {
        &mut self.doc
    }

    pub fn scale(&self) -> Scale {
        self.scale
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// 배율을 바꾼다. 바뀌면 `true` — 접두사는 새 배율로 다시 구워야 한다.
    pub fn set_zoom(&mut self, zoom: f32) -> bool {
        let zoom = zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
        if (zoom - self.zoom).abs() < f32::EPSILON {
            return false;
        }
        self.zoom = zoom;
        self.scale = Scale::from_zoom(zoom);
        self.drawing = None;
        true
    }

    /// 배율을 한 단계 ±.
    pub fn step_zoom(&mut self, closer: bool) -> bool {
        let factor = if closer {
            Self::ZOOM_STEP
        } else {
            1.0 / Self::ZOOM_STEP
        };
        self.set_zoom(self.zoom * factor)
    }

    /// 지금 페이지의 크기(pt) — ④가 표면 크기를 정할 때 쓴다.
    pub fn page_size(&self) -> Size {
        self.doc.active_page().size()
    }

    pub fn baked_count(&self) -> usize {
        self.base.count()
    }

    pub fn base(&self) -> &Base {
        &self.base
    }

    /// 지금 화면에 그릴 라이브 도형 수(꼬리 + 진행 중 획).
    pub fn live_shapes(&self) -> usize {
        self.tail_shapes
            + self
                .drawing
                .as_ref()
                .map(|ink| ink.shape_count())
                .unwrap_or(0)
    }

    pub fn tail(&self) -> &[LiveInk] {
        &self.tail
    }

    pub fn drawing(&self) -> Option<&LiveInk> {
        self.drawing.as_deref()
    }

    /// ④-UI가 그릴 재료 — `Rc` 복사만 있으므로 **프레임마다 만들어도 싸다**.
    pub fn frame(&self) -> Frame {
        Frame {
            base: self.base.clone(),
            tail: Rc::clone(&self.tail),
            drawing: self.drawing.clone(),
            size: self.page_size(),
            scale: self.scale,
            baked: self.base.count(),
            strokes: self.doc.strokes().len(),
        }
    }

    // ── 프레임에 한 번: 꼬리 맞추기 ────────────────────────────────────

    /// ②가 만든 진행 중 획을 넣고 꼬리를 맞춘다. **바뀌면 `true`**(④가 다시 그린다).
    pub fn refresh(&mut self, drawing: Option<&Stroke>) -> bool {
        let mut changed = false;
        let committed = self.doc.strokes();
        let start = self.base.count().min(committed.len());
        // 꼬리는 **접두사 끝**에서 시작해 **지금 확정된 획까지** 이어져야 한다:
        // 커밋 하나(추가)도, 지우개 하나(제거)도 여기를 다시 만든다.
        if self.tail_dirty || self.tail_from != start || self.tail_len != committed.len() {
            let tail: Vec<LiveInk> = committed[start..]
                .iter()
                .map(|stroke| live_ink(stroke, self.scale))
                .filter(|ink| !ink.is_empty())
                .collect();
            self.tail_shapes = tail.iter().map(LiveInk::shape_count).sum();
            self.tail = Rc::from(tail);
            self.tail_from = start;
            self.tail_len = committed.len();
            self.tail_dirty = false;
            changed = true;
        }
        let next = drawing
            .map(|stroke| live_ink(stroke, self.scale))
            .filter(|ink| !ink.is_empty())
            .map(Rc::new);
        let same = match (&next, &self.drawing) {
            (None, None) => true,
            (Some(next), Some(drawing)) => next.as_ref() == drawing.as_ref(),
            _ => false,
        };
        if !same {
            self.drawing = next;
            changed = true;
        }
        changed
    }

    /// 접두사가 화면과 더는 맞지 않는다 — **지우개·되돌리기·다시하기·비우기·PDF 열기**.
    ///
    /// 추가(획 하나 커밋)는 무효가 아니다: 그 획은 꼬리가 그리므로 접두사는 여전히 옳다.
    /// 대신 워커에 가 있던 요청은 버린다(그림이 이미 틀렸을 수 있다).
    pub fn invalidate(&mut self) {
        self.stale = true;
        self.tail_dirty = true; // 획 **집합**이 바뀌었을 수 있다 — 꼬리를 다시 만든다
        self.last_request = None;
    }

    /// 모델을 고치는 편집은 전부 여기로 들어온다 — 접두사 무효화를 잊을 수 없다.
    pub fn edit<R>(&mut self, action: impl FnOnce(&mut Doc) -> R) -> R {
        let result = action(&mut self.doc);
        self.invalidate();
        result
    }

    /// 페이지를 옮긴다. 바뀌면 `true` — 대기 중인 그림은 버린다(다른 페이지의 그림이다).
    pub fn go_to_page(&mut self, index: usize) -> bool {
        if !self.doc.select_page(index) {
            return false;
        }
        self.base.drop_staged();
        self.invalidate();
        true
    }

    pub fn step_page(&mut self, delta: isize) -> bool {
        let next = self.doc.active_index() as isize + delta;
        if next < 0 || next as usize >= self.doc.page_count() {
            return false;
        }
        self.go_to_page(next as usize)
    }

    // ── 베이크: 언제, 무엇을, 어떻게 ───────────────────────────────────

    /// 베이크가 필요한가? `inflight`는 "워커에 요청이 가 있는가"다(**R2: 한 번에 하나**).
    pub fn needs_bake(&self, inflight: bool) -> bool {
        if inflight {
            return false;
        }
        if !self.base.is_valid_for(self.doc.active_index(), self.scale) {
            return true;
        }
        self.stale || self.live_shapes() > LIVE_SHAPE_BUDGET
    }

    /// 지금 페이지 **전체**를 굽는 요청 — ④-워커에 넘길 스냅샷(소유권을 넘긴다).
    pub fn bake_request(&mut self) -> BakeRequest {
        self.next_id += 1;
        self.last_request = Some(self.next_id);
        self.stale = false; // 이 요청이 접두사를 새로 만든다
        BakeRequest {
            id: self.next_id,
            page: self.doc.active_index(),
            size: self.page_size(),
            scale: self.scale,
            count: self.doc.strokes().len(),
            strokes: self.doc.strokes().to_vec(),
        }
    }

    /// 워커 결과를 반영한다.
    ///
    /// - `None` — 낡은 응답이다(더 새 요청이 나갔거나 그 사이 페이지·배율이 바뀌었다).
    /// - `Some(true)` — 새 그림이 화면 밖 자리에 들어갔다. **디코드 신호를 기다린다.**
    /// - `Some(false)` — 같은 그림이라 기다릴 것 없이 이미 반영됐다.
    pub fn accept(&mut self, result: BakeResult) -> Option<bool> {
        if self.last_request != Some(result.id) {
            return None;
        }
        self.last_request = None;
        if result.page != self.doc.active_index()
            || (result.scale.get() - self.scale.get()).abs() > f32::EPSILON
        {
            self.stale = true; // 이 응답은 쓸 수 없다 — 다시 구워야 한다
            return None;
        }
        Some(
            self.base
                .stage(result.png, result.page, result.scale, result.count),
        )
    }

    /// 디코드 완료 — 화면 밖 그림을 앞자리로 올린다(좌표 맞바꾸기, 동기적).
    pub fn promote(&mut self) -> bool {
        self.base.promote(self.doc.active_index())
    }

    /// 워커에 나간 요청이 있는가.
    pub fn has_request(&self) -> bool {
        self.last_request.is_some()
    }
}

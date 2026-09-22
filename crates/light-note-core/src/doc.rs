//! 문서 모델 — 페이지, 스트로크, 진행 중인 획, Undo/Redo.
//!
//! 이 파일이 앱의 **단일 진실**이다. WinUI 호스트도 elm 화면도 여기 있는 것만
//! 읽고, 변경은 전부 `Document`의 메서드를 통해서만 한다(그래야 Undo가 정확하다).
//!
//! 상태 기계 (계약):
//! ```text
//! begin(tool, style, p)  ── 그리기 ──▶  drawing = Some(idx)
//!                        ── 지우개 ──▶  erasing = Some([])
//! extend(p)              진행 중인 획에 샘플 추가 / 반경 안의 스트로크 제거
//! finish()               진행 중인 편집을 Edit 하나로 커밋 (드래그 = Undo 1회)
//! cancel()               진행 중인 편집을 버린다 (히스토리에도 남기지 않는다)
//! ```

use crate::geom::{Bounds, Point, Size};
use crate::history::{Edit, History};
use crate::ink::{InkPoint, Stroke, StrokeStyle, Tool};

/// 지우개 기본 반경(pt) — 드래그 지점에서 이만큼 안의 스트로크를 지운다.
pub const ERASER_RADIUS_PT: f32 = 14.0;

/// 문서의 한 페이지.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    size: Size,
    strokes: Vec<Stroke>,
    /// 배경으로 깔린 PDF 페이지 인덱스(0-based). `None`이면 빈 페이지.
    background: Option<usize>,
}

impl Page {
    /// 빈 페이지.
    pub fn blank(size: Size) -> Self {
        Self {
            size,
            strokes: Vec::new(),
            background: None,
        }
    }

    /// PDF 페이지를 배경으로 깐 페이지 — 그 위에 필기한다.
    pub fn with_pdf_background(size: Size, pdf_page: usize) -> Self {
        Self {
            size,
            strokes: Vec::new(),
            background: Some(pdf_page),
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn set_size(&mut self, size: Size) {
        self.size = size;
    }

    pub fn background(&self) -> Option<usize> {
        self.background
    }

    pub fn set_background(&mut self, pdf_page: Option<usize>) {
        self.background = pdf_page;
    }

    pub fn has_pdf_background(&self) -> bool {
        self.background.is_some()
    }

    pub fn strokes(&self) -> &[Stroke] {
        &self.strokes
    }

    pub fn stroke(&self, index: usize) -> Option<&Stroke> {
        self.strokes.get(index)
    }

    pub fn stroke_count(&self) -> usize {
        self.strokes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty()
    }

    pub fn push_stroke(&mut self, stroke: Stroke) {
        self.strokes.push(stroke);
    }

    pub fn insert_stroke(&mut self, index: usize, stroke: Stroke) {
        let index = index.min(self.strokes.len());
        self.strokes.insert(index, stroke);
    }

    pub fn remove_stroke(&mut self, index: usize) -> Option<Stroke> {
        if index < self.strokes.len() {
            Some(self.strokes.remove(index))
        } else {
            None
        }
    }

    pub fn last_mut(&mut self) -> Option<&mut Stroke> {
        self.strokes.last_mut()
    }

    /// 페이지의 잉크 경계(없으면 `None`) — 더티 렉트/썸네일 계산에 쓴다.
    pub fn ink_bounds(&self) -> Option<Bounds> {
        self.strokes
            .iter()
            .filter_map(|stroke| stroke.bounds())
            .reduce(Bounds::union)
    }

    /// 페이지 전체를 비우고 원래 스트로크를 돌려준다(Undo가 되살린다).
    pub fn take_strokes(&mut self) -> Vec<Stroke> {
        std::mem::take(&mut self.strokes)
    }

    pub fn restore_strokes(&mut self, strokes: Vec<Stroke>) {
        self.strokes = strokes;
    }

    /// 상태바/썸네일용 한 줄 요약.
    pub fn describe(&self) -> String {
        if self.has_pdf_background() {
            format!("PDF 배경 · 스트로크 {}개", self.stroke_count())
        } else {
            format!("빈 페이지 · 스트로크 {}개", self.stroke_count())
        }
    }
}

/// 문서 — 페이지 목록 + 활성 페이지 + 히스토리 + 진행 중인 편집.
///
/// `Clone`이지만 **히스토리까지 복제**된다(테스트가 상태를 통째로 비교할 수 있다).
#[derive(Clone, Debug)]
pub struct Document {
    title: String,
    pages: Vec<Page>,
    active: usize,
    history: History,
    /// 진행 중인 필기: (페이지, 스트로크 인덱스).
    drawing: Option<(usize, usize)>,
    /// 진행 중인 지우기: (페이지, 이번 드래그로 지운 (인덱스, 스트로크)).
    erasing: Option<(usize, Vec<(usize, Stroke)>)>,
    eraser_radius: f32,
    dirty: bool,
}

impl Document {
    /// 빈 A4 한 장으로 시작한다.
    pub fn blank(size: Size) -> Self {
        Self {
            title: "무제".to_string(),
            pages: vec![Page::blank(size)],
            active: 0,
            history: History::default(),
            drawing: None,
            erasing: None,
            eraser_radius: ERASER_RADIUS_PT,
            dirty: false,
        }
    }

    /// PDF 페이지들로 문서를 만든다 — 페이지마다 배경을 깔고 그 위에 필기한다.
    pub fn from_pdf_pages(sizes: impl IntoIterator<Item = Size>, title: impl Into<String>) -> Self {
        let pages: Vec<Page> = sizes
            .into_iter()
            .enumerate()
            .map(|(index, size)| Page::with_pdf_background(size, index))
            .collect();
        let mut document = Self::blank(Size::A4);
        document.title = title.into();
        if !pages.is_empty() {
            document.pages = pages;
        }
        document.dirty = false;
        document
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    pub fn page(&self, index: usize) -> Option<&Page> {
        self.pages.get(index)
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active_page(&self) -> &Page {
        &self.pages[self.active]
    }

    pub fn active_page_mut(&mut self) -> &mut Page {
        &mut self.pages[self.active]
    }

    /// 활성 페이지를 바꾼다. 범위를 벗어나면 `false`(상태는 그대로).
    pub fn select_page(&mut self, index: usize) -> bool {
        if index < self.pages.len() {
            self.active = index;
            true
        } else {
            false
        }
    }

    /// 활성 페이지를 한 칸 옮긴다(경계에서 멈춘다).
    pub fn step_page(&mut self, delta: isize) -> bool {
        let next = (self.active as isize + delta).clamp(0, self.pages.len() as isize - 1) as usize;
        let changed = next != self.active;
        self.active = next;
        changed
    }

    /// 활성 페이지 뒤에 새 페이지를 끼우고 그 페이지로 이동한다.
    pub fn add_page_after_active(&mut self, size: Size) -> usize {
        let index = self.active + 1;
        self.insert_page(index, Page::blank(size))
    }

    /// 페이지를 끼운다(Undo 가능).
    pub fn insert_page(&mut self, index: usize, page: Page) -> usize {
        let index = index.min(self.pages.len());
        self.pages.insert(index, page);
        self.active = index;
        self.history.push(Edit::InsertPage {
            index,
            page: self.pages[index].clone(),
        });
        self.dirty = true;
        index
    }

    /// 페이지를 지운다 — **최소 한 장은 남긴다**.
    pub fn remove_page(&mut self, index: usize) -> bool {
        if self.pages.len() <= 1 || index >= self.pages.len() {
            return false;
        }
        let page = self.pages.remove(index);
        self.active = self.active.min(self.pages.len() - 1);
        self.history.push(Edit::RemovePage { index, page });
        self.dirty = true;
        true
    }

    pub fn total_strokes(&self) -> usize {
        self.pages.iter().map(Page::stroke_count).sum()
    }

    pub fn eraser_radius(&self) -> f32 {
        self.eraser_radius
    }

    pub fn set_eraser_radius(&mut self, radius: f32) {
        self.eraser_radius = radius.clamp(2.0, 80.0);
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn is_drawing(&self) -> bool {
        self.drawing.is_some() || self.erasing.is_some()
    }

    /// 상태바 한 줄 — UI와 테스트가 같은 문자열을 본다.
    pub fn status_line(&self) -> String {
        let active = self.active_page();
        format!(
            "{} / {}페이지 · 스트로크 {}개{}",
            self.active + 1,
            self.pages.len(),
            active.stroke_count(),
            if self.dirty { " · 저장 안 됨" } else { "" }
        )
    }

    // ── 필기 (드래그 = 편집 하나) ─────────────────────────────────

    /// 새 획을 시작한다. 지우개면 "지우기 드래그"를 시작한다.
    pub fn begin(&mut self, tool: Tool, style: StrokeStyle, first: InkPoint) {
        self.cancel(); // 포인터 캡처가 유실됐던 드래그가 남아 있으면 버린다.
        let page = self.active;
        if tool.is_eraser() {
            self.erasing = Some((page, Vec::new()));
            self.erase_at(first.pos);
            return;
        }
        self.pages[page].push_stroke(Stroke::new(tool, style, first));
        self.drawing = Some((page, self.pages[page].stroke_count() - 1));
    }

    /// 진행 중인 획에 샘플을 넣거나, 지우개를 문지른다.
    pub fn extend(&mut self, point: InkPoint) -> bool {
        if let Some((page, index)) = self.drawing {
            if page != self.active {
                return false;
            }
            let Some(stroke) = self.pages[page].strokes.get_mut(index) else {
                return false;
            };
            return stroke.push(point);
        }
        if self.erasing.is_some() {
            return self.erase_at(point.pos) > 0;
        }
        false
    }

    /// 드래그를 끝내고 **편집 하나**로 커밋한다.
    pub fn finish(&mut self) -> Option<Edit> {
        if let Some((page, index)) = self.drawing.take() {
            let stroke = self.pages[page].strokes.get(index)?.clone();
            let edit = Edit::AddStroke { page, index, stroke };
            self.history.push(edit.clone());
            self.dirty = true;
            return Some(edit);
        }
        if let Some((page, removed)) = self.erasing.take() {
            if removed.is_empty() {
                return None;
            }
            let edit = Edit::RemoveStrokes { page, removed };
            self.history.push(edit.clone());
            self.dirty = true;
            return Some(edit);
        }
        None
    }

    /// 진행 중인 편집을 버린다 — 히스토리에도 남기지 않는다. 버렸으면 `true`.
    pub fn cancel(&mut self) -> bool {
        let mut discarded = false;
        if let Some((page, index)) = self.drawing.take() {
            if page == self.active && index + 1 == self.pages[page].stroke_count() {
                self.pages[page].strokes.pop();
                discarded = true;
            }
        }
        if let Some((page, removed)) = self.erasing.take() {
            for (index, stroke) in removed.into_iter().rev() {
                self.pages[page].insert_stroke(index, stroke);
            }
            discarded = true;
        }
        discarded
    }

    /// 진행 중인 획 — WinUI 라이브 레이어가 그릴 대상.
    pub fn live_stroke(&self) -> Option<&Stroke> {
        let (page, index) = self.drawing?;
        if page != self.active {
            return None;
        }
        self.pages[page].stroke(index)
    }

    /// **확정 스트로크만** — 진행 중인 획은 라이브 레이어가 그린다.
    ///
    /// 표면의 정적 레이어(PNG)가 이 목록을 그린다. 진행 중인 획까지 넣으면
    /// 라이브 도형과 겹쳐 **두 번 그려진다**(형광펜에서 특히 티가 난다).
    pub fn committed_strokes(&self) -> &[Stroke] {
        let strokes = self.active_page().strokes();
        if self.drawing.is_some() && !strokes.is_empty() {
            &strokes[..strokes.len() - 1]
        } else {
            strokes
        }
    }

    /// 반경 안의 스트로크를 지운다. 지운 개수를 돌려준다(Undo는 `finish`에서 한 번에).
    pub fn erase_at(&mut self, point: Point) -> usize {
        let Some(page) = self.erasing.as_ref().map(|(page, _)| *page) else {
            return 0;
        };
        let radius = self.eraser_radius;
        let targets: Vec<usize> = self.pages[page]
            .strokes
            .iter()
            .enumerate()
            .filter(|(_, stroke)| stroke.hits_circle(point, radius))
            .map(|(index, _)| index)
            .collect();
        if targets.is_empty() {
            return 0;
        }
        let mut taken: Vec<(usize, Stroke)> = Vec::with_capacity(targets.len());
        for index in targets.iter().rev() {
            if let Some(stroke) = self.pages[page].remove_stroke(*index) {
                taken.push((*index, stroke));
            }
        }
        taken.reverse(); // 인덱스 오름차순으로 기록해야 복구가 정확하다.
        if let Some((_, removed)) = self.erasing.as_mut() {
            removed.extend(taken.iter().cloned());
        }
        taken.len()
    }

    /// 활성 페이지를 비운다(Undo 가능).
    pub fn clear_active_page(&mut self) -> bool {
        let page = self.active;
        if self.pages[page].is_empty() {
            return false;
        }
        let removed = self.pages[page].take_strokes();
        self.history.push(Edit::ClearPage { page, removed });
        self.dirty = true;
        true
    }

    // ── 되돌리기 ────────────────────────────────────────────────

    /// Undo — 진행 중인 드래그가 있으면 먼저 버린다(그림이 남지 않는다).
    pub fn undo(&mut self) -> Option<Edit> {
        self.cancel();
        let edit = self.history.pop_undo()?;
        self.revert(&edit);
        self.dirty = true;
        Some(edit)
    }

    /// Redo.
    pub fn redo(&mut self) -> Option<Edit> {
        self.cancel();
        let edit = self.history.pop_redo()?;
        self.apply(&edit);
        self.dirty = true;
        Some(edit)
    }

    /// 편집을 다시 적용한다(redo).
    fn apply(&mut self, edit: &Edit) {
        match edit {
            Edit::AddStroke {
                page,
                index,
                stroke,
            } => {
                if let Some(page) = self.pages.get_mut(*page) {
                    page.insert_stroke(*index, stroke.clone());
                }
            }
            Edit::RemoveStrokes { page, removed } => {
                if let Some(page) = self.pages.get_mut(*page) {
                    for (index, _) in removed.iter().rev() {
                        page.remove_stroke(*index);
                    }
                }
            }
            Edit::ClearPage { page, .. } => {
                if let Some(page) = self.pages.get_mut(*page) {
                    page.take_strokes();
                }
            }
            Edit::InsertPage { index, page } => {
                let index = (*index).min(self.pages.len());
                self.pages.insert(index, page.clone());
                self.active = index;
            }
            Edit::RemovePage { index, .. } => {
                if self.pages.len() > 1 && *index < self.pages.len() {
                    self.pages.remove(*index);
                    self.active = self.active.min(self.pages.len() - 1);
                }
            }
        }
    }

    /// 편집을 되돌린다(undo).
    fn revert(&mut self, edit: &Edit) {
        match edit {
            Edit::AddStroke { page, index, .. } => {
                if let Some(page) = self.pages.get_mut(*page) {
                    page.remove_stroke(*index);
                }
            }
            Edit::RemoveStrokes { page, removed } => {
                if let Some(page) = self.pages.get_mut(*page) {
                    for (index, stroke) in removed {
                        page.insert_stroke(*index, stroke.clone());
                    }
                }
            }
            Edit::ClearPage { page, removed } => {
                if let Some(page) = self.pages.get_mut(*page) {
                    page.restore_strokes(removed.clone());
                }
            }
            Edit::InsertPage { index, .. } => {
                if self.pages.len() > 1 && *index < self.pages.len() {
                    self.pages.remove(*index);
                    self.active = self.active.min(self.pages.len() - 1);
                }
            }
            Edit::RemovePage { index, page } => {
                let index = (*index).min(self.pages.len());
                self.pages.insert(index, page.clone());
                self.active = index;
            }
        }
    }
}

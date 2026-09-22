//! 문서 모델 — 페이지, 획, Undo/Redo. **앱의 단일 진실**이다.
//!
//! 이 파일에는 **진행 중인 편집이 없다.** 그리는 중인 획은 ② CanvasTool이 들고 있고,
//! 페이지에는 **확정된 획만** 들어간다. 그래서 "라이브 = 확정된 꼬리 + 진행 중 획"이
//! 그냥 두 목록의 합이 되고, 예전처럼 `committed_strokes()`로 마지막 획을 잘라내는
//! 규칙이 필요 없다.
//!
//! ```text
//! 편집 하나 = Edit 하나 (드래그 하나 = Undo 하나)
//! ```

use crate::geom::Size;
use crate::ink::Stroke;

/// 페이지의 한 장 — 크기(pt), PDF 배경 인덱스, 확정된 획들.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    size: Size,
    /// 배경으로 깔린 PDF 페이지 인덱스(0-based). `None`이면 빈 페이지.
    background: Option<usize>,
    strokes: Vec<Stroke>,
}

impl Page {
    pub fn blank(size: Size) -> Self {
        Self {
            size,
            background: None,
            strokes: Vec::new(),
        }
    }

    /// PDF 페이지를 배경으로 깐 페이지 — 그 위에 필기한다.
    pub fn with_background(size: Size, background: usize) -> Self {
        Self {
            size,
            background: Some(background),
            strokes: Vec::new(),
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn background(&self) -> Option<usize> {
        self.background
    }

    pub fn strokes(&self) -> &[Stroke] {
        &self.strokes
    }

    pub fn stroke_count(&self) -> usize {
        self.strokes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty()
    }

    /// 확정 획을 더한다 — 인덱스는 `len()`이 된다.
    pub fn push_stroke(&mut self, stroke: Stroke) -> usize {
        self.strokes.push(stroke);
        self.strokes.len() - 1
    }

    pub fn insert_stroke(&mut self, index: usize, stroke: Stroke) {
        self.strokes.insert(index.min(self.strokes.len()), stroke);
    }

    pub fn remove_stroke(&mut self, index: usize) -> Option<Stroke> {
        (index < self.strokes.len()).then(|| self.strokes.remove(index))
    }

    pub fn take_strokes(&mut self) -> Vec<Stroke> {
        std::mem::take(&mut self.strokes)
    }

    /// 반경 안의 획을 지운다 — **인덱스 오름차순**으로 돌려준다(복구가 정확해진다).
    pub fn erase_at(&mut self, center: crate::geom::Pt, radius: f32) -> Vec<(usize, Stroke)> {
        let targets: Vec<usize> = self
            .strokes
            .iter()
            .enumerate()
            .filter(|(_, stroke)| stroke.hits_circle(center, radius))
            .map(|(index, _)| index)
            .collect();
        let mut removed: Vec<(usize, Stroke)> = Vec::with_capacity(targets.len());
        for index in targets.iter().rev() {
            if let Some(stroke) = self.remove_stroke(*index) {
                removed.push((*index, stroke));
            }
        }
        removed.reverse();
        removed
    }

    /// 상태바/도움말용 한 줄.
    pub fn describe(&self) -> String {
        match self.background {
            Some(page) => format!("PDF page {} · {} strokes", page + 1, self.stroke_count()),
            None => format!("Blank page · {} strokes", self.stroke_count()),
        }
    }
}

/// 되돌릴 수 있는 편집 — **적용된 뒤에** 기록되고, 되돌리기가 정확히 원상복구한다.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    AddStroke {
        page: usize,
        index: usize,
        stroke: Stroke,
    },
    /// 지우개 드래그 하나 = 편집 하나. `removed`는 인덱스 오름차순.
    RemoveStrokes {
        page: usize,
        removed: Vec<(usize, Stroke)>,
    },
    ClearPage {
        page: usize,
        removed: Vec<Stroke>,
    },
    InsertPage {
        index: usize,
        page: Page,
    },
    RemovePage {
        index: usize,
        page: Page,
    },
}

impl Edit {
    /// 이 편집이 속한 페이지(페이지 편집이면 `None`).
    pub fn page(&self) -> Option<usize> {
        match self {
            Edit::AddStroke { page, .. }
            | Edit::RemoveStrokes { page, .. }
            | Edit::ClearPage { page, .. } => Some(*page),
            Edit::InsertPage { .. } | Edit::RemovePage { .. } => None,
        }
    }

    /// 이 편집의 이름 — 상태바가 쓴다(**영어만**).
    pub fn label(&self) -> &'static str {
        match self {
            Edit::AddStroke { .. } => "Ink",
            Edit::RemoveStrokes { .. } => "Erase",
            Edit::ClearPage { .. } => "Clear page",
            Edit::InsertPage { .. } => "Add page",
            Edit::RemovePage { .. } => "Remove page",
        }
    }

    /// 이 편집이 건드린 획 수(상태 표시·계측).
    pub fn affected_strokes(&self) -> usize {
        match self {
            Edit::AddStroke { .. } => 1,
            Edit::RemoveStrokes { removed, .. } => removed.len(),
            Edit::ClearPage { removed, .. } => removed.len(),
            Edit::InsertPage { .. } | Edit::RemovePage { .. } => 0,
        }
    }
}

/// Undo/Redo 스택 — 적용은 [`Doc`]이 한다.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl History {
    /// 보관 상한 — 넘으면 가장 오래된 편집부터 버린다.
    pub const LIMIT: usize = 512;

    /// 편집을 기록한다 — **redo는 버려진다**(표준 편집기 동작).
    pub fn push(&mut self, edit: Edit) {
        self.redo.clear();
        self.undo.push(edit);
        if self.undo.len() > Self::LIMIT {
            let overflow = self.undo.len() - Self::LIMIT;
            self.undo.drain(0..overflow);
        }
    }

    pub fn pop_undo(&mut self) -> Option<Edit> {
        let edit = self.undo.pop()?;
        self.redo.push(edit.clone());
        Some(edit)
    }

    pub fn pop_redo(&mut self) -> Option<Edit> {
        let edit = self.redo.pop()?;
        self.undo.push(edit.clone());
        Some(edit)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn last_label(&self) -> Option<&'static str> {
        self.undo.last().map(Edit::label)
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

/// 문서 — 페이지들 + 활성 페이지 + 히스토리. **진행 중 획은 여기 없다**(②가 들고 있다).
#[derive(Clone, Debug)]
pub struct Doc {
    title: String,
    pages: Vec<Page>,
    active: usize,
    history: History,
    dirty: bool,
}

impl Doc {
    /// 빈 A4 한 장으로 시작한다.
    pub fn blank(size: Size) -> Self {
        Self {
            title: "Untitled".to_string(),
            pages: vec![Page::blank(size)],
            active: 0,
            history: History::default(),
            dirty: false,
        }
    }

    /// PDF 페이지들로 문서를 만든다 — 페이지마다 배경을 깔고 그 위에 필기한다.
    pub fn from_pdf(sizes: impl IntoIterator<Item = Size>, title: impl Into<String>) -> Self {
        let pages: Vec<Page> = sizes
            .into_iter()
            .enumerate()
            .map(|(index, size)| Page::with_background(size, index))
            .collect();
        let mut doc = Self::blank(Size::A4);
        doc.title = title.into();
        if !pages.is_empty() {
            doc.pages = pages;
            doc.history.clear();
        }
        doc.dirty = false;
        doc
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

    pub fn page_mut(&mut self, index: usize) -> Option<&mut Page> {
        self.pages.get_mut(index)
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

    /// 활성 페이지의 **확정 획** — ③이 라이브 꼬리를 계산할 때 읽는다.
    pub fn strokes(&self) -> &[Stroke] {
        self.active_page().strokes()
    }

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
        self.insert_page(self.active + 1, Page::blank(size))
    }

    /// 페이지를 끼운다(Undo 가능).
    pub fn insert_page(&mut self, index: usize, page: Page) -> usize {
        let index = index.min(self.pages.len());
        self.pages.insert(index, page.clone());
        self.active = index;
        self.history.push(Edit::InsertPage { index, page });
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

    // ── 편집 기록 (②가 커밋할 때 부른다) ────────────────────────────────

    /// 확정 획을 페이지에 넣고 편집 하나로 기록한다. 돌려주는 인덱스가 `baked_count`의 기준이다.
    pub fn commit_stroke(&mut self, page: usize, stroke: Stroke) -> usize {
        let index = self.pages[page].push_stroke(stroke.clone());
        self.history.push(Edit::AddStroke {
            page,
            index,
            stroke,
        });
        self.dirty = true;
        index
    }

    /// 지우개 드래그가 지운 획들을 기록한다(빈 목록이면 아무 일도 없다).
    pub fn commit_erasure(&mut self, page: usize, removed: Vec<(usize, Stroke)>) -> bool {
        if removed.is_empty() {
            return false;
        }
        self.history.push(Edit::RemoveStrokes { page, removed });
        self.dirty = true;
        true
    }

    /// 진행 중이던 지우개 결과를 되돌린다(취소) — 히스토리에는 남기지 않는다.
    pub fn restore_erased(&mut self, page: usize, removed: Vec<(usize, Stroke)>) {
        for (index, stroke) in removed.into_iter().rev() {
            self.pages[page].insert_stroke(index, stroke);
        }
    }

    // ── 되돌리기 ─────────────────────────────────────────────────────

    pub fn undo(&mut self) -> Option<Edit> {
        let edit = self.history.pop_undo()?;
        self.revert(&edit);
        self.dirty = true;
        Some(edit)
    }

    pub fn redo(&mut self) -> Option<Edit> {
        let edit = self.history.pop_redo()?;
        self.apply(&edit);
        self.dirty = true;
        Some(edit)
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
                    for stroke in removed {
                        page.push_stroke(stroke.clone());
                    }
                }
            }
            Edit::InsertPage { index, .. } => {
                if *index < self.pages.len() {
                    self.pages.remove(*index);
                    self.active = self.active.min(self.pages.len().saturating_sub(1));
                }
            }
            Edit::RemovePage { index, page } => {
                let index = (*index).min(self.pages.len());
                self.pages.insert(index, page.clone());
                self.active = index;
            }
        }
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

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// 마지막 편집의 이름(상태바) — 없으면 `None`.
    pub fn last_edit(&self) -> Option<&'static str> {
        self.history.last_label()
    }

    pub fn total_strokes(&self) -> usize {
        self.pages.iter().map(Page::stroke_count).sum()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// 상태바 한 줄 — UI와 테스트가 같은 문자열을 본다.
    pub fn status_line(&self) -> String {
        format!(
            "{} · page {} / {} · {} strokes{}",
            self.title,
            self.active + 1,
            self.pages.len(),
            self.active_page().stroke_count(),
            if self.dirty { " · Unsaved" } else { "" }
        )
    }
}

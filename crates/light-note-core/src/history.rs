//! 되돌리기/다시하기 — 편집 하나 = [`Edit`] 하나.
//!
//! 규칙 (계약):
//! - **드래그 하나 = 편집 하나** — 스트로크 100개를 지운 지우개 드래그도 Undo 한 번에 복구된다.
//! - 새 편집을 하면 redo 스택은 버려진다(표준 동작).
//! - 스택은 [`History::LIMIT`]개로 잘린다 — 메모리가 무한정 늘지 않는다.

use crate::doc::Page;
use crate::ink::Stroke;

/// 되돌릴 수 있는 편집. **적용된 뒤에** 기록되며, `revert`가 정확히 되돌린다.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// 스트로크 추가(드래그 완료 시점).
    AddStroke {
        page: usize,
        index: usize,
        stroke: Stroke,
    },
    /// 스트로크 삭제(지우개 드래그 하나 = 편집 하나). `removed`는 인덱스 오름차순.
    RemoveStrokes {
        page: usize,
        removed: Vec<(usize, Stroke)>,
    },
    /// 페이지 전체 지우기.
    ClearPage { page: usize, removed: Vec<Stroke> },
    /// 페이지 삽입.
    InsertPage { index: usize, page: Page },
    /// 페이지 삭제.
    RemovePage { index: usize, page: Page },
}

impl Edit {
    /// 이 편집이 속한 페이지(있으면).
    pub fn page(&self) -> Option<usize> {
        match self {
            Edit::AddStroke { page, .. }
            | Edit::RemoveStrokes { page, .. }
            | Edit::ClearPage { page, .. } => Some(*page),
            Edit::InsertPage { .. } | Edit::RemovePage { .. } => None,
        }
    }

    /// 상태바에 띄울 짧은 이름.
    pub fn label(&self) -> &'static str {
        match self {
            Edit::AddStroke { .. } => "필기",
            Edit::RemoveStrokes { .. } => "지우기",
            Edit::ClearPage { .. } => "페이지 비우기",
            Edit::InsertPage { .. } => "페이지 추가",
            Edit::RemovePage { .. } => "페이지 삭제",
        }
    }

    /// 이 편집이 지운/추가한 스트로크 수(계측·상태 표시용).
    pub fn affected_strokes(&self) -> usize {
        match self {
            Edit::AddStroke { .. } => 1,
            Edit::RemoveStrokes { removed, .. } => removed.len(),
            Edit::ClearPage { removed, .. } => removed.len(),
            Edit::InsertPage { .. } | Edit::RemovePage { .. } => 0,
        }
    }
}

/// Undo/Redo 스택. 상태를 직접 만지지 않는다 — 적용은 [`crate::doc::Document`]가 한다.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl History {
    /// 보관 상한. 넘으면 가장 오래된 편집부터 버린다.
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

    /// Undo 대상 하나를 꺼내 redo로 옮긴다.
    pub fn pop_undo(&mut self) -> Option<Edit> {
        let edit = self.undo.pop()?;
        self.redo.push(edit.clone());
        Some(edit)
    }

    /// Redo 대상 하나를 꺼내 undo로 옮긴다.
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

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// 가장 최근 편집(상태바 미리보기).
    pub fn last_edit(&self) -> Option<&Edit> {
        self.undo.last()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Size;
    use crate::ink::{InkPoint, StrokeStyle, Tool};

    fn stroke() -> Stroke {
        Stroke::new(
            Tool::Pen,
            StrokeStyle::for_tool(Tool::Pen),
            InkPoint::new(0.0, 0.0),
        )
    }

    fn add_stroke_edit() -> Edit {
        Edit::AddStroke {
            page: 0,
            index: 0,
            stroke: stroke(),
        }
    }

    #[test]
    fn new_edit_drops_the_redo_stack() {
        let mut history = History::default();
        history.push(add_stroke_edit());
        assert!(history.pop_undo().is_some());
        assert!(history.can_redo());

        history.push(add_stroke_edit());
        assert!(!history.can_redo(), "새 편집은 redo를 버린다");
    }

    #[test]
    fn stack_is_capped() {
        let mut history = History::default();
        for _ in 0..(History::LIMIT + 10) {
            history.push(add_stroke_edit());
        }
        assert_eq!(history.undo_len(), History::LIMIT);
    }

    #[test]
    fn page_edits_report_their_label_and_size() {
        let edit = Edit::InsertPage {
            index: 0,
            page: Page::blank(Size::A4),
        };
        assert_eq!(edit.label(), "페이지 추가");
        assert_eq!(edit.page(), None);
        assert_eq!(edit.affected_strokes(), 0);
        assert_eq!(add_stroke_edit().affected_strokes(), 1);
    }
}

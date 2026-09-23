//! Document: pages, strokes, history and the eraser.
//!
//! A [`Document`] is a plain value.  The UI thread owns it, mutates it in O(1)
//! per pen sample, and hands a `clone()` of one [`Page`] to a worker when it
//! wants a bitmap — so no lock is ever shared with the rasterizer.

use serde::{Deserialize, Serialize};

use crate::geom::{Pt, Size};
use crate::ink::{Stroke, Tool};

/// One page of a note: a size, an optional PDF page behind the ink, and ink.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page {
    /// Page size in pt.
    pub size: Size,
    /// Index of the background PDF page (0-based), or `None` for blank paper.
    pub background: Option<usize>,
    /// The strokes on this page, in the order they were drawn.
    pub strokes: Vec<Stroke>,
}

impl Page {
    /// Blank paper.
    pub fn blank(size: Size) -> Self {
        Self {
            size,
            background: None,
            strokes: Vec::new(),
        }
    }

    /// A page backed by PDF page `background`.
    pub fn with_background(size: Size, background: usize) -> Self {
        Self {
            size,
            background: Some(background),
            strokes: Vec::new(),
        }
    }

    /// Every stroke index the eraser would remove at `at`.
    pub fn hits(&self, at: Pt, radius: f32, tool: Tool) -> Vec<usize> {
        if !tool.erases() {
            return Vec::new();
        }
        self.strokes
            .iter()
            .enumerate()
            .filter(|(_, stroke)| stroke.touches(at, radius))
            .map(|(index, _)| index)
            .collect()
    }
}

/// One undoable change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Edit {
    /// Adds a stroke to a page (the end of the page's stroke list).
    AddStroke { page: usize, stroke: Stroke },
    /// Removes strokes from a page.  Each removed stroke keeps its original
    /// index, so undo restores the page exactly — order included.
    RemoveStrokes {
        page: usize,
        removed: Vec<(usize, Stroke)>,
    },
}

impl Edit {
    /// The page this edit applies to.
    pub fn page(&self) -> usize {
        match self {
            Self::AddStroke { page, .. } | Self::RemoveStrokes { page, .. } => *page,
        }
    }
}

/// Pages, the current page, and a linear history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pages: Vec<Page>,
    current: usize,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl Document {
    /// One blank page.
    pub fn blank(size: Size) -> Self {
        Self::from_pages(vec![Page::blank(size)])
    }

    /// One page per PDF page, each showing that page as its background.
    pub fn from_backgrounds(sizes: &[Size]) -> Self {
        let pages = sizes
            .iter()
            .enumerate()
            .map(|(index, size)| Page::with_background(*size, index))
            .collect();
        Self::from_pages(pages)
    }

    /// A document over exactly these pages.
    pub fn from_pages(pages: Vec<Page>) -> Self {
        assert!(!pages.is_empty(), "a document always has at least one page");
        Self {
            pages,
            current: 0,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn current_index(&self) -> usize {
        self.current
    }

    /// The page the pen writes on.
    pub fn page(&self) -> &Page {
        &self.pages[self.current]
    }

    pub fn page_mut(&mut self) -> &mut Page {
        &mut self.pages[self.current]
    }

    /// Total strokes over all pages (a status-bar number).
    pub fn total_strokes(&self) -> usize {
        self.pages.iter().map(|page| page.strokes.len()).sum()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Moves to `index`; refused (not clamped) when it does not exist.
    pub fn go_to(&mut self, index: usize) -> bool {
        if index < self.pages.len() {
            self.current = index;
            true
        } else {
            false
        }
    }

    pub fn next_page(&mut self) -> bool {
        self.go_to(self.current + 1)
    }

    pub fn previous_page(&mut self) -> bool {
        if self.current == 0 {
            false
        } else {
            self.go_to(self.current - 1)
        }
    }

    /// Adds a stroke to the current page.
    pub fn add_stroke(&mut self, stroke: Stroke) -> bool {
        self.apply(Edit::AddStroke {
            page: self.current,
            stroke,
        })
    }

    /// Removes the strokes at `indices` from the current page.
    pub fn remove_strokes(&mut self, indices: &[usize]) -> bool {
        if indices.is_empty() {
            return false;
        }
        let mut indices = indices.to_vec();
        indices.sort_unstable();
        indices.dedup();
        let page = &self.pages[self.current];
        if indices.iter().any(|index| *index >= page.strokes.len()) {
            return false;
        }
        let removed = indices
            .iter()
            .map(|index| (*index, page.strokes[*index].clone()))
            .collect();
        self.apply(Edit::RemoveStrokes {
            page: self.current,
            removed,
        })
    }

    /// Every stroke the eraser would remove at `at` on the current page.
    pub fn hits(&self, at: Pt, radius: f32, tool: Tool) -> Vec<usize> {
        self.page().hits(at, radius, tool)
    }

    /// Removes every stroke on the current page as **one** undoable edit.
    pub fn clear_page(&mut self) -> bool {
        let count = self.page().strokes.len();
        if count == 0 {
            return false;
        }
        self.remove_strokes(&(0..count).collect::<Vec<_>>())
    }

    /// Applies an edit and records it.  Refused (and not recorded) when the edit
    /// does not fit the document — the history never lies about what happened.
    pub fn apply(&mut self, edit: Edit) -> bool {
        let page = edit.page();
        if page >= self.pages.len() {
            return false;
        }
        match &edit {
            Edit::AddStroke { stroke, .. } => self.pages[page].strokes.push(stroke.clone()),
            Edit::RemoveStrokes { removed, .. } => {
                if removed.is_empty() {
                    return false;
                }
                for (index, _) in removed.iter().rev() {
                    if *index >= self.pages[page].strokes.len() {
                        return false;
                    }
                    self.pages[page].strokes.remove(*index);
                }
            }
        }
        self.undo.push(edit);
        self.redo.clear();
        true
    }

    /// Undoes the last edit.
    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.undo.pop() else {
            return false;
        };
        match &edit {
            Edit::AddStroke { page, .. } => {
                self.pages[*page].strokes.pop();
            }
            Edit::RemoveStrokes { page, removed } => {
                // Ascending, so every index still refers to the original list.
                for (index, stroke) in removed.iter() {
                    let at = (*index).min(self.pages[*page].strokes.len());
                    self.pages[*page].strokes.insert(at, stroke.clone());
                }
            }
        }
        self.redo.push(edit);
        true
    }

    /// Re-applies the last undone edit.
    pub fn redo(&mut self) -> bool {
        let Some(edit) = self.redo.pop() else {
            return false;
        };
        match &edit {
            Edit::AddStroke { page, stroke } => self.pages[*page].strokes.push(stroke.clone()),
            Edit::RemoveStrokes { page, removed } => {
                for (index, _) in removed.iter().rev() {
                    self.pages[*page].strokes.remove(*index);
                }
            }
        }
        self.undo.push(edit);
        true
    }
}
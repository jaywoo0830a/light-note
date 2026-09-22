//! 화면 조각 — **단축키 패널**: 키 하나 = 알약 하나 + 설명 한 줄.
//!
//! 열림 상태는 **호스트가 소유**한다(`view.help`) — elm은 `<If>`로 자리만 정하고,
//! F1/Esc는 elm이 선언해 **의도로 바꿔** 호스트에 넘긴다(`ui.rs`의 `Screen`).

use elm_magic_windows_reactor::RawSlot;
use windows_reactor::{FontWeight, View};

use super::{card, chip, column, label, meta, row};
use crate::style::TOKENS;

/// 단축키 패널 하나.
pub fn shortcuts() -> RawSlot {
    let body = column(
        TOKENS.gap,
        vec![
            label("Keyboard shortcuts", TOKENS.title, FontWeight::SEMI_BOLD).into(),
            shortcut("F1", "Toggle this panel"),
            shortcut("Esc", "Close this panel"),
            shortcut("Ctrl + Plus / Minus", "Zoom in and out"),
            shortcut("Ctrl + Enter", "Add a page"),
            shortcut("Toolbar", "Everything else is a button above"),
        ],
    );
    Some(card(TOKENS.pad, body))
}

/// 한 줄 — 키(알약) + 설명.
fn shortcut(keys: &str, description: &str) -> View {
    row(TOKENS.gap, vec![chip(keys), meta(description).into()])
}

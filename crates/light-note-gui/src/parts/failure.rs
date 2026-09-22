//! 화면 조각 — **실패**: 이유를 그 자리에서 말하고 복구 동선을 같은 자리에 둔다.
//!
//! 이유는 `Stage::Failed`가 들고 있다([`ViewModel::stage`]) — 조각은 문장을 만들지 않고
//! 그대로 옮긴다. 색은 WinUI의 `InfoBar`가 정한다(오류 = `Error` 심각도).

use elm_magic_windows_reactor::RawSlot;
use windows_reactor::{ContentControl, CornerRadius, InfoBar, InfoBarSeverity, Thickness, View};

use super::{buttons, column, meta, row};
use crate::style::TOKENS;
use crate::ui::{Intent, IntentSink, ViewModel};

/// 실패 카드 하나.
pub fn failure(view: &ViewModel, sink: &IntentSink) -> RawSlot {
    let bar: View = InfoBar::new()
        .severity(InfoBarSeverity::Error)
        .is_open(true)
        .is_closable(false)
        .title("Could not open the file")
        .message(view.stage.message())
        .into();
    let action = row(
        TOKENS.gap,
        vec![
            buttons::label_button(Intent::Retry.label(), Intent::Retry, sink),
            meta("or open another PDF from the toolbar").into(),
        ],
    );
    let body = column(TOKENS.block, vec![bar, action]);

    Some(
        windows_reactor::Border::new()
            .corner_radius(CornerRadius::uniform(TOKENS.radius))
            .padding(Thickness::uniform(TOKENS.pad))
            .content(body),
    )
}

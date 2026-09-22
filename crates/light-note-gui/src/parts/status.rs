//! 화면 조각 — **정보 띠**: 위 줄은 사람이 읽는 문장, 아래 줄은 힌트와 사실 요약.
//!
//! 본문 **위**에 놓인다(`ui.rs`의 `Screen`): 아래에 두면 잉크 영역의 높이 추정
//! ([`super::content_height`])이 틀릴 때 화면 밖으로 밀려 아무도 못 본다.
//!
//! 문장은 **줄바꿈 금지 + 말줄임**(예제 22), 사실은 `meta`(작고 흐린 글자)로 둔다 —
//! 색을 새로 만들지 않고 투명도로 낮춘다. 아래쪽 1px 선이 본문과의 경계다(예제 28).
//!
//! 두 줄인 이유: 한 줄에 문장과 사실을 함께 두면 긴 경로(`Saved: C:\…`)가 사실 위에
//! 겹친다 — Grid의 열 정의를 이 백엔드에서 쓸 수 없어(GridColumns 미지원) 문장이 줄어들
//! 자리가 없기 때문이다.

use elm_magic_windows_reactor::RawSlot;
use windows_reactor::{Border, ContentControl, FontWeight, LayoutControl, Thickness, View};

use super::{chrome_brush, clipped, column, meta, row, stroke_brush};
use crate::style::TOKENS;
use crate::ui::ViewModel;

/// 상태바 하나.
pub fn status(view: &ViewModel) -> RawSlot {
    // 위 줄: 마지막으로 한 일.
    let message = clipped(view.status.clone(), TOKENS.body, FontWeight::NORMAL);
    // 아래 줄 왼쪽: 지금 도구의 사용법.
    let hint = clipped(view.hint(), TOKENS.caption, FontWeight::NORMAL).opacity(0.7);

    // 아래 줄 오른쪽: 있는 것만 말한다(없으면 자리를 차지하지 않는다).
    let mut facts: Vec<View> = vec![
        meta(format!("Tool: {}", view.tool_label())).into(),
        meta(format!("{} strokes", view.stroke_count)).into(),
        meta(view.pipeline_label()).into(),
        meta(format!("Zoom {}", view.zoom_label())).into(),
    ];
    if view.can_undo {
        facts.push(meta("Undo available").into());
    }
    if view.can_redo {
        facts.push(meta("Redo available").into());
    }
    if view.dirty {
        facts.push(meta("Unsaved").into());
    }

    let body = column(
        TOKENS.tight,
        vec![
            message.into(),
            row(
                TOKENS.block,
                vec![hint.into()]
                    .into_iter()
                    .chain(facts)
                    .collect::<Vec<View>>(),
            ),
        ],
    );

    Some(
        Border::new()
            .background(chrome_brush())
            .border_brush(stroke_brush())
            .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
            .padding(Thickness::xy(TOKENS.pad, TOKENS.tight))
            .min_height(TOKENS.status_h)
            .content(body),
    )
}

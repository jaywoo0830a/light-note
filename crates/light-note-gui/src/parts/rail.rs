//! 화면 조각 — **좌측 레일**: 페이지 목록 + 페이지 조작 + 배율.
//!
//! 폭은 토큰(`rail`)으로 **고정**이다(예제 27): 페이지가 늘어도 본문이 밀리지 않는다.
//! 활성 페이지는 **왼쪽 액센트 바 + 굵은 글자**(예제 24)로 표시하고, 줄 자체가 버튼이라
//! 눌러서 그 페이지로 갈 수 있다([`Intent::GoToPage`]).
//!
//! 목록에 `ScrollViewer`를 두지 않았다 — elm 트리가 `StackPanel`뿐이라 높이가 묶이지 않아
//! 스크롤이 생기지 않는다(자세한 이유는 `parts::paper`의 문서에 있다).

use elm_magic_windows_reactor::RawSlot;
use windows_reactor::{Border, CornerRadius, FontWeight, LayoutControl, VerticalAlignment, View};

use super::{
    accent_brush, buttons, card_fixed, clipped, column, divider, hairline, label, meta, row,
    stroke_brush,
};
use crate::style::TOKENS;
use crate::ui::{Intent, IntentSink, ViewModel};

/// 레일 하나.
pub fn rail(view: &ViewModel, sink: &IntentSink) -> RawSlot {
    // 머리: 제목 + 전체 페이지 수.
    let head = row(
        TOKENS.gap,
        vec![
            label("Pages", TOKENS.title, FontWeight::SEMI_BOLD).into(),
            meta(format!("{} total", view.page_count)).into(),
        ],
    );

    // 목록: 페이지 줄.
    let mut rows: Vec<View> = Vec::new();
    if view.page_labels.is_empty() {
        rows.push(meta("No pages yet").into());
    }
    for (index, text) in view.page_labels.iter().enumerate() {
        rows.push(page_row(index, index == view.page, text, sink));
    }
    let list = column(TOKENS.tight, rows);

    // 바닥: 페이지 조작 · 배율 · 파이프라인 요약.
    let actions = row(
        TOKENS.tight,
        vec![
            buttons::icon_button(Intent::PagePrev, sink),
            buttons::icon_button(Intent::PageNext, sink),
            divider(),
            buttons::icon_button(Intent::PageAdd, sink),
            buttons::icon_button(Intent::PageRemove, sink),
        ],
    );
    let zoom = row(
        TOKENS.tight,
        vec![
            meta("Zoom").into(),
            buttons::icon_button(Intent::ZoomOut, sink),
            label(view.zoom_label(), TOKENS.body, FontWeight::SEMI_BOLD).into(),
            buttons::icon_button(Intent::ZoomIn, sink),
        ],
    );
    let foot = column(
        TOKENS.gap,
        vec![actions, zoom, meta(view.pipeline_label()).into()],
    );

    Some(card_fixed(
        TOKENS.pad,
        TOKENS.rail,
        super::content_height(view),
        column(TOKENS.block, vec![head, list, hairline(), foot]),
    ))
}

/// 페이지 한 줄 — **줄 자체가 버튼**이다(누르면 그 페이지로 간다).
fn page_row(index: usize, active: bool, text: &str, sink: &IntentSink) -> View {
    let bar: View = Border::new()
        .width(3.0)
        .height(TOKENS.body + TOKENS.tight)
        .background(if active {
            accent_brush()
        } else {
            stroke_brush()
        })
        .corner_radius(CornerRadius::uniform(TOKENS.pill))
        .vertical_alignment(VerticalAlignment::Center)
        .into();
    let weight = if active {
        FontWeight::SEMI_BOLD
    } else {
        FontWeight::NORMAL
    };
    let content = row(
        TOKENS.gap,
        vec![bar, clipped(text, TOKENS.body, weight).into()],
    );
    let tooltip = format!("{} {}", Intent::GoToPage(index).label(), index + 1);
    buttons::row_button(content, active, &tooltip, Intent::GoToPage(index), sink)
}

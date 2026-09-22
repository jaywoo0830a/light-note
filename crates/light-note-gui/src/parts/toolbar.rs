//! 화면 조각 — **툴바**: 아이콘 + 라벨 버튼 묶음 + 잉크 미리보기.
//!
//! 버튼의 **순서와 묶음**은 [`Intent::TOOLBAR`]에 있다 — 플랫폼 무관 정의라
//! `tests/ui_plan.rs`가 그대로 검증한다(빠진 의도가 없다). 여기서는 그 정의를
//! **그리기만** 한다: 묶음 사이에 세로 구분선, 활성 도구는 액센트 면.
//!
//! ## 이름을 붙이고, 좁으면 줄을 나눈다
//! 아이콘만 있는 버튼은 툴팁을 읽어야 뜻이 통한다. 그래서 버튼마다 **이름을 옆에** 둔다
//! ([`buttons::labeled_button`]). 대신 줄이 길어지므로 **창 폭**에 따라 배치를 바꾼다:
//!
//! - 창이 [`TOKENS.toolbar_break`](=1360 DIP) 이상: **한 줄**(버튼 12개).
//! - 그보다 좁으면: **두 줄** — 도구(펜·형광펜·지우개 / 굵기) · 동작(되돌리기·지우기·
//!   열기·내보내기·도움말). 나누는 자리는 정의의 묶음 경계([`TOOL_SPLIT`])다.
//!
//! 두 배치 모두 **같은 라벨**을 유지한다(좁다고 뜻을 잃지 않는다). 폭은
//! [`crate::style::Tokens::labeled_row_width`]가 어림하고, `tests/ui_plan.rs`가 **실제
//! 라벨**로 두 배치가 기준 폭 안에 서는지 검사한다 — 어림값이 계약이 되는 자리다.
//!
//! ## 잉크 미리보기
//! 마지막 줄의 띠는 "지금 무엇으로 그리는지"를 눈으로 확인시킨다: 색 · 굵기(pt) ·
//! 투명도(형광펜은 반투명하다). 버튼은 결과를 바꾸고, 이 띠는 결과를 보여준다.

use elm_magic_windows_reactor::RawSlot;
use windows_reactor::{
    Border, Brush, ContentControl, CornerRadius, FontWeight, HorizontalAlignment, LayoutControl,
    Thickness, VerticalAlignment, View,
};

use super::{buttons, chrome_brush, column, divider, label, meta, rgb, row, stroke_brush};
use crate::ink::{Style, Tool};
use crate::style::TOKENS;
use crate::ui::{Intent, IntentSink, ViewModel};

/// 툴바 하나 — 버튼 줄(창 폭에 따라 1~2줄) + 잉크 미리보기 줄.
pub fn toolbar(view: &ViewModel, sink: &IntentSink) -> RawSlot {
    let mut rows: Vec<View> = button_rows(view, sink);
    rows.push(row(TOKENS.gap, preview(view)));

    // 줄을 **쌓는다**(세로 `StackPanel`): Grid의 열 정의·정렬은 이 백엔드에서 폭을 못 받아
    // 자식이 사라진다(`parts::row` 참고). 줄 사이 간격도 리듬 토큰이다.
    let body = column(TOKENS.gap, rows);

    Some(
        Border::new()
            .background(super::surface_brush())
            .border_brush(stroke_brush())
            .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
            .padding(Thickness::xy(TOKENS.pad, TOKENS.tight))
            .content(body),
    )
}

/// 좁은 창에서 도구와 동작을 가르는 자리 — [`Intent::TOOLBAR`]의 **묶음 인덱스**다.
///
/// `tests/ui_plan.rs`가 이 값으로 두 줄의 폭을 검사한다(정의와 화면이 같은 자리를 본다).
pub const TOOL_SPLIT: usize = 2;

/// 버튼 줄 — 창이 넓으면 한 줄, 좁으면 둘로 나눈다(둘 다 **라벨은 유지**).
fn button_rows(view: &ViewModel, sink: &IntentSink) -> Vec<View> {
    if view.viewport.0 >= TOKENS.toolbar_break {
        return vec![button_row(&Intent::TOOLBAR, view, sink)];
    }
    vec![
        button_row(&Intent::TOOLBAR[..TOOL_SPLIT], view, sink),
        button_row(&Intent::TOOLBAR[TOOL_SPLIT..], view, sink),
    ]
}

/// 묶음 목록 **한 줄** — 묶음 사이에 세로 구분선, 버튼에는 아이콘 + 라벨.
fn button_row(groups: &[&'static [Intent]], view: &ViewModel, sink: &IntentSink) -> View {
    let mut children: Vec<View> = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        if index > 0 {
            children.push(divider());
        }
        for intent in group.iter().copied() {
            children.push(match tool_of(intent) {
                // 도구는 **활성 상태**를 가진다(액센트 면) — 지금 무엇으로 그리는지가 보인다.
                Some(tool) => buttons::labeled_tool_button(view.tool == tool, intent, sink),
                None => buttons::labeled_button(intent, sink),
            });
        }
    }
    row(TOKENS.tight, children)
}

/// 의도 → 도구 — 도구 버튼만 활성 상태를 가진다.
fn tool_of(intent: Intent) -> Option<Tool> {
    match intent {
        Intent::Pen => Some(Tool::Pen),
        Intent::Highlighter => Some(Tool::Highlighter),
        Intent::Eraser => Some(Tool::Eraser),
        _ => None,
    }
}

/// 잉크 미리보기 — 색 막대 + 도구 이름 + 굵기. **툴바 오른쪽 끝**에 앉는다.
///
/// 막대는 알약(`pill`)이다: 둥근 캡을 가진 잉크와 같은 모양이고 좌표가 필요 없다.
/// 굵기가 상자 높이를 넘으면 **자른다**(넘치면 조용히 잘리기 때문이다).
fn preview(view: &ViewModel) -> Vec<View> {
    let ink = Style::for_tool(view.tool);
    let thickness = (view.width_pt as f64).clamp(1.0, TOKENS.line_max);
    let alpha = f64::from(ink.color.a) / 255.0;

    // 종이 여백을 흉내 낸다: 막대는 상자보다 좁게(양옆 `gap`만큼) 그린다.
    let bar: View = Border::new()
        .width(TOKENS.preview_w - TOKENS.gap * 2.0)
        .height(thickness)
        .corner_radius(CornerRadius::uniform(TOKENS.pill))
        .background(Brush::from(rgb(ink.color)))
        .opacity(alpha)
        .horizontal_alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Center)
        .into();

    let swatch: View = Border::new()
        .width(TOKENS.preview_w)
        .height(TOKENS.preview_h)
        .background(chrome_brush())
        .border_brush(stroke_brush())
        .border_thickness(Thickness::uniform(1.0))
        .corner_radius(CornerRadius::uniform(TOKENS.control))
        .content(bar);

    vec![
        label("Ink", TOKENS.caption, FontWeight::SEMI_BOLD).into(),
        swatch,
        meta(format!("{} · {:.1} pt", view.tool_label(), view.width_pt)).into(),
    ]
}

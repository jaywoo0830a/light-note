//! 화면 조각 — **툴바**: 아이콘 버튼 묶음 + 잉크 미리보기.
//!
//! 버튼의 **순서와 묶음**은 [`Intent::TOOLBAR`]에 있다 — 플랫폼 무관 정의라
//! `tests/ui_plan.rs`가 그대로 검증한다(빠진 의도가 없다). 여기서는 그 정의를
//! **그리기만** 한다: 묶음 사이에 세로 구분선, 활성 도구는 액센트 면.
//!
//! 오른쪽 끝의 **잉크 미리보기**는 "지금 무엇으로 그리는지"를 눈으로 확인시킨다:
//! 색 · 굵기(pt) · 투명도(형광펜은 반투명하다). 버튼은 결과를 바꾸고, 이 띠는 결과를 보여준다.

use elm_magic_windows_reactor::RawSlot;
use windows_reactor::{
    Border, Brush, ContentControl, CornerRadius, FontWeight, HorizontalAlignment, LayoutControl,
    Thickness, VerticalAlignment, View,
};

use super::{buttons, chrome_brush, column, divider, label, meta, rgb, row, stroke_brush};
use crate::ink::{Style, Tool};
use crate::style::TOKENS;
use crate::ui::{Intent, IntentSink, ViewModel};

/// 툴바 하나.
pub fn toolbar(view: &ViewModel, sink: &IntentSink) -> RawSlot {
    // ── 버튼 묶음: 정의(`Intent::TOOLBAR`)를 순서대로 그린다 ──
    let mut children: Vec<View> = Vec::new();
    for (index, group) in Intent::TOOLBAR.iter().enumerate() {
        if index > 0 {
            children.push(divider());
        }
        for intent in group.iter().copied() {
            children.push(match tool_of(intent) {
                // 도구는 **활성 상태**를 가진다(액센트 면) — 지금 무엇으로 그리는지가 보인다.
                Some(tool) => buttons::tool_button(view.tool == tool, intent, sink),
                None => buttons::icon_button(intent, sink),
            });
        }
    }

    // 두 줄: 버튼 묶음 / 잉크 미리보기.
    // (Grid의 열 정의·정렬은 이 백엔드에서 폭을 못 받아 자식이 사라진다 — `parts::row` 참고.)
    let body = column(
        TOKENS.gap,
        vec![row(TOKENS.tight, children), row(TOKENS.gap, preview(view))],
    );

    Some(
        Border::new()
            .background(super::surface_brush())
            .border_brush(stroke_brush())
            .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
            .padding(Thickness::xy(TOKENS.pad, TOKENS.tight))
            .content(body),
    )
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

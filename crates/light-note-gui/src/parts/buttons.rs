//! 조각들이 쓰는 **버튼 어휘** — elm의 `<Button>`을 대신한다.
//!
//! ## 왜 elm 버튼을 안 쓰는가
//! 스타일을 줄 통로가 `<Raw>`뿐이다(예제 16·26): elm의 `<Button>`은 WinUI 기본 모양으로
//! 굳고, 색·크기·아이콘·툴팁을 줄 방법이 없다. 반대로 `<Raw>` 안에서는 **elm 콜백**을
//! 못 쓰지만 **호스트가 넘긴 [`IntentSink`]는 캡처할 수 있다** — 표면이 포인터 이벤트를
//! 다루는 것과 같은 방법이다(`app.rs`가 sender를 캡처해 빌더를 등록한다).
//!
//! ## 규칙
//! - **아이콘은 `SymbolIcon`**(WinUI 내장 심볼 폰트) — 글꼴 패밀리를 못 바꾸는 제약과
//!   무관하고, 색은 테마를 따라간다.
//! - **툴팁 + 자동화 이름을 항상 같이** 준다: 아이콘만 있는 버튼은 눈으로만 읽을 수 없다.
//! - **활성 상태는 `ButtonStyle::Accent`**, 나머지는 `Subtle`(호버에만 배경) — WinUI가
//!   대비를 보장하므로 색을 계산하지 않는다(예제 25의 "테마가 정하게 둔다").
//! - 크기는 토큰(`control_h`)에서 온다 — 정사각 아이콘 버튼.
//! - `content()`는 `View`를 돌려주므로 **마지막에** 부른다(리액터의 사실).

use std::rc::Rc;

use windows_reactor::{
    AutomationExt, Button, ButtonStyle, ContentControl, FontWeight, HorizontalAlignment,
    LayoutControl, Symbol, SymbolIcon, TooltipExt, View,
};

use crate::style::TOKENS;
use crate::ui::{Intent, IntentSink};

/// 의도 → 아이콘 — **전수 match**라 의도가 늘면 컴파일이 멈춘다(아이콘 없는 버튼이 없다).
pub fn symbol(intent: Intent) -> Symbol {
    match intent {
        Intent::Pen => Symbol::Edit,
        Intent::Highlighter => Symbol::Highlight,
        Intent::Eraser => Symbol::Delete,
        Intent::Thinner => Symbol::FontDecrease,
        Intent::Thicker => Symbol::FontIncrease,
        Intent::Undo => Symbol::Undo,
        Intent::Redo => Symbol::Redo,
        Intent::Clear => Symbol::Clear,
        Intent::Open => Symbol::OpenFile,
        Intent::ExportPng => Symbol::Pictures,
        Intent::ExportPdf => Symbol::Document,
        Intent::PageAdd => Symbol::Add,
        Intent::PageRemove => Symbol::Remove,
        Intent::PagePrev => Symbol::Previous,
        Intent::PageNext => Symbol::Next,
        Intent::ZoomIn => Symbol::ZoomIn,
        Intent::ZoomOut => Symbol::ZoomOut,
        Intent::Retry => Symbol::Refresh,
        Intent::ToggleHelp => Symbol::Help,
        Intent::CloseHelp => Symbol::Cancel,
        Intent::GoToPage(_) => Symbol::Page,
    }
}

/// 동작 버튼 — 아이콘 하나(호버에만 배경이 생긴다).
pub fn icon_button(intent: Intent, sink: &IntentSink) -> View {
    square_button(ButtonStyle::Subtle, intent, sink)
}

/// 도구 버튼 — **활성 도구는 액센트 면**이라 지금 무엇으로 그리는지가 한눈에 보인다.
pub fn tool_button(active: bool, intent: Intent, sink: &IntentSink) -> View {
    let style = if active {
        ButtonStyle::Accent
    } else {
        ButtonStyle::Subtle
    };
    square_button(style, intent, sink)
}

/// **정사각 아이콘 버튼** — 모든 아이콘 버튼이 지나는 길(모양이 갈라지지 않는다).
fn square_button(style: ButtonStyle, intent: Intent, sink: &IntentSink) -> View {
    let sink = Rc::clone(sink);
    Button::new()
        .style(style)
        .width(TOKENS.control_h)
        .height(TOKENS.control_h)
        .automation_name(intent.label())
        .on_click(move || sink(intent))
        .content(SymbolIcon::new().symbol(symbol(intent)))
        .tooltip(intent.label())
}

/// **넓은 버튼** — 내용을 왼쪽 정렬한 줄(레일의 페이지 목록이 쓴다).
///
/// 내용(강조 바 + 글자)은 부르는 쪽이 만든다: 버튼은 "누를 수 있다"만 책임진다.
pub fn row_button(
    content: impl Into<View>,
    active: bool,
    tooltip: &str,
    intent: Intent,
    sink: &IntentSink,
) -> View {
    let sink = Rc::clone(sink);
    let style = if active {
        ButtonStyle::Default
    } else {
        ButtonStyle::Subtle
    };
    Button::new()
        .style(style)
        .horizontal_content_alignment(HorizontalAlignment::Left)
        .automation_name(tooltip)
        .on_click(move || sink(intent))
        .content(content)
        .tooltip(tooltip)
}

/// **글자 버튼** — 아이콘 + 라벨. 아이콘만으로 뜻이 서지 않을 때만 쓴다(예: Retry).
pub fn label_button(label: &str, intent: Intent, sink: &IntentSink) -> View {
    let sink = Rc::clone(sink);
    let content = super::row(
        TOKENS.gap,
        vec![
            SymbolIcon::new().symbol(symbol(intent)).into(),
            super::label(label, TOKENS.body, FontWeight::NORMAL).into(),
        ],
    );
    Button::new()
        .style(ButtonStyle::Subtle)
        .height(TOKENS.control_h)
        .automation_name(label)
        .on_click(move || sink(intent))
        .content(content)
        .tooltip(label)
}

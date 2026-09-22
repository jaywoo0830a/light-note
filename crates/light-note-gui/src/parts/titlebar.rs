//! 화면 조각 — **네이티브 타이틀바**: 문서 제목 · 배경 요약 · 배지.
//!
//! ## 왜 손수 만든 앱바를 버렸는가
//! 리액터는 `TitleBar` 요소를 만나면 **그것을 창의 타이틀바로 붙인다** — 네이티브 구현이
//! `Window.SetExtendsContentIntoTitleBar(true)` + `Window.SetTitleBar(element)` +
//! `AppWindowTitleBar.SetPreferredHeightOption(...)`을 대신 불러 준다
//! (`windows-reactor`의 `native/winui/mod.rs`). 그래서 이 조각 하나로 창의 **네이티브 크롬**이
//! 바뀐다:
//!
//! - 캡션 버튼(최소화·최대화·닫기)이 이 띠 **안으로** 들어온다(우리가 그리지 않는다).
//! - Mica 배경이 타이틀바까지 이어진다(창 배경은 `app.rs`의 `WindowVisuals`).
//! - 띠 전체가 **드래그 영역**이고, 슬롯에 넣은 내용은 클릭이 살아 있다.
//! - 높이는 프리셋(`WindowTitleBarHeight::Tall`) — [`TOKENS.titlebar_h`]가 그 값(48 DIP)이다.
//!
//! 그래서 이전의 `Border` 두 줄 앱바는 없어졌다: 제목·부제는 컨트롤의 **속성**이 되고,
//! 배지는 오른쪽 슬롯으로 간다(오른쪽 슬롯은 캡션 버튼을 침범하지 않는 자리다).
//!
//! ## 슬롯 두 개만 쓴다
//! - `Content` — 가운데: 앱 이름(액센트 바 + `light-note`). 장식은 여기까지다.
//! - `RightHeader` — 오른쪽: **한 화면에 하나뿐인 사실**만 배지로.
//!
//! 나머지 화면(툴바·레일·상태 띠)은 그대로다 — 바뀌는 것은 **창의 크롬**뿐이다.

use elm_magic_windows_reactor::RawSlot;
use windows_reactor::{
    Border, CornerRadius, FontWeight, LayoutControl, SlotView, SlotsControl, TitleBar,
    TitleBarSlot, VerticalAlignment, View, WindowTitleBarHeight,
};

use super::{accent_brush, chip, chip_accent, label, row};
use crate::style::TOKENS;
use crate::ui::ViewModel;

/// 네이티브 타이틀바 하나.
pub fn titlebar(view: &ViewModel) -> RawSlot {
    // 앱의 시작을 표시하는 유일한 장식 — 왼쪽 액센트 바.
    let brand: View = Border::new()
        .width(3.0)
        .height(TOKENS.body + TOKENS.tight)
        .background(accent_brush())
        .corner_radius(CornerRadius::uniform(TOKENS.pill))
        .vertical_alignment(VerticalAlignment::Center)
        .into();
    let name = row(
        TOKENS.gap,
        vec![
            brand,
            label("light-note", TOKENS.body, FontWeight::SEMI_BOLD).into(),
        ],
    );

    // 오른쪽: 한 화면에 하나뿐인 사실만 담는다(배지가 늘면 타이틀바가 시끄러워진다).
    let mut badges: Vec<View> = vec![
        chip(format!("Page {} / {}", view.page + 1, view.page_count)),
        chip(format!("Zoom {}", view.zoom_label())),
        chip(view.input.clone()),
    ];
    if view.dirty {
        badges.push(chip_accent("Unsaved"));
    }

    Some(
        TitleBar::new()
            .preferred_height(WindowTitleBarHeight::Tall)
            .title(view.title.clone())
            .subtitle(background_line(view))
            .slots([
                SlotView::new(TitleBarSlot::Content, name),
                SlotView::new(TitleBarSlot::RightHeader, row(TOKENS.tight, badges)),
            ]),
    )
}

/// 부제 한 줄 — 배경 PDF와 이 페이지의 획 수(**영어만**).
fn background_line(view: &ViewModel) -> String {
    let background = if view.pdf_name.is_empty() {
        "Blank note — no background PDF".to_string()
    } else {
        format!("Background: {}", view.pdf_name)
    };
    format!("{background} · {} strokes on this page", view.stroke_count)
}

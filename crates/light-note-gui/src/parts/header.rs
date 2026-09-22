//! 화면 조각 — **앱바**: 앱 이름 · 문서 제목 · 배지.
//!
//! 배치는 `Grid` 한 줄이다: `Star` 열이 가운데를 비워 오른쪽 배지 묶음을 밀어낸다(예제 27).
//! 계층은 **크기와 굵기**로 만들고(예제 22), 색은 테마 브러시 이름만 쓴다.
//! 제목 왼쪽의 액센트 바는 "앱의 시작"을 표시하는 유일한 장식이다.

use elm_magic_windows_reactor::RawSlot;
use windows_reactor::{
    Border, ContentControl, CornerRadius, FontWeight, LayoutControl, Thickness, VerticalAlignment,
    View,
};

use super::{accent_brush, chip, chip_accent, column, label, meta, row, stroke_brush};
use crate::style::TOKENS;
use crate::ui::ViewModel;

/// 앱바 하나.
pub fn header(view: &ViewModel) -> RawSlot {
    // 왼쪽: 액센트 바 + (앱 이름 / 문서 제목 / 배경 요약).
    let brand: View = Border::new()
        .width(3.0)
        .height(TOKENS.display + TOKENS.block)
        .background(accent_brush())
        .corner_radius(CornerRadius::uniform(TOKENS.pill))
        .vertical_alignment(VerticalAlignment::Center)
        .into();
    let titles = column(
        TOKENS.tight,
        vec![
            meta("light-note").into(),
            label(view.title.clone(), TOKENS.display, FontWeight::BOLD).into(),
            meta(background_line(view)).into(),
        ],
    );

    // 오른쪽: 배지 — 한 화면에 하나뿐인 사실만 담는다.
    let mut badges: Vec<View> = vec![
        chip(format!("Page {} / {}", view.page + 1, view.page_count)),
        chip(format!("Zoom {}", view.zoom_label())),
        chip(view.input.clone()),
    ];
    if view.dirty {
        badges.push(chip_accent("Unsaved"));
    }

    // 앱바 = **두 줄**: (액센트 바 + 제목 블록) / 배지 줄.
    // Grid의 열 정의·RelativePanel 정렬은 이 백엔드에서 폭을 못 받아 자식이 사라지거나
    // 화면 밖으로 나가므로(`parts::row`의 문서 참고) StackPanel 두 줄로만 짠다.
    let body = column(
        TOKENS.gap,
        vec![
            row(TOKENS.block, vec![brand, titles]),
            row(TOKENS.tight, badges),
        ],
    );

    Some(
        Border::new()
            .background(super::surface_brush())
            .border_brush(stroke_brush())
            .border_thickness(Thickness::new(0.0, 0.0, 0.0, 1.0))
            .padding(Thickness::xy(TOKENS.pad, TOKENS.block))
            .content(body),
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

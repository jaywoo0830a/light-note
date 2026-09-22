//! 화면 조각 — **여는 중**: 스피너 + 한 줄.
//!
//! 오래 걸리는 일(PDF 읽기·파싱)은 워커가 한다 — 화면은 "기다리는 중"이라는 사실만
//! 보여주고 UI 스레드는 멈추지 않는다(R2).

use elm_magic_windows_reactor::RawSlot;
use windows_reactor::{
    Border, ContentControl, CornerRadius, FontWeight, LayoutControl, ProgressRing, Thickness, View,
};

use super::{column, label, meta, row, stroke_brush, surface_brush};
use crate::style::TOKENS;

/// 여는 중 카드 하나.
pub fn loading() -> RawSlot {
    let spinner: View = ProgressRing::new()
        .is_active(true)
        .is_indeterminate(true)
        .width(TOKENS.control_h / 2.0)
        .height(TOKENS.control_h / 2.0)
        .into();
    let body = row(
        TOKENS.block,
        vec![
            spinner,
            column(
                TOKENS.tight,
                vec![
                    label("Opening PDF…", TOKENS.body, FontWeight::SEMI_BOLD).into(),
                    meta("Reading and parsing on a worker thread").into(),
                ],
            ),
        ],
    );

    Some(
        Border::new()
            .background(surface_brush())
            .border_brush(stroke_brush())
            .border_thickness(Thickness::uniform(1.0))
            .corner_radius(CornerRadius::uniform(TOKENS.radius))
            .padding(Thickness::uniform(TOKENS.pad))
            .margin(Thickness::uniform(TOKENS.tight))
            .content(body),
    )
}

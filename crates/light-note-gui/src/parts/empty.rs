//! 화면 조각 — **빈 상태 안내**: 종이 위에서 무엇을 하면 되는지.
//!
//! 이 조각은 `<Raw>` 슬롯이 아니라 **잉크 표면 안**에 들어간다([`crate::render::surface`]):
//! 밖에 두면 잉크 영역의 높이가 단계마다 달라져 크롬(상태바)이 화면 밖으로 밀린다.
//!
//! 필기가 안 되는 이유(펜이 감지되지 않음 / 훅이 안 걸림)는 호스트가 만든 문구
//! ([`ViewModel::input`])를 **그대로** 보여준다 — 조각은 문장을 만들지 않는다.

use windows_reactor::{
    Border, ContentControl, CornerRadius, FontWeight, LayoutControl, Symbol, SymbolIcon, Thickness,
    View,
};

use super::{column, label, meta, row, stroke_brush, surface_brush};
use crate::style::TOKENS;
use crate::ui::ViewModel;

/// 안내 카드 하나.
pub fn empty(view: &ViewModel) -> View {
    let icon: View = SymbolIcon::new().symbol(Symbol::Edit).into();
    let body = row(
        TOKENS.block,
        vec![
            icon,
            column(
                TOKENS.tight,
                vec![
                    label("Draw here with a pen", TOKENS.title, FontWeight::SEMI_BOLD).into(),
                    meta("Finger and mouse do not ink — this app inks with a digitizer pen").into(),
                    meta(view.input.clone()).into(),
                ],
            ),
        ],
    );

    Border::new()
        .background(surface_brush())
        .border_brush(stroke_brush())
        .border_thickness(Thickness::uniform(1.0))
        .corner_radius(CornerRadius::uniform(TOKENS.radius))
        .padding(Thickness::uniform(TOKENS.pad))
        .margin(Thickness::uniform(TOKENS.tight))
        .content(body)
}

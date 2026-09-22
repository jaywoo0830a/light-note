//! 화면 조각 — **빈 상태 안내**: 종이 위에서 무엇을 하면 되는지.
//!
//! 이 조각은 `<Raw>` 슬롯이 아니라 **잉크 표면 안**에 들어간다([`crate::render::surface`]):
//! 밖에 두면 잉크 영역의 높이가 단계마다 달라져 크롬(정보 띠)이 화면 밖으로 밀린다.
//!
//! 카드 폭은 **창 폭에서 레일과 여백을 뺀 값**으로 못 박고 글자는 여러 줄로 감는다 —
//! 안 그러면 좁은 창에서 안내가 화면 밖으로 잘린다(실제로 겪었다).
//!
//! 모서리는 **큰 면의 눈금**([`TOKENS.sheet`] = 16×2 DIP)을 쓴다: 이 카드는 종이와 거의
//! 같은 폭을 차지하므로 종이와 같은 곡률이어야 같은 가족으로 보인다.
//!
//! 필기가 안 되는 이유(펜이 감지되지 않음 / 훅이 안 걸림)는 호스트가 만든 문구
//! ([`ViewModel::input`])를 **그대로** 보여준다 — 조각은 문장을 만들지 않는다.

use windows_reactor::{
    Border, ContentControl, CornerRadius, FontWeight, LayoutControl, Symbol, SymbolIcon, Thickness,
    View,
};

use super::{column, label, meta_wrapped, row, stroke_brush, surface_brush};
use crate::style::TOKENS;
use crate::ui::ViewModel;

/// 안내 카드 하나.
pub fn empty(view: &ViewModel) -> View {
    let width =
        (view.viewport.0 - TOKENS.rail - TOKENS.page * 2.0 - TOKENS.pad * 2.0).max(TOKENS.rail);
    let icon: View = SymbolIcon::new().symbol(Symbol::Edit).into();
    let body = row(
        TOKENS.block,
        vec![
            icon,
            column(
                TOKENS.tight,
                vec![
                    label("Draw here with a pen", TOKENS.title, FontWeight::SEMI_BOLD).into(),
                    meta_wrapped(
                        "Finger and mouse do not ink — this app inks with a digitizer pen",
                    )
                    .into(),
                    meta_wrapped(view.input.clone()).into(),
                ],
            ),
        ],
    );

    Border::new()
        .width(width)
        .background(surface_brush())
        .border_brush(stroke_brush())
        .border_thickness(Thickness::uniform(1.0))
        .corner_radius(CornerRadius::uniform(TOKENS.sheet))
        .padding(Thickness::uniform(TOKENS.pad))
        .margin(Thickness::uniform(TOKENS.tight))
        .content(body)
}

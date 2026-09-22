//! 화면 조각 — **잉크 종이**와 그 둘레.
//!
//! 모양과 동작을 나눈다: 겉모습(종이·책상)은 여기, 포인터 이벤트(`on_pointer_*`)와
//! 내용(캔버스)은 [`crate::render::surface`]가 같은 테두리에 붙인다. 그래서 "종이가
//! 어떻게 보이는가"를 바꿀 때 입력 경로를 건드릴 일이 없다.
//!
//! `content()`가 `View`를 돌려주므로(리액터의 사실) `frame()`은 **뼈대**만 만든다 —
//! 내용은 render가 마지막에 넣는다.
//!
//! 종이는 테마를 따라간다(`SolidBackground`) — PDF 배경이 없어도 종이로 보이고,
//! 다크 테마에서도 글자가 읽힌다. 반지름은 토큰에서 온다(컨트롤마다 다르지 않다).
//!
//! ## 왜 스크롤 상자에 **높이를 못 박는가**
//! A4 @ 150%는 893×1263px이라 창보다 크다 — 스크롤로 닿아야 정상이다. 그런데 elm 트리는
//! `<Col>`/`<Row>` = `StackPanel`뿐이고(어댑터의 계획 표), `StackPanel`은 자식을 **무한
//! 높이**로 측정한다. 그래서 안쪽 `ScrollViewer`는 자기 내용만큼 커져 스크롤이 생기지
//! 않고, 크롬(상태바)이 화면 밖으로 밀린다. 그래서 **호스트가 창 크기를 관측해**
//! (`ViewContext::on_window_size` → `ui::ViewModel::viewport`) 높이를 내려준다 —
//! 그제야 스크롤이 생기고 크롬이 창 안에 남는다.

use windows_reactor::{
    Border, Brush, ContentControl, CornerRadius, LayoutControl, ScrollBarVisibility, ScrollViewer,
    ThemeBrush, Thickness, View,
};

use crate::style::TOKENS;

/// 종이의 뼈대 — 내용과 이벤트는 부르는 쪽이 붙인다.
pub fn frame() -> Border {
    Border::new()
        .background(Brush::from(ThemeBrush::SolidBackground))
        .border_brush(Brush::from(ThemeBrush::CardStroke))
        .border_thickness(Thickness::uniform(1.0))
        .corner_radius(CornerRadius::uniform(TOKENS.sheet))
        .padding(Thickness::uniform(0.0))
}

/// **책상** — 종이 둘레의 여백(예제 23의 리듬: `page` = 24 DIP).
pub fn desk(content: impl Into<View>) -> View {
    Border::new()
        .padding(Thickness::uniform(TOKENS.page))
        .content(content)
}

/// **스크롤 상자** — 페이지가 창보다 클 때 종이에 닿는 유일한 길(예제 28).
///
/// 높이는 [`super::content_height`](창 높이 − 크롬)이고, 창이 아주 작아도
/// [`TOKENS.content_min`]만큼은 남긴다.
pub fn scroll(content: impl Into<View>, height: f64) -> View {
    ScrollViewer::new()
        .height(height)
        .horizontal_scroll_bar_visibility(ScrollBarVisibility::Auto)
        .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
        .content(content)
}

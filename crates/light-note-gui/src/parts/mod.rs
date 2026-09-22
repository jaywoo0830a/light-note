//! ④-UI 화면 조각 — **조각마다 파일 하나, 함수 하나**.
//!
//! ## 왜 파일을 나누는가
//! 이 백엔드는 `css!`/`class`를 읽지 않는다(예제 16). 스타일을 만드는 통로는 `<Raw>`
//! **하나뿐**이고, `<Raw>` 본문은 매크로가 그대로 복사하므로 그 안에서 elm 상태를 읽을
//! 수도 없다(예제 21). 그래서 조각을 한 파일에 몰아넣으면 "무엇이 어디서 만들어지는가"가
//! 금방 흐려진다 — 조각마다 파일 하나로 두고, 각 파일이 **함수 하나**만 공개한다.
//!
//! | 조각 | 파일 | 함수 | elm 쪽 |
//! |---|---|---|---|
//! | 타이틀바(네이티브) | `titlebar` | `titlebar::titlebar` | `<TitleBar />` |
//! | 툴바(버튼 + 미리보기) | `toolbar` | `toolbar::toolbar` | `<Toolbar />` |
//! | 좌측 레일 | `rail` | `rail::rail` | `<Rail />` |
//! | 상태바 | `status` | `status::status` | `<Status />` |
//! | 빈 상태 안내 | `empty` | `empty::empty` | `<EmptyHint />` |
//! | 여는 중 | `loading` | `loading::loading` | `<Loading />` |
//! | 실패 | `failure` | `failure::failure` | `<Failure />` |
//! | 단축키 패널 | `shortcuts` | `shortcuts::shortcuts` | `<Shortcuts />` |
//! | 잉크 종이 | `paper` | `paper::frame` | `<InkSurface />`(render가 부른다) |
//! | 버튼 어휘 | `buttons` | `buttons::icon_button` | (조각들이 쓴다) |
//!
//! ## 버튼도 조각이 만든다
//! elm의 `<Button>`을 쓰지 않는다 — 스타일을 줄 통로가 `<Raw>`뿐이라 elm 위젯은 WinUI
//! 기본 모양으로 굳는다(예제 26). `<Raw>` 안에서 elm 콜백은 못 쓰지만, **호스트가 넘긴
//! [`crate::ui::IntentSink`]는 캡처할 수 있다** — 표면이 포인터 이벤트를 다루는 것과 같은
//! 방법이다. 그래서 버튼 하나는 `Button::new().on_click(|| sink(intent))`이고,
//! 그 어휘(아이콘·툴팁·스타일·자동화 이름)는 `buttons` 한 곳에 있다.
//!
//! ## 색 (예제 25)
//! 표면·선·글자는 전부 **테마 브러시 이름**이다(`ThemeBrush`) — 라이트/다크/고대비가
//! 자동으로 따라온다. 색을 새로 만들지 않는다: 활성 도구는 `ButtonStyle::Accent`가
//! 표시하고(대비는 WinUI가 보장한다), 잉크 미리보기만 도구 색을 그대로 쓴다.
//!
//! ## 자름 (예제 22)
//! 긴 문장은 **줄바꿈 금지 + 말줄임**을 짝으로 준다(`clipped`) — 안 주면 WinUI 기본값이
//! 컨트롤을 조용히 넘치거나 자른다.

pub mod buttons;
pub mod empty;
pub mod failure;
pub mod loading;
pub mod paper;
pub mod rail;
pub mod shortcuts;
pub mod status;
pub mod titlebar;
pub mod toolbar;

use elm_magic_windows_reactor::RawSlot;
use windows_reactor::{
    Border, Brush, ChildrenControl, Color, ContentControl, CornerRadius, FontWeight, KeyedView,
    LayoutControl, Orientation, StackPanel, TextBlock, TextTrimming, TextWrapping, ThemeBrush,
    Thickness, VerticalAlignment, View,
};

use crate::ink::Rgba;
use crate::style::TOKENS;
use crate::ui::{IntentSink, Part, ViewModel};

/// `<Raw>` 슬롯 하나를 채운다 — `app.rs`가 이 함수 **하나**를 등록한다.
///
/// 조각이 늘어도 호스트의 배선은 그대로다(분기는 여기 한 곳).
pub fn build(part: Part, view: &ViewModel, sink: &IntentSink) -> RawSlot {
    match part {
        Part::TitleBar => titlebar::titlebar(view),
        Part::Toolbar => toolbar::toolbar(view, sink),
        Part::Rail => rail::rail(view, sink),
        Part::Status => status::status(view),
        Part::Loading => loading::loading(),
        Part::Failure => failure::failure(view, sink),
        Part::Shortcuts => shortcuts::shortcuts(),
    }
}

// ── 색: 테마 브러시 **이름**만 쓴다 (예제 25) ──────────────────────

/// 카드·레일의 면.
pub(crate) fn surface_brush() -> Brush {
    Brush::from(ThemeBrush::CardBackground)
}

/// 창·띠의 면(카드보다 뒤에 있는 층).
pub(crate) fn chrome_brush() -> Brush {
    Brush::from(ThemeBrush::SolidBackground)
}

/// 테두리·구분선.
pub(crate) fn stroke_brush() -> Brush {
    Brush::from(ThemeBrush::CardStroke)
}

/// 본문 글자.
pub(crate) fn text_brush() -> Brush {
    Brush::from(ThemeBrush::PrimaryText)
}

/// 액센트(선택·강조).
pub(crate) fn accent_brush() -> Brush {
    Brush::from(ThemeBrush::Accent)
}

/// 액센트 **위의** 글자 — WinUI가 대비를 보장한다.
pub(crate) fn on_accent_brush() -> Brush {
    Brush::from(ThemeBrush::AccentText)
}

// ── 재료 ──────────────────────────────────────────────────────────

/// 카드 하나 — 면·선·반지름·여백을 **한 곳에서만** 정한다(예제 30의 `surface`).
pub(crate) fn card(pad: f64, content: impl Into<View>) -> View {
    card_border(pad, None).content(content)
}

/// **고정 폭·고정 높이** 카드 — 레일처럼 크기가 흔들리면 안 되는 곳(예제 27의 `Pixel`).
pub(crate) fn card_fixed(pad: f64, width: f64, height: f64, content: impl Into<View>) -> View {
    card_border(pad, Some((width, height))).content(content)
}

/// 카드의 **뼈대** — `content()`가 `View`를 돌려주므로(리액터의 사실) 내용은 나중에 넣는다.
fn card_border(pad: f64, size: Option<(f64, f64)>) -> Border {
    let border = Border::new()
        .background(surface_brush())
        .border_brush(stroke_brush())
        .border_thickness(Thickness::uniform(1.0))
        .corner_radius(CornerRadius::uniform(TOKENS.radius))
        .padding(Thickness::uniform(pad));
    match size {
        Some((width, height)) => border.width(width).height(height),
        None => border,
    }
}

/// **잉크 영역의 높이** — `창 높이 − 크롬`(추정). 레일과 종이가 **같은 값**을 쓴다.
///
/// 둘이 다른 높이를 가지면 elm의 `<Row>`(가로 `StackPanel`)가 **더 큰 쪽**을 따라 커지고,
/// 그러면 상태바가 화면 밖으로 밀린다(실제로 겪었다). 값의 근거는
/// [`crate::ui::ViewModel::viewport`] 하나뿐이다.
pub(crate) fn content_height(view: &ViewModel) -> f64 {
    (view.viewport.1 - TOKENS.chrome_h).max(TOKENS.content_min)
}

/// **위쪽 1px 선** — 앱바·툴바·상태바를 본문과 나눈다(예제 28의 머리글/바닥글).
pub(crate) fn hairline() -> View {
    Border::new()
        .border_brush(stroke_brush())
        .border_thickness(Thickness::new(0.0, 1.0, 0.0, 0.0))
        .into()
}

/// **세로 1px 선** — 툴바의 묶음 사이.
pub(crate) fn divider() -> View {
    Border::new()
        .width(1.0)
        .height(TOKENS.control_h / 2.0)
        .background(stroke_brush())
        .vertical_alignment(VerticalAlignment::Center)
        .into()
}

/// 글자 하나 — 크기와 굵기는 **호출자가 토큰에서 고른다**.
pub(crate) fn label(text: impl Into<String>, size: f64, weight: FontWeight) -> TextBlock {
    TextBlock::new()
        .text(text)
        .font_size(size)
        .font_weight(weight)
        .foreground(text_brush())
}

/// 보조 설명 — 색을 하나 더 만들지 않고 **투명도**로 낮춘다(예제 22).
pub(crate) fn meta(text: impl Into<String>) -> TextBlock {
    label(text, TOKENS.caption, FontWeight::NORMAL).opacity(0.7)
}

/// 보조 설명(줄바꿈 허용) — 좁은 창에서 잘리지 않게 **여러 줄로 감는다**.
pub(crate) fn meta_wrapped(text: impl Into<String>) -> TextBlock {
    meta(text).text_wrapping(TextWrapping::Wrap)
}

/// 한 줄로 **자르는** 글자 — 줄바꿈 금지 + 말줄임은 항상 짝이다(예제 22).
pub(crate) fn clipped(text: impl Into<String>, size: f64, weight: FontWeight) -> TextBlock {
    label(text, size, weight)
        .text_wrapping(TextWrapping::NoWrap)
        .text_trimming(TextTrimming::WordEllipsis)
        .max_lines(1)
}

/// 알약 배지 — 면·글자·선 색을 호출자가 정한다(색은 테마 브러시 이름으로만 온다).
pub(crate) fn pill(text: impl Into<String>, fill: Brush, ink: Brush, line: Brush) -> View {
    Border::new()
        .background(fill)
        .border_brush(line)
        .border_thickness(Thickness::uniform(1.0))
        .corner_radius(CornerRadius::uniform(TOKENS.pill))
        .padding(Thickness::xy(TOKENS.gap, TOKENS.tight))
        .content(label(text, TOKENS.caption, FontWeight::SEMI_BOLD).foreground(ink))
}

/// 중립 배지 — 표면 + 보조 테두리.
pub(crate) fn chip(text: impl Into<String>) -> View {
    pill(text, surface_brush(), text_brush(), stroke_brush())
}

/// 강조 배지 — 액센트 면 + 그 위의 글자.
pub(crate) fn chip_accent(text: impl Into<String>) -> View {
    pill(text, accent_brush(), on_accent_brush(), accent_brush())
}

/// **왼쪽 정렬 한 줄** — 이 백엔드에서 **믿을 수 있는 유일한 배치**는 `StackPanel`이다.
///
/// ## 왜 Grid/RelativePanel로 "양 끝"을 만들지 않는가
/// 둘 다 이 리액터 버전에서 폭을 못 받는다: `Grid`에 `columns()`를 주면 첫 열이 남은 폭을
/// 다 먹어(무한 폭으로 측정된다) 둘째 열이 화면 밖으로 나가고, 열 없이 한 셀에 겹쳐 놓고
/// `HorizontalAlignment::Right`를 주면 셀이 0폭이라 자식이 **왼쪽 바깥**으로 밀린다
/// (실제로 겪었다 — 앱바의 배지와 툴바의 미리보기가 통째로 안 보였다).
///
/// 그래서 배치는 **줄을 나누는 것**으로만 한다: 앱바는 (제목 줄 / 배지 줄),
/// 툴바는 (버튼 줄 / 미리보기 줄), 상태바는 (문장 줄 / 힌트·사실 줄) — 전부 왼쪽에서
/// 시작하는 흐름이라 폭과 무관하게 안전하다.
pub(crate) fn row(spacing: f64, children: Vec<View>) -> View {
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(spacing)
        .keyed_children(keys(children))
}

/// 세로 묶음 — 줄과 줄 사이의 간격은 **항상** 준다(예제 23).
pub(crate) fn column(spacing: f64, children: Vec<View>) -> View {
    StackPanel::new()
        .spacing(spacing)
        .keyed_children(keys(children))
}

/// 목록을 위치 키로 넘긴다 — 개수가 변하는 목록의 기본형(예제 28).
fn keys(children: Vec<View>) -> Vec<KeyedView> {
    children
        .into_iter()
        .enumerate()
        .map(|(index, view)| KeyedView::new(index as u64, view))
        .collect()
}

/// 잉크 색 → WinUI `Color`(알파는 호출자가 `opacity`로 옮긴다).
pub(crate) fn rgb(color: Rgba) -> Color {
    let (red, green, blue) = color.to_rgb();
    Color::rgb(red, green, blue)
}

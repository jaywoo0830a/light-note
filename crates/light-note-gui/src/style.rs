//! ④-UI 디자인 토큰 — 예제 21~30이 정리한 방식을 이 앱의 언어로 옮긴 곳.
//!
//! ## 이 파일이 지키는 경계
//! - **플랫폼을 모른다**: WinUI 타입(`Color`/`Brush`/`Thickness`/`CornerRadius`)이
//!   하나도 없다. 그래서 토큰 규칙(리듬·스케일)은 헤드리스 테스트가 그대로 검증한다.
//! - 토큰 → WinUI `View`로 옮기는 일은 [`crate::parts`]가 한다(조각마다 파일 하나).
//!
//! ## 두 층 (예제 21)
//! 1. **기하 토큰** — 테마와 무관한 고정값(반지름·간격·글자 크기·고정 폭).
//!    라이트/다크가 바뀌어도 값이 바뀌지 않는다(레이아웃이 흔들리면 안 된다).
//! 2. **색** — 표면·선·글자는 여기서 정하지 않는다. **테마 브러시 이름**
//!    (`CardBackground`/`CardStroke`/`SolidBackground`/`PrimaryText`/`Accent`/
//!    `AccentText`/`SystemCritical`/`SystemCriticalBackground`)을 [`crate::parts`]가
//!    고르므로 라이트/다크/고대비가 공짜로 따라온다(예제 25). 브랜드 색을 새로
//!    만들면 테마를 따라가지 **않으므로** 만들지 않았다 — 이 앱은 시스템 색으로 충분하다.
//!
//! ## 폰트 — Google Sans Flex를 지금 쓸 수 없는 이유 (확인한 사실)
//! - Google Fonts의 `<link rel="stylesheet">`는 **HTML/CSS 기법**이다. 이 앱은
//!   네이티브 WinUI라서 CSS도 `<link>`도 없고, 그 태그를 넣을 자리가 아예 없다.
//! - WinUI에서 글꼴은 `TextBlock.FontFamily`로 정한다. 그런데 `windows-reactor`
//!   0.100의 `TextBlock`은 `font_size` / `font_weight` / `foreground` /
//!   `text_wrapping` / `max_lines` / `text_trimming` /
//!   `is_text_selection_enabled`만 노출하고 **`FontFamily`는 노출하지 않는다**
//!   (`generated.rs`의 프로퍼티 목록에 없다 — 예제 22의 결론과 같다).
//!   `ResourceOverrides`도 `Color`/`Thickness`/`CornerRadius`만 받는다(예제 21).
//! - 그래서 **어댑터의 `<Raw>`로도 글꼴 패밀리는 바꿀 수 없다**. 폰트 파일을 설치해도
//!   이름을 지정할 통로가 없으면 적용되지 않는다. 지금은 WinUI 기본 글꼴
//!   (Windows 11의 `Segoe UI Variable`)을 쓰고, "타이포 시스템"은 예제 22대로
//!   **크기 · 굵기 · 색 · 자름** 네 축으로 만든다([`Tokens`]의 `title`/`subtitle`/
//!   `body`/`caption` + `parts`의 `FontWeight` + `TextTrimming`).
//! - 실제로 적용하려면 **업스트림 변경**이 필요하다: `windows-reactor`의 `TextBlock`에
//!   `font_family` 프로퍼티를 추가하고(`PropertyId`/`PropertyValue`/`native/winui`
//!   매핑까지 함께), 그 뒤 `FontFamily("Google Sans Flex")`를 넘긴다. 폰트가 시스템에
//!   설치돼 있어야 이름이 해석되며, 폰트 파일 배포는 OFL 1.1 조건을 따른다.
//!   [`Tokens::FONT_FAMILY`]에 그 이름을 적어 두었다 — 통로가 생기면 한 곳만 고치면 된다.
//!
//! ## 규칙 (테스트가 고정한다)
//! - 모든 간격은 **4 DIP의 배수**다(예제 23의 리듬) — `tests/style.rs`가 확인한다.
//! - 글자 크기는 내림차순이다(`title > subtitle > body > caption`) — 예제 22.
//! - 화면 코드(`parts`/`ui`)에는 숫자가 없다 — 전부 토큰 이름이다.

/// 화면이 쓰는 기하·타이포 토큰. **값은 여기 한 곳에만 있다.**
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// 카드·표면 모서리 반지름 (DIP).
    pub radius: f64,
    /// 컨트롤(버튼·배지) 모서리 반지름 (DIP).
    pub control: f64,
    /// 알약(pill) 반지름 — "높이보다 큰 값"이라는 관례를 토큰으로 둔다(예제 24).
    pub pill: f64,
    /// 같은 묶음 안의 간격 (DIP).
    pub tight: f64,
    /// 요소 사이 간격 (DIP) — 툴바 그룹 안, 배지 사이.
    pub gap: f64,
    /// 묶음 사이 간격 (DIP) — 제목과 부제 사이.
    pub block: f64,
    /// 표면 **안쪽** 여백 (DIP) — 앱바·툴바·레일·상태바.
    pub pad: f64,
    /// 종이 둘레의 책상 여백 (DIP) — 페이지가 창보다 클 때의 호흡.
    pub page: f64,
    /// 문서 제목 글자 크기 (DIP) — 타이포 4단계(예제 22).
    pub display: f64,
    /// 섹션 제목 글자 크기 (DIP).
    pub title: f64,
    /// 본문 글자 크기 (DIP).
    pub body: f64,
    /// 보조 설명 글자 크기 (DIP).
    pub caption: f64,
    /// 좌측 레일 고정 폭 (DIP) — 예제 27의 `Pixel`.
    pub rail: f64,
    /// 아이콘 버튼 한 변 (DIP) — 정사각.
    pub control_h: f64,
    /// 잉크 미리보기 폭 (DIP).
    pub preview_w: f64,
    /// 잉크 미리보기 높이 (DIP).
    pub preview_h: f64,
    /// 미리보기 선의 **최대** 굵기 (DIP) — 상자 높이를 넘지 않게 자른다.
    pub line_max: f64,
    /// 상태바 높이 (DIP).
    pub status_h: f64,
    /// 크롬(앱바 + 툴바 + 정보 띠)의 대략 높이 (DIP).
    ///
    /// **추정값**이다: 잉크 영역이 창 안에 들어오게 하려면 "창 높이 − 크롬"이 필요한데,
    /// 크롬 높이는 컨트롤이 스스로 정하므로 여기서 어림한다(레이아웃이 바뀌면 이 값도 바꾼다).
    /// 실제로 재려면 호스트가 `ElementRef::observe_surface`로 크롬의 크기를 관측해야 한다.
    ///
    /// 이 값이 틀려도 **정보 띠는 창 안에 남는다**(크롬은 본문 위에 있다) — 잘리는 것은
    /// 본문(종이)의 아래쪽뿐이다.
    pub chrome_h: f64,
    /// 잉크 영역의 **최소** 높이 (DIP) — 창이 아주 작아도 종이가 사라지지 않게.
    pub content_min: f64,
}

impl Tokens {
    /// 글꼴 패밀리 이름 — **지금은 쓰이지 않는다**(모듈 문서의 이유 참고).
    ///
    /// `windows-reactor`가 `FontFamily`를 노출하면 [`crate::parts`]가 이 값을 쓴다.
    pub const FONT_FAMILY: &'static str = "Google Sans Flex";

    /// 간격 단위 — 모든 간격이 이 값의 배수다.
    pub const UNIT: f64 = 4.0;

    /// 타이포 4단계 — 내림차순이어야 계층이 읽힌다(예제 22).
    pub fn sizes(&self) -> [f64; 4] {
        [self.display, self.title, self.body, self.caption]
    }

    /// 간격 5단계 — 리듬의 정의를 코드로 옮긴 것(예제 23).
    pub fn steps(&self) -> [f64; 5] {
        [self.tight, self.gap, self.block, self.pad, self.page]
    }

    /// 모든 간격이 단위의 배수인가 — 테스트가 이 규칙을 고정한다.
    pub fn is_rhythmic(&self) -> bool {
        self.steps()
            .iter()
            .all(|step| (step / Self::UNIT).fract() == 0.0)
    }
}

/// 이 앱의 토큰 — 화면 코드는 이 값만 본다(예제 30의 `guide()`와 같은 역할).
///
/// 리듬: 4 / 8 / 12 / 16 / 24 DIP. 타이포: 26 / 15 / 13 / 11 DIP.
/// 컨트롤: 아이콘 버튼 32×32 DIP, 레일 248 DIP, 상태바 28 DIP.
pub const TOKENS: Tokens = Tokens {
    radius: 8.0,
    control: 4.0,
    pill: 999.0,
    tight: 4.0,
    gap: 8.0,
    block: 12.0,
    pad: 16.0,
    page: 24.0,
    display: 26.0,
    title: 15.0,
    body: 13.0,
    caption: 11.0,
    rail: 248.0,
    control_h: 32.0,
    preview_w: 200.0,
    preview_h: 28.0,
    line_max: 20.0,
    status_h: 28.0,
    chrome_h: 480.0,
    content_min: 240.0,
};

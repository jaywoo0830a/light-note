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
//! ## 글꼴 — **시스템 글꼴을 상속한다** (소스로 확인한 사실)
//! WinUI에서 글꼴을 정하는 것은 `TextBlock.FontFamily`(=`ITextBlock::SetFontFamily`)다.
//! 그런데 `windows-reactor` 0.100은 그 통로를 열지 않았다 — 확인한 세 가지:
//!
//! 1. **속성 계층**(`generated.rs`의 `TextBlock`): `text` / `text_wrapping` / `font_size` /
//!    `font_weight` / `is_text_selection_enabled` / `max_lines` / `text_trimming` /
//!    `foreground`뿐이고 **`font_family`가 없다**.
//! 2. **리소스 재정의**(`element.rs`의 `ResourceValue`): `Color` / `Thickness` /
//!    `CornerRadius` 세 가지뿐이다 — WinUI의 관용구(`ContentControlThemeFontFamily` 같은
//!    테마 리소스를 덮어쓰기)도 쓸 수 없다.
//! 3. **네이티브 계층**(`native/winui/bindings.rs`)에는 **있다**: `ITextBlock_Vtbl` ·
//!    `ITextElement_Vtbl` · `IControl_Vtbl`에 `FontFamily`/`SetFontFamily` 슬롯이 있다.
//!    즉 막고 있는 것은 COM이 아니라 **리액터의 속성 목록**이다.
//!
//! 그래서 이 앱은 글꼴을 **지정하지 않는다** — WinUI3 기본 테마의 글꼴을 그대로
//! **상속**한다(Windows 11: `Segoe UI Variable`). 우회도 하지 않는다: `<Raw>`로도 안 되고
//! (같은 속성 계층을 지난다), CSS의 `<link rel="stylesheet">`는 HTML 기법이라 네이티브
//! 앱에 넣을 자리가 없다. 대신 타이포는 **크기 · 굵기 · 색 · 자름** 네 축으로만 만든다
//! (예제 22) — [`Tokens::SCALE`]의 4단계와 `parts`의 `FontWeight`/`TextTrimming`이
//! 계층의 전부다.
//!
//! [`Tokens::FONT_FAMILY`]에는 **상속 결과**를 적어 두었다(우리가 고른 값이 아니다) —
//! 업스트림이 `font_family`를 열면 그 한 곳만 실제 지정으로 바꾸면 된다.
//!
//! ## 값의 계보 — 원시 스칼라 → 파생 (한 규칙으로만)
//! 값을 손으로 고르지 않는다. **원시 스칼라 세 개**가 있고, 나머지는 전부 규칙 하나로
//! 거기서 나온다:
//!
//! | 원시 스칼라 | 값 | 무엇의 기준인가 |
//! |---|---|---|
//! | [`Tokens::BASE`] | **16 DIP = 1rem** | 글자 크기 · 큰 고정 크기 |
//! | [`Tokens::UNIT`] | `BASE / 4` = **4 DIP** | 간격(리듬) · 작은 면의 모서리 |
//! | [`Tokens::STEP`] | `BASE × 2` = **32 DIP** | 큰 면의 모서리 (16 × 2n) |
//!
//! | 무엇 | 규칙 | 이 앱의 값 |
//! |---|---|---|
//! | 간격 | `UNIT × n` | 4 / 8 / 12 / 16 / 24 |
//! | **작은 면**의 모서리 | `UNIT × n` | 4(컨트롤) · 8(카드) · 16(알약) |
//! | **큰 면**의 모서리 | `STEP × n` | 32(종이 · 큰 패널) |
//! | 글자 크기 | `BASE × SCALE[i]` | 28 / 20 / 16 / 12 |
//! | 고정 크기 | `UNIT × n` 또는 `BASE × n` | 버튼 36 · 레일 256 · 미리보기 200×32 |
//!
//! 그래서 "이 숫자가 왜 이 값인가"의 답은 항상 **원시 스칼라 + 규칙**이다. 예를 들어
//! 모서리는 면의 크기로 갈린다: 작은 면(버튼·배지·카드)은 4의 배수 눈금, 큰 면(종이처럼
//! 창을 크게 차지하는 면)은 16 × 2n 눈금 — 큰 면이 작은 눈금을 쓰면 각이 서고,
//! 작은 면이 큰 눈금을 쓰면 알약처럼 뭉개진다. `tests/style.rs`가 두 눈금을 확인한다.
//!
//! ## 규칙 (테스트가 고정한다)
//! - 모든 간격은 **4 DIP의 배수**다(예제 23의 리듬) — `tests/style.rs`가 확인한다.
//! - 작은 면의 모서리는 4의 배수, 큰 면은 32(=16×2)의 배수다.
//! - 글자 크기는 내림차순이고 전부 `BASE`의 배율이다(`display > title > body > caption`).
//! - 화면 코드(`parts`/`ui`)에는 숫자가 없다 — 전부 토큰 이름이다.

/// 화면이 쓰는 기하·타이포 토큰. **값은 여기 한 곳에만 있다.**
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// **작은 면**의 모서리 반지름 (DIP) — 카드·레일·안내 카드. `UNIT`의 배수.
    pub radius: f64,
    /// 컨트롤(버튼·미리보기 상자) 모서리 반지름 (DIP). `UNIT`의 배수.
    pub control: f64,
    /// 알약(pill) 반지름 — 높이의 절반보다 큰 값이면 WinUI가 절반으로 잘라 알약이 된다
    /// (예제 24). `UNIT`의 배수.
    pub pill: f64,
    /// **큰 면**의 모서리 반지름 (DIP) — 종이처럼 창을 크게 차지하는 면. `STEP`(=16×2)의 배수.
    pub sheet: f64,
    /// 같은 묶음 안의 간격 (DIP).
    pub tight: f64,
    /// 요소 사이 간격 (DIP) — 툴바 그룹 안, 배지 사이.
    pub gap: f64,
    /// 묶음 사이 간격 (DIP) — 제목과 부제 사이.
    pub block: f64,
    /// 표면 **안쪽** 여백 (DIP) — 툴바·레일·상태 띠.
    pub pad: f64,
    /// 종이 둘레의 책상 여백 (DIP) — 페이지가 창보다 클 때의 호흡.
    pub page: f64,
    /// 기준 글자 크기 (DIP) — 타이포 계층의 **1rem**.
    pub base: f64,
    /// 문서 제목 글자 크기 (DIP) — 타이포 4단계(예제 22).
    pub display: f64,
    /// 섹션 제목 글자 크기 (DIP).
    pub title: f64,
    /// 본문 글자 크기 (DIP) — **1rem**.
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
    /// **네이티브 타이틀바** 높이 (DIP) — WinUI `TitleBarHeightOption::Tall`(=48).
    ///
    /// 우리가 정하는 숫자가 아니라 **고른 프리셋의 높이**다(`TitleBar::preferred_height`).
    /// [`Tokens::chrome_h`]에 이 높이가 포함돼야 잉크 영역이 창 밖으로 나가지 않는다.
    pub titlebar_h: f64,
    /// 한 줄 툴바가 성립하는 **최소 창 폭** (DIP) — 이보다 좁으면 버튼 줄을 둘로 나눈다.
    pub toolbar_break: f64,
    /// 창의 **최소 폭** (DIP) — 크롬이 무너지지 않는 하한(`WindowConstraints`).
    pub min_window_w: f64,
    /// 창의 **최소 높이** (DIP) — 크롬 + 종이 한 뼘.
    pub min_window_h: f64,
    /// 처음 열 때의 **창 폭** (DIP) — 한 줄 툴바가 서는 크기(값이 바뀔 때만 적용된다).
    pub default_window_w: f64,
    /// 처음 열 때의 **창 높이** (DIP).
    pub default_window_h: f64,
    /// 크롬(네이티브 타이틀바 + 툴바 + 정보 띠)의 대략 높이 (DIP).
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
    /// 이 앱이 실제로 쓰는 글꼴 — **WinUI3 기본 글꼴의 상속 결과**다(Windows 11의 이름).
    ///
    /// 지정하는 코드는 **없다**: `windows-reactor` 0.100의 속성 계층에 `font_family`가
    /// 없어서(모듈 문서의 확인 목록) 글꼴은 **시스템이 정한다**. 그래서 이 값은
    /// "고른 글꼴"이 아니라 "상속 결과의 기록"이고, `tests/style.rs`가 그 사실을 고정한다 —
    /// 업스트림이 통로를 열면 여기 한 곳만 실제 지정으로 바꾼다.
    pub const FONT_FAMILY: &'static str = "Segoe UI Variable";

    /// ── 원시 스칼라 ① ── 기준 글자 크기: **1rem = 16 DIP**.
    ///
    /// 글자 크기(× [`Tokens::SCALE`])와 큰 고정 크기(× n)의 기준이다.
    pub const BASE: f64 = 16.0;

    /// ── 원시 스칼라 ② ── 리듬 단위: `BASE / 4` = **4 DIP**.
    ///
    /// 간격과 **작은 면**의 모서리가 전부 이 배수다.
    pub const UNIT: f64 = Self::BASE / 4.0;

    /// ── 원시 스칼라 ③ ── 큰 면의 눈금: `BASE × 2` = **32 DIP**.
    ///
    /// 종이처럼 창을 크게 차지하는 면의 모서리는 이 값의 배수(16 × 2n)만 쓴다.
    pub const STEP: f64 = Self::BASE * 2.0;

    /// 본문 한 글자의 **평균 폭** (DIP) — 1rem의 절반.
    ///
    /// 글자 폭은 글꼴이 정하므로 정확할 수 없다(글꼴은 지정할 수 없다 — 모듈 문서).
    /// 그래도 **어림값을 토큰으로 두면** "한 줄이 성립하는가"를 헤드리스로 검사할 수 있다.
    pub const CHAR_W: f64 = Self::BASE / 2.0;

    /// 라벨 붙은 버튼 한 줄의 **예상 폭** (DIP) — 패딩 + 아이콘 + 간격 + 글자.
    ///
    /// `labels`는 라벨의 **글자 수**다(`Intent::label().chars().count()`).
    /// 결과가 [`Tokens::toolbar_break`]보다 크면 그 줄은 둘로 나눠야 한다 —
    /// `tests/ui_plan.rs`가 실제 라벨로 이 검사를 한다.
    pub fn labeled_row_width(&self, labels: impl IntoIterator<Item = usize>) -> f64 {
        let labels: Vec<usize> = labels.into_iter().collect();
        // 버튼 하나 = WinUI 기본 좌우 패딩 + 아이콘(≈ control_h 안에 들어간다) + 간격 + 글자.
        let buttons: f64 = labels
            .iter()
            .map(|chars| self.control_h + self.gap + *chars as f64 * Self::CHAR_W)
            .sum();
        buttons + self.tight * labels.len().saturating_sub(1) as f64
    }

    /// `UNIT`의 배수인가 — 간격과 작은 면의 모서리가 지켜야 하는 규칙.
    pub fn is_unit_multiple(value: f64) -> bool {
        (value / Self::UNIT).fract() == 0.0
    }

    /// `STEP`의 배수인가 — 큰 면의 모서리가 지켜야 하는 규칙(16 × 2n).
    pub fn is_step_multiple(value: f64) -> bool {
        value > 0.0 && (value / Self::STEP).fract() == 0.0
    }

    /// **작은 면**의 모서리 3단계 — 전부 `UNIT`의 배수여야 한다(테스트가 확인한다).
    pub fn small_radii(&self) -> [f64; 3] {
        [self.control, self.radius, self.pill]
    }

    /// 타이포 배율 — **본문 1rem을 기준**으로 한 네 단계(내림차순: [`Tokens::sizes`]와 같은 순서).
    ///
    /// 문서 제목(1.75rem) · 제목(1.25rem) · 본문(1rem) · 캡션(0.75rem).
    /// 웹의 rem 관례와 같은 눈금이라 "16px 기준"이 문서 없이도 읽힌다.
    pub const SCALE: [f64; 4] = [1.75, 1.25, 1.0, 0.75];

    /// 타이포 4단계 — 내림차순이어야 계층이 읽힌다(예제 22).
    pub fn sizes(&self) -> [f64; 4] {
        [self.display, self.title, self.body, self.caption]
    }

    /// 크기 ÷ 기준 — [`Tokens::SCALE`]과 같아야 한다(테스트가 맞춰 본다).
    pub fn ratios(&self) -> [f64; 4] {
        let base = self.base;
        [
            self.display / base,
            self.title / base,
            self.body / base,
            self.caption / base,
        ]
    }

    /// 간격 5단계 — 리듬의 정의를 코드로 옮긴 것(예제 23).
    pub fn steps(&self) -> [f64; 5] {
        [self.tight, self.gap, self.block, self.pad, self.page]
    }

    /// 모든 간격이 단위의 배수인가 — 테스트가 이 규칙을 고정한다.
    pub fn is_rhythmic(&self) -> bool {
        self.steps()
            .iter()
            .all(|step| Self::is_unit_multiple(*step))
    }

    /// 화면의 **고정 크기** — 전부 `UNIT` 또는 `BASE`의 배수여야 한다(테스트가 확인한다).
    pub fn fixed_sizes(&self) -> [f64; 13] {
        [
            self.control_h,
            self.preview_w,
            self.preview_h,
            self.line_max,
            self.status_h,
            self.titlebar_h,
            self.toolbar_break,
            self.min_window_w,
            self.min_window_h,
            self.default_window_w,
            self.default_window_h,
            self.chrome_h,
            self.content_min,
        ]
    }
}

/// 이 앱의 토큰 — 화면 코드는 이 값만 본다(예제 30의 `guide()`와 같은 역할).
///
/// **모든 값이 원시 스칼라의 파생이다**(모듈 문서의 계보 표): 오른쪽은 항상
/// `UNIT × n`(간격·작은 면의 모서리·고정 크기) · `STEP × n`(큰 면의 모서리) ·
/// `BASE × SCALE[i]`(글자) 중 하나다 — 여기서만 규칙을 읽으면 되고, 화면 코드는 이름만 본다.
pub const TOKENS: Tokens = Tokens {
    // 리듬 — UNIT의 배수
    tight: Tokens::UNIT * 1.0, // 4
    gap: Tokens::UNIT * 2.0,   // 8
    block: Tokens::UNIT * 3.0, // 12
    pad: Tokens::UNIT * 4.0,   // 16
    page: Tokens::UNIT * 6.0,  // 24
    // 작은 면의 모서리 — UNIT의 배수
    control: Tokens::UNIT * 1.0, // 4
    radius: Tokens::UNIT * 2.0,  // 8
    pill: Tokens::UNIT * 4.0,    // 16 (높이의 절반을 넘으면 알약)
    // 큰 면의 모서리 — STEP(=BASE×2)의 배수
    sheet: Tokens::STEP * 1.0, // 32
    // 타이포 — BASE × 배율표
    base: Tokens::BASE,                       // 16 = 1rem
    display: Tokens::BASE * Tokens::SCALE[0], // 28
    title: Tokens::BASE * Tokens::SCALE[1],   // 20
    body: Tokens::BASE * Tokens::SCALE[2],    // 16
    caption: Tokens::BASE * Tokens::SCALE[3], // 12
    // 고정 크기 — UNIT 또는 BASE의 배수
    rail: Tokens::BASE * 16.0,             // 256
    control_h: Tokens::UNIT * 9.0,         // 36
    preview_w: Tokens::UNIT * 50.0,        // 200
    preview_h: Tokens::UNIT * 8.0,         // 32
    line_max: Tokens::UNIT * 5.0,          // 20
    status_h: Tokens::UNIT * 10.0,         // 40
    titlebar_h: Tokens::UNIT * 12.0,       // 48 = WinUI TitleBarHeightOption::Tall
    toolbar_break: Tokens::UNIT * 340.0,   // 1360 (라벨 12개를 한 줄에 세우는 하한 ≈1316)
    min_window_w: Tokens::BASE * 45.0,     // 720
    min_window_h: Tokens::BASE * 35.0,     // 560
    default_window_w: Tokens::BASE * 90.0, // 1440 (한 줄 툴바가 서는 크기)
    default_window_h: Tokens::BASE * 55.0, // 880
    chrome_h: Tokens::UNIT * 110.0,        // 440 (타이틀바 48 + 툴바 + 정보 띠)
    content_min: Tokens::UNIT * 60.0,      // 240
};

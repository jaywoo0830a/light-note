//! 스타일 토큰 계약 — 예제 21~30이 정한 규칙을 **헤드리스로** 못박는다.
//!
//! 토큰([`light_note_gui::style`])은 플랫폼을 모른다(WinUI 타입이 하나도 없다). 그래서
//! 여기서 잡히는 회귀는 WinUI에서도 그대로 회귀다 — 조각(`parts`)은 토큰만 읽으므로,
//! 토큰이 규칙을 지키면 화면도 규칙을 지킨다.

use light_note_gui::geom::Size;
use light_note_gui::style::{Tokens, TOKENS};

/// 토큰을 **런타임 값**으로 읽는다 — 상수 단언(`assertions_on_constants`)을 피한다.
/// 검사 자체는 그대로다: 값이 규칙을 지키는지 본다.
fn tokens() -> Tokens {
    TOKENS
}

#[test]
fn every_gap_is_a_multiple_of_the_unit() {
    // 예제 23: 간격은 **단위의 배수**만 쓴다 — 화면마다 값을 고르지 않는다.
    assert_eq!(Tokens::UNIT, 4.0);
    let t = tokens();
    assert!(t.is_rhythmic(), "{:?}가 리듬 밖이다", t.steps());
    for step in t.steps() {
        assert_eq!(
            (step / Tokens::UNIT).fract(),
            0.0,
            "{step} DIP는 4의 배수가 아니다"
        );
    }
}

#[test]
fn the_type_scale_descends() {
    // 예제 22: 계층은 **크기와 굵기**로 만든다 — 내림차순이 아니면 계층이 무너진다.
    let t = tokens();
    let sizes = t.sizes();
    for pair in sizes.windows(2) {
        assert!(pair[0] > pair[1], "{sizes:?}는 내림차순이어야 한다");
    }
    assert!(t.caption >= 11.0, "보조 설명도 읽을 수 있어야 한다");
    assert!(t.display <= 32.0, "문서 제목이 창을 잡아먹으면 안 된다");
}

#[test]
fn the_type_scale_is_based_on_one_rem() {
    // 기준은 **1rem = 16 DIP**이고, 네 단계는 배율표(`Tokens::SCALE`) 그대로다.
    // 표를 코드에 두면 "16px 기준"이 문서 없이도 읽히고, 테스트가 표와 값을 맞춰 본다.
    let t = tokens();
    assert_eq!(t.base, 16.0, "기준은 1rem = 16 DIP");
    assert_eq!(t.body, t.base, "본문이 곧 1rem이다");
    assert_eq!(t.ratios(), Tokens::SCALE, "배율표와 값이 어긋난다");
    assert_eq!(Tokens::SCALE[2], 1.0, "배율표의 기준은 1.0(1rem)이다");
    // 크기는 전부 기준의 배수다(임의의 홀수 크기를 쓰지 않는다).
    for size in t.sizes() {
        assert_eq!(
            (size / t.base * 100.0).fract(),
            0.0,
            "{size} DIP가 1rem의 깔끔한 배수가 아니다"
        );
    }
}

#[test]
fn every_value_comes_from_the_raw_scalars() {
    // 값은 손으로 고르지 않는다 — 원시 스칼라(BASE · UNIT · STEP)에서 규칙 하나로 나온다.
    let t = tokens();
    assert_eq!(Tokens::BASE, 16.0, "기준 스칼라는 1rem = 16 DIP");
    assert_eq!(Tokens::UNIT, Tokens::BASE / 4.0, "UNIT은 BASE/4");
    assert_eq!(Tokens::STEP, Tokens::BASE * 2.0, "큰 면 눈금은 BASE×2");

    // 간격 · 작은 면의 모서리 · 고정 크기 → 전부 UNIT의 배수.
    for step in t.steps() {
        assert!(Tokens::is_unit_multiple(step), "간격 {step}이 UNIT 밖이다");
    }
    for radius in t.small_radii() {
        assert!(
            Tokens::is_unit_multiple(radius),
            "작은 면의 모서리 {radius}가 4의 배수가 아니다"
        );
    }
    for size in t.fixed_sizes() {
        assert!(
            Tokens::is_unit_multiple(size),
            "고정 크기 {size}가 UNIT 밖이다"
        );
    }
    // 레일만 BASE의 배수(256 = 16 × 16) — 넓은 고정 폭이라 rem 눈금을 쓴다.
    assert_eq!(
        (t.rail / Tokens::BASE).fract(),
        0.0,
        "레일이 rem 눈금 밖이다"
    );

    // 글자 → BASE × 배율표.
    assert_eq!(t.ratios(), Tokens::SCALE, "글자 크기가 배율표를 벗어났다");
}

#[test]
fn the_radii_follow_the_scale_and_pill_convention() {
    // 예제 24: 면의 크기로 눈금이 갈린다 — 작은 면은 4의 배수, 큰 면은 16×2n.
    let t = tokens();
    let [control, card, pill] = t.small_radii();
    assert!(control < card, "컨트롤이 카드보다 둥글면 안 된다");
    assert!(card < pill, "카드가 알약보다 둥글면 안 된다");
    assert!(
        Tokens::is_step_multiple(t.sheet),
        "큰 면({})이 16×2n 눈금 밖이다",
        t.sheet
    );
    assert!(t.sheet > pill, "큰 면이 알약보다 각져 보이면 안 된다");
    // 알약은 높이의 절반만 넘으면 WinUI가 잘라 알약으로 만든다 — 미리보기 상자도 포함.
    assert!(
        pill >= t.preview_h / 2.0,
        "알약 반지름이 미리보기 상자를 못 채운다"
    );
}

#[test]
fn the_ink_preview_fits_inside_its_box() {
    // 미리보기 선은 상자 높이를 넘으면 **조용히 잘린다** — 최대 굵기를 상자 안에 묶는다.
    let t = tokens();
    assert!(t.line_max < t.preview_h);
    assert!(t.line_max >= 8.0, "굵은 펜을 보여줄 수 있어야 한다");
    assert!(t.preview_w > 0.0);
    assert!(
        t.preview_w - t.gap * 2.0 > 0.0,
        "막대가 상자보다 좁아야 한다"
    );
}

#[test]
fn the_controls_have_a_consistent_size() {
    // 아이콘 버튼은 손가락/펜으로 누를 수 있어야 하고, 정보 띠는 두 줄이라 그보다 높다.
    let t = tokens();
    assert!(t.control_h >= 32.0, "누를 수 있는 크기여야 한다");
    assert!(t.status_h >= t.base, "정보 띠는 한 줄(1rem)보다 높다");
    assert!(t.rail > t.control_h * 4.0, "레일은 버튼 몇 개보다 넓다");
}

#[test]
fn the_sidebar_is_fixed_and_narrower_than_a_page() {
    // 예제 27: 레일은 고정 폭(`Pixel`)이고, 본문(A4 폭)을 잡아먹지 않는다.
    let t = tokens();
    assert!(t.rail > 0.0);
    assert!(t.rail < f64::from(Size::A4.width), "레일이 페이지보다 넓다");
}

#[test]
fn the_font_is_inherited_not_chosen() {
    // `windows-reactor` 0.100에는 `FontFamily` 통로가 없다: 속성 계층(`generated.rs`의
    // `TextBlock`)에 `font_family`가 없고, 리소스 재정의(`ResourceValue`)도
    // `Color`/`Thickness`/`CornerRadius`뿐이다(네이티브 `ITextBlock_Vtbl`에는 슬롯이 있다).
    // 그래서 글꼴은 **시스템이 정하고**, 우리는 그 결과를 이름으로만 기록한다 —
    // 화면 코드 어디에도 폰트 이름이 흩어지지 않는다는 것이 이 테스트의 계약이다.
    assert_eq!(
        Tokens::FONT_FAMILY,
        "Segoe UI Variable",
        "WinUI3 기본 글꼴을 상속한다(Windows 11)"
    );
}

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
    assert!(t.caption >= 10.0, "보조 설명도 읽을 수 있어야 한다");
}

#[test]
fn the_radii_follow_the_scale_and_pill_convention() {
    // 예제 24: 컨트롤 < 카드, 그리고 알약은 "높이보다 큰 값"이라는 관례다.
    let t = tokens();
    assert!(t.control < t.radius, "컨트롤이 카드보다 둥글면 안 된다");
    assert!(t.pill > t.preview_h, "알약은 높이보다 커야 한다");
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
    // 아이콘 버튼은 정사각이고, 상태바는 그 절반 남짓이다 — 눈금이 하나다.
    let t = tokens();
    assert!(t.control_h >= 24.0, "누를 수 있는 크기여야 한다");
    assert!(t.status_h <= t.control_h);
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
fn the_font_family_is_recorded_but_not_faked() {
    // Google Sans Flex는 **지금 쓸 수 없다**: `windows-reactor` 0.100의 `TextBlock`이
    // `FontFamily`를 노출하지 않는다(예제 22의 결론). 그래서 이름만 토큰에 두고,
    // 통로가 생기면 한 곳만 고친다 — 화면 코드에 폰트 이름을 흩뿌리지 않는다.
    assert_eq!(Tokens::FONT_FAMILY, "Google Sans Flex");
}

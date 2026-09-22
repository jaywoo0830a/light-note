//! 화면 조각 — **개발자 도구**: 디지타이저 진단 표 + 리스캔.
//!
//! 필기가 안 될 때 그 이유를 화면에서 읽게 한다: ①시스템에 펜이 있는가 ②어느 창에 훅이
//! 걸렸는가 ③그 창으로 포인터 메시지가 몇 개 왔는가(`digitizer::Digest`).
//!
//! ## 값은 **호스트가 문장으로** 만든다
//! 이 조각은 표를 그리기만 한다: 줄(`(이름, 값)`)은 호스트가 `Digest::report`로 만들고 화면
//! 언어 규칙(영어)도 거기서 지킨다 — 진단 문구가 조각 밖에서 검증된다.
//!
//! ## 정렬은 못 한다 (이 백엔드의 한계)
//! 이름/값을 좌우 열로 나누려면 `Grid`의 열이 필요한데 이 버전은 열에 폭을 주지 못한다
//! (`parts::row`의 확인 목록). 그래서 한 줄 = `이름(굵게) + 값`의 **왼쪽 흐름**으로 둔다.
//!
//! ## 자리
//! 본문 **위**(정보 띠 아래)에 앉는다: 아래에 두면 잉크 영역 높이 추정이 틀릴 때 화면 밖으로
//! 밀려 **정작 필요할 때 안 보인다**.

use elm_magic_windows_reactor::RawSlot;
use windows_reactor::{FontWeight, View};

use super::{buttons, card, column, label, meta, meta_wrapped, row};
use crate::style::TOKENS;
use crate::ui::{Intent, IntentSink, ViewModel};

/// 개발자 도구 패널 하나 — 진단 줄들 + 리스캔 버튼.
pub fn devtools(view: &ViewModel, sink: &IntentSink) -> RawSlot {
    let mut rows: Vec<View> = vec![
        label("Diagnostics", TOKENS.title, FontWeight::SEMI_BOLD).into(),
        meta_wrapped(
            "Pen input comes from OpenTabletDriver: a plugin writes tablet reports into shared \
             memory and this app reads them. If nothing arrives, the OTD daemon or the plugin is \
             missing.",
        )
        .into(),
    ];
    if view.diag.is_empty() {
        rows.push(meta("No diagnostics yet — the table fills on the next update.").into());
    }
    for (name, value) in &view.diag {
        rows.push(row(
            TOKENS.gap,
            vec![
                label(name, TOKENS.caption, FontWeight::SEMI_BOLD).into(),
                meta(value).into(),
            ],
        ));
    }
    rows.push(row(
        TOKENS.gap,
        vec![
            buttons::label_button(Intent::Rescan.label(), Intent::Rescan, sink),
            meta("Reopens the shared memory and re-reads the tablet range.").into(),
        ],
    ));
    Some(card(TOKENS.pad, column(TOKENS.gap, rows)))
}

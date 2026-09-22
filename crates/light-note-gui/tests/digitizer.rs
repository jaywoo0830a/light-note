//! 디지타이저 진단의 **머리** — 화면 없이 검증한다.
//!
//! 필기가 안 될 때 사람이 보는 것은 결국 진단 표의 문장이다: 그 문장이 **사실을 정확히**
//! 말하는지, 영어인지, 어느 관문에서 막혔는지를 말하는지를 여기서 못박는다.
//! (`digitizer::Digest`는 값만 들고 있고, 문장은 `report`가 만든다 — 순수 함수다.)

use light_note_gui::digitizer::{Digest, Digitizer, HookState, Probe};
use light_note_gui::input::{Device, PointerFrame};

/// 진단 표에서 값 하나를 꺼낸다 — 표의 **순서가 아니라 이름**으로 읽는다.
fn row(digest: &Digest, name: &str) -> String {
    digest
        .report()
        .into_iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
        .unwrap_or_else(|| panic!("진단 표에 {name} 줄이 없다"))
}

/// 펜이 있는 기계 + 훅이 걸린 창 + 펜 메시지가 온 상태.
fn working() -> Digest {
    Digest {
        state: HookState::Hooking,
        seen_pen: true,
        messages: 42,
        probes: vec![Probe {
            hwnd: 0x2a1c,
            class: "Microsoft.UI.Content.DesktopChildSiteBridge".to_string(),
            title: String::new(),
            visible: true,
            size: (1440, 880),
            hooked: true,
            messages: 42,
            pen: 42,
        }],
        last: Some(PointerFrame::new(
            Device::Pen,
            Some(0.5),
            Some((-3.0, 1.0)),
            std::time::Instant::now(),
        )),
        last_age_ms: Some(12),
        digitizer: Digitizer {
            present: true,
            integrated_pen: true,
            external_pen: true,
            integrated_touch: true,
            ready: true,
        },
    }
}

#[test]
fn a_working_digitizer_reports_every_gate_as_open() {
    let digest = working();
    assert!(row(&digest, "System digitizer").contains("integrated pen"));
    assert!(row(&digest, "System digitizer").contains("external pen"));
    assert_eq!(row(&digest, "Hook"), "hooked on 1 of 1 window(s)");
    assert!(row(&digest, "Pen frames").starts_with("yes"));
    assert_eq!(row(&digest, "WM_POINTER messages"), "42");
    // 마지막 프레임은 장치·필압·틸트·나이를 그대로 보여준다(숫자를 지어내지 않는다).
    let frame = row(&digest, "Last frame");
    assert!(frame.starts_with("Pen"), "{frame}");
    assert!(frame.contains("pressure 0.50"), "{frame}");
    assert!(frame.contains("tilt -3/1"), "{frame}");
    assert!(frame.contains("12 ms ago"), "{frame}");
    // 창 표는 **어느 창에 걸렸는지**를 말한다 — 이게 진단의 핵심이다.
    let window = row(
        &digest,
        "Microsoft.UI.Content.DesktopChildSiteBridge  0x2a1c",
    );
    assert!(window.contains("hooked"), "{window}");
    assert!(window.contains("msgs 42 · pen 42"), "{window}");
    // 펜이 있으므로 힌트 줄은 **없다**(거짓 조언을 하지 않는다).
    assert!(
        !digest.report().iter().any(|(name, _)| name == "Hint"),
        "펜이 있으면 힌트가 없다"
    );
}

#[test]
fn a_missing_pen_is_reported_first_with_a_hint() {
    // 펜이 없는 기계: 첫 줄이 그 사실을 말하고, 힌트가 **결론**을 준다.
    let digest = Digest {
        digitizer: Digitizer::default(),
        seen_pen: false,
        messages: 0,
        probes: Vec::new(),
        last: None,
        last_age_ms: None,
        state: HookState::NoWindow,
    };
    let rows = digest.report();
    assert_eq!(rows[0].0, "System digitizer", "첫 줄은 하드웨어다");
    assert!(rows[0].1.starts_with("none reported"), "{:?}", rows[0]);
    assert!(
        rows.iter()
            .any(|(name, value)| name == "Hint" && value.contains("needs a pen digitizer")),
        "펜이 없으면 힌트가 결론을 말한다"
    );
    assert_eq!(row(&digest, "Hook"), "no app window found — still looking");
    assert!(row(&digest, "Pen frames").contains("no pointer message"));
    assert_eq!(row(&digest, "Windows"), "none found on this thread");
}

#[test]
fn the_hook_state_is_named_not_numbered() {
    // 상태는 숫자가 아니라 **문장**으로 나온다(사람이 읽는 표다).
    let digest = working();
    for state in [
        HookState::Hooking,
        HookState::Idle,
        HookState::NoWindow,
        HookState::Failed,
    ] {
        let text = Digest {
            state,
            ..digest.clone()
        }
        .report();
        let hook = text
            .iter()
            .find(|(name, _)| name == "Hook")
            .map(|(_, value)| value.clone())
            .expect("Hook 줄");
        assert!(!hook.is_empty(), "{state:?}");
        assert!(
            hook.chars().any(|c| c.is_ascii_alphabetic()),
            "{state:?} → {hook}: 문장이어야 한다"
        );
    }
}

#[test]
fn the_report_is_english_only() {
    // 화면 언어 규칙: 진단 표도 예외가 아니다(사용자가 읽는 문자열이다).
    let samples = [
        working(),
        Digest {
            seen_pen: false,
            ..working()
        },
        Digest {
            messages: 0,
            probes: Vec::new(),
            digitizer: Digitizer::default(),
            ..working()
        },
    ];
    for digest in samples {
        for (name, value) in digest.report() {
            for text in [name, value] {
                assert!(
                    !text.chars().any(|c| c.is_alphabetic() && !c.is_ascii()),
                    "{text}: 영어만 쓴다"
                );
            }
        }
    }
}

#[test]
fn a_touch_only_machine_does_not_claim_a_pen() {
    // 터치만 있는 기계: 디지타이저는 있다고 말하되 **펜은 없다**고 말해야 한다.
    let digest = Digest {
        digitizer: Digitizer {
            present: true,
            integrated_touch: true,
            ready: true,
            ..Digitizer::default()
        },
        seen_pen: false,
        ..working()
    };
    assert!(!digest.digitizer.has_pen());
    // 터치만 있으면 요약에 **펜이 없다**: "touch"라고만 말한다(펜을 봤다고 하면 거짓말이다).
    assert!(!row(&digest, "System digitizer").contains("pen"));
    assert!(digest.report().iter().any(|(name, _)| name == "Hint"));
}

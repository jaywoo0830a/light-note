//! 디지타이저 진단의 **머리** — 화면 없이 검증한다.
//!
//! 필기가 안 될 때 사람이 보는 것은 결국 진단 표의 문장이다: 그 문장이 **사실을 정확히**
//! 말하는지, 영어인지, 어느 관문에서 막혔는지를 말하는지를 여기서 못박는다.
//! (`digitizer::Digest`는 값만 들고 있고, 문장은 `report`가 만든다 — 순수 함수다.)

use light_note_gui::digitizer::{
    compact_device_name, usage_label, Digest, Digitizer, HookState, PenDevice, Probe,
};
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
        mouse: 0,
        probes: vec![Probe {
            hwnd: 0x2a1c,
            class: "Microsoft.UI.Content.DesktopChildSiteBridge".to_string(),
            title: String::new(),
            visible: true,
            size: (1440, 880),
            hooked: true,
            messages: 42,
            pen: 42,
            mouse: 0,
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
        // OTD의 Windows Ink 플러그인은 VMulti 가상 디지타이저(VID 0x00FF/PID 0xBACC)로 펜을 낸다.
        pen_devices: vec![PenDevice {
            name: "hid vid_00ff&pid_bacc".to_string(),
            usage: 0x02,
        }],
    }
}

#[test]
fn the_frame_line_reports_the_pen_pose() {
    // `penFlags`/`PEN_MASK_ROTATION`도 **사실**이다 — 진단이 뒤집힘·지우개 끝·회전을 말하는가.
    let digest = Digest {
        last: Some(
            PointerFrame::new(Device::Pen, Some(0.4), None, std::time::Instant::now())
                .with_pen_pose(true, true, Some(90.0)),
        ),
        ..working()
    };
    let frame = row(&digest, "Last frame");
    assert!(frame.contains("flipped (erasing)"), "{frame}");
    assert!(frame.contains("eraser tip"), "{frame}");
    assert!(frame.contains("rotation 90"), "{frame}");

    // 보고하지 않은 자세는 **말하지 않는다**(없는 값을 0으로 채우지 않는다).
    let plain = row(&working(), "Last frame");
    assert!(!plain.contains("flipped"), "{plain}");
    assert!(!plain.contains("rotation"), "{plain}");
}

#[test]
fn a_pen_sent_as_mouse_is_reported_as_such() {
    // OTD의 기본 Absolute/Relative Mode는 `SendInput` **마우스**다 — `WM_POINTER`가 하나도 안 온다.
    // 그때 표가 "아무 입력도 없다"고만 하면 사람은 훅을 의심한다(거짓 단서). 마우스 수가 답이다.
    let digest = Digest {
        seen_pen: false,
        messages: 0,
        mouse: 512,
        ..working()
    };
    assert_eq!(row(&digest, "WM_POINTER messages"), "0");
    assert_eq!(row(&digest, "Mouse messages"), "512");
    assert!(
        digest
            .report()
            .iter()
            .any(|(name, value)| name == "Hint" && value.contains("Windows Ink plugin")),
        "마우스만 오면 출력 모드를 바꾸라고 말한다"
    );
    // 펜이 오고 있으면 그 힌트는 **없다**(거짓 조언을 하지 않는다).
    assert!(!working()
        .report()
        .iter()
        .any(|(name, value)| name == "Hint" && value.contains("Windows Ink plugin")),);
}

#[test]
fn the_pen_device_list_names_the_driver() {
    // Windows가 아는 디지타이저 이름이 곧 **어떤 드라이버가 펜을 내보내는가**다(VMulti = VirtualHID).
    let digest = working();
    let devices = row(&digest, "Windows pen devices");
    assert!(devices.contains("hid vid_00ff&pid_bacc"), "{devices}");
    assert!(devices.ends_with("· pen"), "{devices}");

    // OS가 펜을 아예 모르면 목록이 비고, 그 사실이 **문장으로** 나온다(빈 칸이 아니다).
    let empty = Digest {
        pen_devices: Vec::new(),
        ..digest
    };
    assert_eq!(
        row(&empty, "Windows pen devices"),
        "none — Windows sees no digitizer device"
    );
}

#[test]
fn device_names_and_usages_are_readable() {
    // 원시 입력 장치는 긴 경로로 온다 — 표에는 **사람이 읽는 이름**만 남긴다.
    assert_eq!(
        compact_device_name(
            "\\\\?\\hid#vid_00ff&pid_bacc&mi_00#7&5b1c&0&0000#{4d1e55b2-7ba6-4d9f-b4c4-4b5f}"
        ),
        "hid vid_00ff&pid_bacc&mi_00",
        "인터페이스(mi_xx)까지가 장치를 가르는 부분이다"
    );
    assert_eq!(
        compact_device_name("hid#vid_056a&pid_0374"),
        "hid vid_056a&pid_0374"
    );
    assert_eq!(compact_device_name(""), "", "빈 경로는 빈 이름이다");
    // 용도의 이름 — 펜과 손가락은 **다른 장치**다.
    assert_eq!(usage_label(0x02), "pen");
    assert_eq!(usage_label(0x04), "touch screen");
    assert_eq!(usage_label(0x99), "digitizer device");
}

#[test]
fn the_raw_input_query_answers_on_this_machine() {
    // 실제 Win32 왕복이 크래시 없이 **문장**을 만드는지 — 이 기계에 디지타이저가 없어도 답이 나온다.
    // 무엇으로 답하는지 보려면: `cargo test --test digitizer the_raw_input -- --nocapture`
    let digest = light_note_gui::digitizer::digest();
    let devices = row(&digest, "Windows pen devices");
    println!("Windows pen devices: {devices}");
    assert!(!devices.is_empty(), "빈 칸이 아니라 문장이 나온다");
    // 창을 하나도 못 봤으면 마우스 카운터도 0이다(창 프로시저가 센 값이므로).
    assert_eq!(row(&digest, "Mouse messages"), "0");
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
        mouse: 0,
        probes: Vec::new(),
        last: None,
        last_age_ms: None,
        state: HookState::NoWindow,
        pen_devices: Vec::new(),
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

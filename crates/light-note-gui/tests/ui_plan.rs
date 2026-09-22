//! 화면 계약 — elm 계획(plan)과 `<Raw>` 경계를 **헤드리스로** 검증한다.
//!
//! 화면은 플랫폼을 모르므로 여기서 잡히는 회귀는 WinUI에서도 그대로 회귀다. 버튼은 이제
//! 조각(`<Raw>`)이 만들지만, **정의**(어떤 의도가 어디에 있는가)와 **통로**(의도가 호스트로
//! 가는 길)는 플랫폼 무관이라 여기서 못박는다 — 그리는 일만 WinUI가 한다.

use std::cell::RefCell;
use std::rc::Rc;

use elm_magic::prelude::*;
use elm_magic_windows_reactor::{plan, Pass, PlanNode};

use light_note_gui::geom::{Pt, Scale, Size};
use light_note_gui::ink::{InkPoint, Stroke, Style, Tool};
use light_note_gui::input::Device;
use light_note_gui::shape::live_ink;
use light_note_gui::style::TOKENS;
use light_note_gui::ui::{
    clear_frame, clear_intent_sink, clear_part_builder, clear_surface_builder, clear_view,
    page_label, set_intent_sink, set_part_builder, set_surface_builder, stage_frame, stage_view,
    Frame, InkSurface, InkSurfaceProps, Intent, IntentSink, Part, Screen, ScreenProps, Stage,
    SurfaceSlot, TitleBar, TitleBarProps, ViewModel,
};

/// 화면이 **항상** 갖는 조각 수 — 타이틀바 · 툴바 · 레일 · 상태 띠.
const CHROME_RAW: usize = 4;

/// 화면에 내려가는 값 하나 — 호스트가 진실을 소유하므로 테스트가 그 값을 정한다.
fn view(tool: Tool, stage: Stage) -> ViewModel {
    ViewModel {
        tool,
        width_pt: Style::for_tool(tool).width_pt,
        zoom: 100.0,
        page: 0,
        page_count: 1,
        stroke_count: 0,
        live_shapes: 0,
        baked: 0,
        can_undo: false,
        can_redo: false,
        dirty: false,
        title: "Untitled".to_string(),
        pdf_name: String::new(),
        stage,
        status: "Untitled · page 1 / 1 · 0 strokes".to_string(),
        input: "Pen — digitizer active".to_string(),
        help: false,
        viewport: (1280.0, 800.0),
        page_labels: vec![page_label(0, 0)],
    }
}

/// 의도를 기록하는 콜백 하나 — elm에서 올라오는 길(키보드·콜백 prop)의 계약이다.
fn props(view: ViewModel, log: &Rc<RefCell<Vec<Intent>>>) -> ScreenProps {
    let log = Rc::clone(log);
    ScreenProps {
        view: Some(view),
        on_intent: Some(Callback::new(move |_arena: &mut Arena, intent: Intent| {
            log.borrow_mut().push(intent);
        })),
        ..Default::default()
    }
}

fn plan_of(view: ViewModel) -> (PlanNode, Pass) {
    let mut ctx = Ctx::new();
    let tree =
        elm_magic::frame::<Screen>(&mut ctx, &props(view, &Rc::new(RefCell::new(Vec::new()))));
    plan(&tree)
}

/// "영어만"의 판정 — 알파벳은 ASCII뿐이어야 한다(`—`/`·`/`…` 같은 구두점은 자유다).
fn is_english(text: &str) -> bool {
    !text.chars().any(|c| c.is_alphabetic() && !c.is_ascii())
}

#[test]
fn the_chrome_definition_covers_every_intent_once() {
    // 버튼의 **정의**는 플랫폼 무관이다 — 툴바 + 레일 + 복구가 화면의 의도를 나눠 갖는다.
    let flat: Vec<Intent> = Intent::TOOLBAR
        .iter()
        .flat_map(|group| group.iter().copied())
        .chain(Intent::RAIL.iter().flat_map(|group| group.iter().copied()))
        .chain([Intent::Retry])
        .collect();
    assert_eq!(
        flat.len(),
        Intent::CHROME.len(),
        "정의(툴바+레일+복구)와 CHROME의 개수가 어긋난다"
    );
    for intent in Intent::CHROME {
        assert_eq!(
            flat.iter()
                .filter(|candidate| **candidate == intent)
                .count(),
            1,
            "{intent:?}가 화면 어딘가에 **정확히 한 번** 있어야 한다"
        );
    }

    // 라벨은 서로 다르고 전부 영어다(툴팁·자동화 이름이 서로 구별돼야 한다).
    let mut labels: Vec<&str> = Intent::CHROME.iter().map(|intent| intent.label()).collect();
    for label in &labels {
        assert!(is_english(label), "{label}");
    }
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), Intent::CHROME.len(), "라벨이 중복이다");
}

#[test]
fn the_toolbar_rows_fit_the_break_width() {
    // 라벨을 붙이면 줄이 길어진다 — **한 줄 배치와 좁은 창의 두 줄 배치 모두** 기준 폭 안에
    // 서야 한다. 폭은 어림값(글자 수 × 1rem/2)이지만 이 계산이 곧 계약이다:
    // 어림이 넘치면 실제 배치도 넘친다(그때는 라벨을 줄이거나 나누는 자리를 옮긴다).
    let chars = |groups: &[&[Intent]]| -> Vec<usize> {
        groups
            .iter()
            .flat_map(|group| group.iter().map(|intent| intent.label().chars().count()))
            .collect()
    };
    let one = TOKENS.labeled_row_width(chars(&Intent::TOOLBAR));
    assert!(
        one <= TOKENS.toolbar_break,
        "한 줄 툴바({one} DIP)가 기준 폭({})을 넘는다",
        TOKENS.toolbar_break
    );

    // 좁은 창에서 쓰는 나누는 자리도 **정의에서** 온다(화면과 검사가 같은 값을 본다).
    let split = light_note_gui::parts::toolbar::TOOL_SPLIT;
    assert!(
        split > 0 && split < Intent::TOOLBAR.len(),
        "나누는 자리가 밖이다"
    );
    for half in [&Intent::TOOLBAR[..split], &Intent::TOOLBAR[split..]] {
        let width = TOKENS.labeled_row_width(chars(half));
        assert!(
            width <= TOKENS.toolbar_break,
            "좁은 창의 줄({width} DIP)이 기준 폭을 넘는다"
        );
    }
}

#[test]
fn every_visible_string_is_english_only() {
    // 화면 언어 규칙: 사용자가 읽는 문자열은 **전부 영어**다(라벨·힌트·단계·장치).
    for intent in Intent::CHROME {
        assert!(is_english(intent.label()), "{}", intent.label());
    }
    assert!(is_english(Intent::CloseHelp.label()));
    assert!(is_english(Intent::GoToPage(0).label()));
    for tool in Tool::ALL {
        assert!(is_english(tool.label()), "{}", tool.label());
        assert!(is_english(Style::hint(tool)), "{}", Style::hint(tool));
    }
    for stage in [
        Stage::Empty,
        Stage::Loading,
        Stage::Ready,
        Stage::Failed("Failed".to_string()),
    ] {
        assert!(is_english(&stage.message()), "{}", stage.message());
    }
    assert!(is_english(&page_label(0, 4)), "{}", page_label(0, 4));
    for device in [Device::Pen, Device::Touch, Device::Mouse] {
        assert!(is_english(device.label()), "{}", device.label());
    }
}

#[test]
fn the_screen_is_chrome_plus_ink() {
    // 조각의 **순서**가 곧 화면 순서다: 타이틀바 → 툴바 → 정보 띠 → 본문(레일 + 잉크).
    // 정보 띠가 본문 **위**에 있는 것은 의도다: 아래에 두면 잉크 영역 높이 추정이 틀릴 때
    // 화면 밖으로 밀린다.
    let (node, _) = plan_of(view(Tool::Pen, Stage::Ready));
    let kinds: Vec<&str> = node.children.iter().map(|child| child.control()).collect();
    assert_eq!(
        kinds,
        vec!["Raw", "Raw", "Raw", "StackPanel"],
        "타이틀바 · 툴바 · 정보 띠(조각) · 본문(레일 조각 + 잉크 조각)"
    );
}

#[test]
fn no_elm_buttons_remain() {
    // 버튼은 전부 조각이 만든다: elm 계획에는 버튼이 **하나도** 없어야 한다.
    let (_, pass) = plan_of(view(Tool::Pen, Stage::Ready));
    assert_eq!(pass.count("Button"), 0, "elm 기본 버튼을 쓰지 않는다");
    assert_eq!(pass.count("TextBlock"), 0, "글자도 조각이 만든다");
}

#[test]
fn the_stage_decides_which_parts_appear() {
    // 준비됨: 조각 넷 + 잉크 표면.
    let (_, pass) = plan_of(view(Tool::Pen, Stage::Ready));
    assert_eq!(pass.count("Raw"), CHROME_RAW + 1, "잉크 표면은 언제나 하나");

    // 빈 페이지: 표면은 그대로 있고 **안내는 표면 안**에 있다(잉크 영역 높이가 단계마다
    // 달라지면 크롬이 화면 밖으로 밀린다 — `render::surface`가 안내를 스크롤 안에 둔다).
    let (_, pass) = plan_of(view(Tool::Pen, Stage::Empty));
    assert_eq!(pass.count("Raw"), CHROME_RAW + 1, "빈 상태도 표면 하나");

    // 여는 중: 표면 대신 스피너(아직 문서가 없다).
    let (_, pass) = plan_of(view(Tool::Pen, Stage::Loading));
    assert_eq!(pass.count("Raw"), CHROME_RAW + 1, "표면 없이 진행 상황만");

    // 실패: 표면 대신 이유 + 복구 동선.
    let (_, pass) = plan_of(view(Tool::Pen, Stage::Failed("Encrypted PDF".to_string())));
    assert_eq!(pass.count("Raw"), CHROME_RAW + 1, "표면 없이 복구 동선만");
}

#[test]
fn the_shortcut_panel_follows_the_host_flag() {
    // 열림 상태는 **호스트가 소유**한다(`view.help`) — elm은 자리만 정한다.
    let mut open = view(Tool::Pen, Stage::Ready);
    open.help = true;
    let (_, pass) = plan_of(open);
    assert_eq!(pass.count("Raw"), CHROME_RAW + 2, "단축키 패널이 더 있다");

    let (_, pass) = plan_of(view(Tool::Pen, Stage::Ready));
    assert_eq!(pass.count("Raw"), CHROME_RAW + 1, "닫혀 있으면 자리도 없다");
}

#[test]
fn the_f1_and_escape_keys_send_intents() {
    // elm은 상태를 소유하지 않는다 — 키는 **의도로 바뀌어** 호스트로 간다.
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut app = elm_magic::mount_with::<Screen>(props(view(Tool::Pen, Stage::Ready), &log));

    app.press_key("F1");
    app.press_key("Escape");
    assert_eq!(
        log.borrow().as_slice(),
        [Intent::ToggleHelp, Intent::CloseHelp],
        "F1 = 토글, Esc = 닫기"
    );
}

thread_local! {
    /// 조각 빌더가 받은 것 — 헤드리스에서는 WinUI 뷰를 만들 수 없으므로 **받은 것**을 기록한다.
    static SEEN_PART: RefCell<Option<(Part, ViewModel, IntentSink)>> = const { RefCell::new(None) };
    /// 표면 빌더가 받은 재료(프레임 + 값).
    static SEEN_FRAME: RefCell<Option<(Frame, ViewModel)>> = const { RefCell::new(None) };
}

/// 조각 빌더 대역 — 창이 있어야 뷰를 만들 수 있으므로 `None`을 돌려준다.
fn recording_part_builder(part: Part, view: &ViewModel, sink: &IntentSink) -> SurfaceSlot {
    SEEN_PART.with(|slot| *slot.borrow_mut() = Some((part, view.clone(), Rc::clone(sink))));
    None
}

/// 표면 빌더 대역 — 같은 이유로 `None`.
fn recording_builder(frame: &Frame, view: &ViewModel) -> SurfaceSlot {
    SEEN_FRAME.with(|slot| *slot.borrow_mut() = Some((frame.clone(), view.clone())));
    None
}

#[test]
fn the_chrome_values_and_intent_sink_reach_the_parts() {
    // 조각은 **화면이 받은 값 그대로**와, 버튼이 쓸 **의도 통로**를 함께 받는다.
    clear_view();
    clear_part_builder();
    clear_intent_sink();
    set_part_builder(Rc::new(recording_part_builder));
    let sent = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&sent);
    set_intent_sink(Rc::new(move |intent: Intent| log.borrow_mut().push(intent)));

    let mut status = view(Tool::Highlighter, Stage::Ready);
    status.zoom = 150.0;
    status.width_pt = 14.0;
    status.stroke_count = 3;
    status.baked = 2;
    status.live_shapes = 5;
    status.can_undo = true;
    status.dirty = true;
    status.page = 1;
    status.page_count = 2;
    status.page_labels = vec![page_label(0, 4), page_label(1, 0)];
    stage_view(status.clone());

    let mut ctx = Ctx::new();
    let tree = elm_magic::frame::<TitleBar>(&mut ctx, &TitleBarProps::default());
    let mut slot: SurfaceSlot = None;
    elm_magic::raw::invoke(&tree, &mut slot);
    assert!(slot.is_none(), "헤드리스에서는 WinUI 뷰를 만들 수 없다");

    let (part, seen, sink) = SEEN_PART
        .with(|slot| slot.borrow().clone())
        .expect("조각 빌더가 받은 것");
    assert_eq!(part, Part::TitleBar);
    assert_eq!(seen, status, "조각은 화면이 받은 값 그대로를 본다");

    // 조각들이 화면에 쓰는 문구 — 값에서 나오고, 전부 영어다.
    assert_eq!(seen.tool_label(), "Highlighter");
    assert_eq!(seen.zoom_label(), "150%");
    assert_eq!(seen.pipeline_label(), "Base 2 strokes · live 5 shapes");
    assert_eq!(seen.hint(), Style::hint(Tool::Highlighter));
    assert_eq!(seen.page_labels[1], "Page 2 · 0 strokes");
    assert!(is_english(&seen.status), "{}", seen.status);
    assert!(is_english(&seen.input), "{}", seen.input);

    // **버튼이 쓸 통로**: 통로로 보낸 의도가 호스트 대역에 그대로 도착한다.
    sink(Intent::Pen);
    sink(Intent::Undo);
    assert_eq!(sent.borrow().as_slice(), [Intent::Pen, Intent::Undo]);

    // 값이 없으면 조각도 그리지 않는다 — 거짓 그림을 만들지 않는다.
    clear_view();
    SEEN_PART.with(|slot| *slot.borrow_mut() = None);
    let mut empty: SurfaceSlot = None;
    elm_magic::raw::invoke(&tree, &mut empty);
    assert!(SEEN_PART.with(|slot| slot.borrow().is_none()));

    clear_part_builder();
    clear_intent_sink();
}

#[test]
fn the_surface_reaches_the_registered_builder_through_one_raw_slot() {
    clear_frame();
    clear_surface_builder();
    set_surface_builder(Rc::new(recording_builder));

    // ③이 만든 재료 — 호스트가 `view()` 안에서 `stage_frame`으로 넘기는 것과 같다.
    let scale = Scale::new(1.5);
    let stroke = Stroke::new(
        Tool::Pen,
        Style::for_tool(Tool::Pen),
        InkPoint::at(Pt::new(10.0, 10.0)),
    );
    let frame = Frame {
        tail: Rc::from(vec![live_ink(&stroke, scale)]),
        size: Size::new(200.0, 100.0),
        scale,
        baked: 2,
        strokes: 3,
        ..Default::default()
    };
    stage_frame(frame.clone());
    // 표면은 값도 받는다(잉크 영역 높이·빈 상태 안내) — 호스트의 `stage_view`와 같다.
    stage_view(view(Tool::Pen, Stage::Ready));

    // 잉크 표면만 계획한다 — 조각(타이틀바/툴바/…)과 섞이지 않게 **컴포넌트 하나**로 본다.
    let mut ctx = Ctx::new();
    let tree = elm_magic::frame::<InkSurface>(&mut ctx, &InkSurfaceProps::default());
    let (_, pass) = plan(&tree);
    assert_eq!(pass.count("Raw"), 1, "표면은 <Raw> **하나**로만 붙는다");

    let mut slot: SurfaceSlot = None;
    elm_magic::raw::invoke(&tree, &mut slot);
    // Windows의 슬롯은 `Option<View>`이고 뷰는 WinUI 스레드에서만 만들어진다 —
    // 헤드리스에서는 채울 수 없다. 그래서 **빌더가 불렸다**는 사실을 받은 재료로 확인한다.
    assert!(slot.is_none(), "헤드리스에서는 WinUI 뷰를 만들 수 없다");

    let (seen, view) = SEEN_FRAME
        .with(|slot| slot.borrow().clone())
        .expect("빌더가 받은 재료");
    assert_eq!(seen.size, frame.size);
    assert_eq!(seen.scale, frame.scale);
    assert_eq!(seen.baked, frame.baked);
    assert_eq!(seen.strokes, frame.strokes);
    assert_eq!(seen.tail.len(), frame.tail.len(), "꼬리 도형이 그대로 간다");
    // 값도 함께 간다: 잉크 영역의 높이와 빈 상태 안내가 표면 안에서 필요하다.
    assert_eq!(view.viewport, (1280.0, 800.0), "창 크기가 표면까지 온다");

    // 재료는 **프레임당 유지**된다 — 같은 발행이 여러 번 그려도 같은 것을 본다.
    SEEN_FRAME.with(|slot| *slot.borrow_mut() = None);
    let mut again: SurfaceSlot = None;
    elm_magic::raw::invoke(&tree, &mut again);
    assert!(
        SEEN_FRAME.with(|slot| slot.borrow().is_some()),
        "재료는 소비되지 않는다"
    );

    // 발행이 끝나면 비운다 — 다음 프레임이 옛 재료로 그려지면 안 된다.
    clear_frame();
    SEEN_FRAME.with(|slot| *slot.borrow_mut() = None);
    let mut empty: SurfaceSlot = None;
    elm_magic::raw::invoke(&tree, &mut empty);
    assert!(
        SEEN_FRAME.with(|slot| slot.borrow().is_none()),
        "재료가 없으면 빌더도 불리지 않는다"
    );

    clear_surface_builder();
    clear_view();
}

#[test]
fn a_surface_without_a_builder_leaves_the_slot_empty() {
    // 호스트가 빌더를 등록하지 않은 상태(테스트/헤드리스)에서도 화면은 계획까지 만들어져야 한다.
    clear_frame();
    clear_surface_builder();
    stage_frame(Frame::default());

    let mut ctx = Ctx::new();
    let tree = elm_magic::frame::<InkSurface>(&mut ctx, &InkSurfaceProps::default());
    let mut slot: SurfaceSlot = None;
    elm_magic::raw::invoke(&tree, &mut slot);
    assert!(slot.is_none());
    clear_frame();
}

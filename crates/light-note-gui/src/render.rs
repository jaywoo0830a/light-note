//! ④ Render — 화면에 그리는 **유일한** 곳.
//!
//! 얼굴이 두 개다:
//! - **UI 스레드**([`surface`]): ③의 상태를 WinUI 트리로 옮긴다. 만드는 도형 수는
//!   `canvas.live_shapes()`가 정하고 **예산 상한**이 있다 — 잉크가 쌓여도 늘지 않는다.
//! - **워커 스레드**([`bake`]): 페이지를 픽스맵+PNG로 굽는다. **페이지 크기에 비례하는 일은
//!   이 함수뿐**이고, UI 스레드는 결과를 기다리지 않는다(R2: latest-wins).
//!
//! 트리 모양 (windows-reactor 0.100의 사실 + 예제 08):
//!
//! ```text
//! Grid            가속기만 담는다 — Ctrl+더하기/빼기/Enter
//! └ Border        **포인터 이벤트는 Border에만 있다** + 종이 배경
//!   └ Canvas      유일한 절대 좌표 컨테이너
//!     ├ Image #0  베이스 A — 보이는 자리만 종이 위(0,0), 나머지는 창 밖
//!     ├ Image #1  베이스 B
//!     └ Canvas…   라이브 획 하나 = 한 합성 그룹(불투명 도형 + 그룹 투명도)
//!       ├ Line…   몸통 — 늘린 구간(관절을 덮는다)
//!       └ Ellipse… 둥근 캡 (WinUI `Line`에는 캡 속성이 없고 `Path`는 아예 없다)
//! ```

use std::rc::Rc;

use elm_magic_windows_reactor::RawSlot;
use windows_reactor::{
    AcceleratorKey, AcceleratorModifiers, Border, Brush, Canvas, CanvasChildExt, ChildrenControl,
    Color, ContentControl, CornerRadius, Ellipse, EncodedImage, Grid, Image, KeyAccelerator,
    KeyAccelerators, KeyedView, LayoutControl, Line, PointerEventInfo, Stretch, ThemeBrush,
    Thickness, View,
};

use crate::canvas::{BakeRequest, BakeResult, Base};
use crate::input::{InputSink, Phase};
use crate::shape::{self, LiveCap, LiveInk, LiveLine};
use crate::ui::{Frame, Intent};

/// 숨은 그림을 밀어 두는 창 밖 좌표 — 어떤 창 크기에서도 보이지 않는다.
const PARKED: f64 = -10_000.0;

/// 디코드 완료 싱크 — 화면 밖 그림이 준비됐다고 호스트에 알린다.
///
/// **깜빡임 방지의 절반**이다: 호스트는 이 신호를 받고서야 앞뒤를 맞바꾼다([`Base::promote`]).
pub type ImageSink = Rc<dyn Fn(usize)>;

/// ③의 상태 → WinUI 트리. `<Raw>`가 [`crate::ui::surface_builder`]로 부른다.
pub fn surface(
    frame: &Frame,
    sink: &InputSink,
    decoded: &ImageSink,
    accelerators: KeyAccelerators,
) -> RawSlot {
    let (width, height) = frame.scale.pixels(frame.size);
    let base = &frame.base;

    // 베이스 두 장은 **항상 트리에 있다** — 마운트/언마운트로 깜빡이지 않는다.
    let mut children: Vec<KeyedView> = (0..2)
        .map(|index| {
            KeyedView::new(
                format!("base-{index}"),
                base_view(base, index, width, height, decoded),
            )
        })
        .collect();

    // 라이브 획 — 꼬리(안 구운 확정 획) + 진행 중 획. 획마다 한 그룹.
    for (index, ink) in frame.tail.iter().enumerate() {
        children.push(KeyedView::new(
            format!("tail-{index}"),
            live_view(width, height, ink),
        ));
    }
    if let Some(ink) = frame.drawing.as_deref() {
        children.push(KeyedView::new("drawing", live_view(width, height, ink)));
    }

    let canvas = Canvas::new()
        .width(width as f64)
        .height(height as f64)
        .keyed_children(children);

    let pressed = Rc::clone(sink);
    let moved = Rc::clone(sink);
    let released = Rc::clone(sink);
    let lost = Rc::clone(sink);
    let canceled = Rc::clone(sink);

    let paper = Border::new()
        .background(Brush::from(ThemeBrush::SolidBackground))
        .border_brush(Brush::from(ThemeBrush::CardStroke))
        .border_thickness(Thickness::uniform(1.0))
        .corner_radius(CornerRadius::uniform(4.0))
        .padding(Thickness::uniform(0.0))
        .capture_pointer_on_press(true)
        .on_pointer_pressed(move |info: PointerEventInfo| pressed(Phase::Pressed, info.x, info.y))
        .on_pointer_moved(move |info: PointerEventInfo| moved(Phase::Moved, info.x, info.y))
        .on_pointer_released(move |info: PointerEventInfo| {
            released(Phase::Released, info.x, info.y)
        })
        // 취소는 두 경로로 온다 — 둘 다 하나로 모은다.
        .on_pointer_capture_lost(move || lost(Phase::Canceled, 0.0, 0.0))
        .on_pointer_canceled(move || canceled(Phase::Canceled, 0.0, 0.0))
        .content(canvas);

    Some(
        Grid::new()
            .key_accelerators(accelerators)
            .children((paper,)),
    )
}

/// 베이스 한 장 → WinUI `Image`.
///
/// **보이는 자리만 종이 위(0,0)** 에 있고 나머지는 창 밖으로 밀려 있다. 크기를 0으로 줄이지
/// 않는다 — 숨은 그림도 페이지 크기 그대로라 디코드와 `ImageOpened`가 정상으로 온다.
fn base_view(base: &Base, index: usize, width: u16, height: u16, decoded: &ImageSink) -> Image {
    let (left, top) = if base.visible_index() == index {
        (0.0, 0.0)
    } else {
        (PARKED, PARKED)
    };
    let on_opened = Rc::clone(decoded);
    Image::new()
        // `Arc<[u8]>`를 그대로 넘긴다 — 어댑터가 `Arc` 동일성으로 비교하므로 그림이 그대로면
        // 소스를 다시 설정하지 않는다(디코드도, 빈 프레임도, 복사도 없다).
        .source_data(EncodedImage::new(
            base.png(index).cloned().unwrap_or_else(shape::blank_png),
        ))
        .stretch(Stretch::None)
        .width(width as f64)
        .height(height as f64)
        .canvas_left(left)
        .canvas_top(top)
        .on_opened(move || on_opened(index))
}

/// 라이브 획 하나 → WinUI 도형 묶음 = **한 합성 그룹**.
///
/// 자식은 **불투명 색**으로 그리고 투명도는 그룹이 맡는다 — WinUI는 자식을 먼저 합성한 뒤
/// 투명도를 한 번만 적용하므로 겹친 캡·관절이 두 번 곱해지지 않는다. 래스터가 한 획을
/// **한 번의 채움**으로 그리는 것과 같은 결과다(형광펜이 얼룩지지 않는다).
fn live_view(width: u16, height: u16, ink: &LiveInk) -> View {
    let rgb = ink.solid().to_rgb();
    let mut children: Vec<KeyedView> = Vec::with_capacity(ink.shape_count());
    for (index, line) in ink.lines.iter().enumerate() {
        children.push(KeyedView::new(
            format!("line-{index}"),
            line_view(line, rgb),
        ));
    }
    for (index, cap) in ink.caps.iter().enumerate() {
        children.push(KeyedView::new(format!("cap-{index}"), cap_view(cap, rgb)));
    }
    Canvas::new()
        .width(width as f64)
        .height(height as f64)
        .canvas_left(0.0)
        .canvas_top(0.0)
        .opacity(ink.opacity())
        .keyed_children(children)
}

/// 라이브 선분 하나 → WinUI `Line`.
fn line_view(line: &LiveLine, rgb: (u8, u8, u8)) -> Line {
    let (red, green, blue) = rgb;
    Line::new()
        .x1(line.x1)
        .y1(line.y1)
        .x2(line.x2)
        .y2(line.y2)
        .stroke(Brush::from(Color::rgb(red, green, blue)))
        .stroke_thickness(line.width)
}

/// 둥근 캡(또는 점 하나) → WinUI `Ellipse` — 래스터의 캡과 **같은 반지름**.
fn cap_view(cap: &LiveCap, rgb: (u8, u8, u8)) -> Ellipse {
    let (red, green, blue) = rgb;
    let diameter = cap.radius * 2.0;
    Ellipse::new()
        .fill(Brush::from(Color::rgb(red, green, blue)))
        .width(diameter)
        .height(diameter)
        .canvas_left(cap.x - cap.radius)
        .canvas_top(cap.y - cap.radius)
}

// ── 워커: 페이지를 굽는다 ────────────────────────────────────────────

/// **④-워커 본체** — 페이지 하나를 투명 PNG로 굽는다. UI 스레드에서는 부르지 않는다.
///
/// 돌려주는 값은 `Send`라서 메시지로 UI 스레드에 온다(호스트가 `spawn_background`로 돌린다).
pub fn bake(request: &BakeRequest) -> BakeResult {
    BakeResult {
        id: request.id,
        page: request.page,
        scale: request.scale,
        count: request.count,
        png: shape::bake(&request.strokes, request.size, request.scale),
    }
}

/// Reactor 0.100이 지원하는 가속기 — 지원 키가 적어서 **확대/축소/페이지 추가**만 붙인다.
///
/// (`AcceleratorKey`에는 `R`, NumPad, Add/Subtract, Enter만 있다. Ctrl+Z 같은 조합은
/// elm의 `on_key`로 붙인다 — 예제 12.)
pub fn accelerators(on_intent: impl Fn(Intent) + Clone + 'static) -> KeyAccelerators {
    let zoom_in = on_intent.clone();
    let zoom_out = on_intent.clone();
    let page_add = on_intent;
    KeyAccelerators::new([
        KeyAccelerator::new(
            AcceleratorKey::Add,
            AcceleratorModifiers::Control,
            move || zoom_in(Intent::ZoomIn),
        ),
        KeyAccelerator::new(
            AcceleratorKey::Subtract,
            AcceleratorModifiers::Control,
            move || zoom_out(Intent::ZoomOut),
        ),
        KeyAccelerator::new(
            AcceleratorKey::Enter,
            AcceleratorModifiers::Control,
            move || page_add(Intent::PageAdd),
        ),
    ])
}

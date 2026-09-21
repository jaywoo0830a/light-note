//! `<Raw>` 표면 — WinUI 컨트롤 트리로 잉크를 그린다.
//!
//! ```text
//! Grid            가속기만 담는다 (Ctrl+더하기/빼기/Enter)
//! └ Border        **포인터 이벤트는 Border에만 있다** + 종이 배경
//!   └ Canvas      절대 좌표 컨테이너 (CanvasChildExt)
//!     ├ Image     정적 레이어: 확정 스트로크 PNG (투명)
//!     └ Line…     라이브 레이어: 진행 중인 획의 선분들
//! ```
//!
//! ## windows-reactor 0.100.0의 사실에 맞춘 이유
//! - **포인터 이벤트는 `Border`에만 있다**(`on_pointer_pressed/moved/released/capture_lost`,
//!   `on_pointer_canceled`, `capture_pointer_on_press`). `Canvas`·`StackPanel`에는 없다 —
//!   그래서 `Border`가 입력을 받고 `Canvas`가 그림을 담는다. **취소는 두 경로로 온다**:
//!   포인터 캡처 상실(`capture_lost`)과 시스템 취소(`canceled`) — 둘 다
//!   `PointerPhase::Canceled`로 모여 진행 중인 획을 되돌린다(모델이 한 점만 남기지 않도록).
//! - **`Canvas`가 유일한 절대 좌표 컨테이너**다(`CanvasChildExt::canvas_left/canvas_top`).
//! - **가속기는 `Grid`에만 붙는다**(`Grid::key_accelerators`). 높이가 내용만큼이라
//!   레이아웃에 영향을 주지 않는다.
//! - 확정 레이어는 **드래그가 끝날 때만** 다시 만들어지고(코어의 [`InkSurface`] 계약),
//!   라이브 레이어는 선분 몇 개만 갱신된다 — 잉크가 쌓여도 입력 지연이 늘지 않는다.
//! - 그 "다시 만들기"는 **백그라운드**에서 돈다(호스트가 코어의 `surface::static_layer`를
//!   워커에 맡기고 결과를 `SurfaceData::png`로 받는다). 렌더가 도착하기 전까지 방금 확정한
//!   획은 `data.lines`(라이브 레이어)에 남는다 — 그래서 획을 끝내도 화면이 멈추지 않는다.

use std::rc::Rc;

use elm_magic_windows_reactor::RawSlot;
use light_note_core::surface::SurfaceLine;
use light_note_core::ui::SurfaceData;
use windows_reactor::{
    AcceleratorKey, AcceleratorModifiers, Border, Brush, Canvas, CanvasChildExt, ChildrenControl,
    Color, ContentControl, CornerRadius, EncodedImage, Grid, Image, KeyAccelerator, KeyAccelerators,
    KeyedView, LayoutControl, Line, PointerEventInfo, Stretch, ThemeBrush, Thickness,
};

/// 포인터 위상 — elm과 무관한 WinUI 입력 단계.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerPhase {
    Pressed,
    Moved,
    Released,
    Canceled,
}

/// 호스트가 표면에 넘기는 포인터 싱크 — 자기 메시지 큐로 옮긴다.
///
/// `fn`이 아니라 `Rc<dyn Fn>`인 이유: 빌더가 **호스트의 `LocalSender`를 캡처**한다
/// (elm `<Raw>`는 슬롯/props를 볼 수 없으므로 캡처가 유일한 통로다 — 예제 08).
pub type PointerSink = Rc<dyn Fn(PointerPhase, f64, f64)>;

/// 표면을 만든다 — `<Raw>`가 [`light_note_core::ui::surface_builder`]로 부른다.
pub fn build(data: &SurfaceData, sink: &PointerSink, accelerators: KeyAccelerators) -> RawSlot {
    let mut children: Vec<KeyedView> = Vec::with_capacity(data.lines.len() + 1);

    if let Some(png) = data.png.as_ref() {
        children.push(KeyedView::new(
            "static-layer",
            Image::new()
                .source_data(EncodedImage::new(png.clone()))
                .stretch(Stretch::None)
                .width(data.width as f64)
                .height(data.height as f64)
                .canvas_left(0.0)
                .canvas_top(0.0),
        ));
    }

    for (index, line) in data.lines.iter().enumerate() {
        children.push(KeyedView::new(format!("live-{index}"), line_view(line)));
    }

    let canvas = Canvas::new()
        .width(data.width as f64)
        .height(data.height as f64)
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
        .on_pointer_pressed(move |info: PointerEventInfo| {
            pressed(PointerPhase::Pressed, info.x, info.y)
        })
        .on_pointer_moved(move |info: PointerEventInfo| {
            moved(PointerPhase::Moved, info.x, info.y)
        })
        .on_pointer_released(move |info: PointerEventInfo| {
            released(PointerPhase::Released, info.x, info.y)
        })
        .on_pointer_capture_lost(move || lost(PointerPhase::Canceled, 0.0, 0.0))
        .on_pointer_canceled(move || canceled(PointerPhase::Canceled, 0.0, 0.0))
        .content(canvas);

    Some(
        Grid::new()
            .key_accelerators(accelerators)
            .children((paper,)),
    )
}

/// 라이브 선분 하나 → WinUI `Line`.
fn line_view(line: &SurfaceLine) -> Line {
    let (red, green, blue) = line.color.to_rgb();
    Line::new()
        .x1(line.x1)
        .y1(line.y1)
        .x2(line.x2)
        .y2(line.y2)
        .stroke(Brush::from(Color::rgb(red, green, blue)))
        .stroke_thickness(line.width)
}

/// Reactor 0.100이 지원하는 가속기 — 지원 키가 적어서 **확대/축소/페이지 추가**만 붙인다.
///
/// (`AcceleratorKey`는 `R`, NumPad, Add/Subtract, Enter만 있다. Ctrl+Z 같은 조합은
/// 어댑터가 `ElmView` 핸들을 노출하면 `drive::dispatch_key`로 붙일 수 있다 — README 참고.)
pub fn default_accelerators(on_intent: impl Fn(&'static str) + Clone + 'static) -> KeyAccelerators {
    let zoom_in = on_intent.clone();
    let zoom_out = on_intent.clone();
    let page_add = on_intent;
    KeyAccelerators::new([
        KeyAccelerator::new(
            AcceleratorKey::Add,
            AcceleratorModifiers::Control,
            move || zoom_in("zoom_in"),
        ),
        KeyAccelerator::new(
            AcceleratorKey::Subtract,
            AcceleratorModifiers::Control,
            move || zoom_out("zoom_out"),
        ),
        KeyAccelerator::new(
            AcceleratorKey::Enter,
            AcceleratorModifiers::Control,
            move || page_add("page_add"),
        ),
    ])
}

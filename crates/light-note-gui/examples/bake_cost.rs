//! 계측 도구 — 파이프라인 한 바퀴에 드는 비용(필기감의 근거).
//!
//! 재는 것:
//! - **베이크**(④-워커) — [`render::bake`]: 페이지 **전체**를 래스터화하고 PNG로 인코딩한다.
//!   워커에서 도는 비용이라 UI는 기다리지 않는다(R2) — 그래도 얼마인지 알아야
//!   "왜 워커로 뺐는가"를 숫자로 판단할 수 있다.
//! - **라이브 도형**(④-UI) — [`shape::live_ink`]: 포인터가 움직일 때마다 UI 스레드에서.
//!   획 하나만 다시 만들고, 도형 수는 [`LIVE_SHAPE_BUDGET`]으로 묶인다(R1).
//! - **프레임 비용** — [`Canvas::frame`]/[`Canvas::refresh`]: 매 프레임 도는 부수 비용.
//!
//! 실행: `cargo run --release -p light-note-gui --example bake_cost`
//!
//! **debug(opt-level 0)는 20배 느리다** — 필기감을 판단할 때는 `--release`로 돌려라.
//!
//! [`LIVE_SHAPE_BUDGET`]: light_note_gui::canvas::LIVE_SHAPE_BUDGET

use std::time::Instant;

use light_note_gui::canvas::{Canvas, LIVE_SHAPE_BUDGET};
use light_note_gui::geom::{Pt, Scale, Size};
use light_note_gui::ink::{InkPoint, Style, Tool};
use light_note_gui::{render, shape};

/// 한 번 실행하고 걸린 시간(ms)을 출력한다.
fn time<T>(label: &str, mut body: impl FnMut() -> T) -> f64 {
    let start = Instant::now();
    let value = body();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("{label:<44} {ms:>9.1} ms");
    std::hint::black_box(value);
    ms
}

/// A4 한 장에 획 `count`개를 **모델에 직접** 넣은 캔버스.
///
/// ②(도구)를 거치지 않는 이유: 여기서 재는 것은 렌더 비용이지 제스처가 아니다.
fn canvas_with(count: usize) -> Canvas {
    let mut canvas = Canvas::new(Size::A4);
    for index in 0..count {
        let y = 60.0 + (index % 60) as f32 * 12.0;
        let mut stroke = shape_stroke(Pt::new(60.0, y));
        for step in 1..80 {
            stroke.push(InkPoint::new(
                Pt::new(60.0 + step as f32 * 6.0, y + (step % 7) as f32),
                0.4 + (step % 6) as f32 * 0.1,
            ));
        }
        canvas.doc_mut().commit_stroke(0, stroke);
    }
    canvas
}

/// 점 하나로 시작한 펜 획.
fn shape_stroke(first: Pt) -> light_note_gui::ink::Stroke {
    light_note_gui::ink::Stroke::new(
        Tool::Pen,
        Style::for_tool(Tool::Pen),
        InkPoint::new(first, 1.0),
    )
}

fn main() {
    let scale = Scale::default();
    let (width, height) = scale.pixels(Size::A4);
    println!(
        "A4 @ scale {} = {width}x{height} px ({:.2} Mpx)\n",
        scale.get(),
        (width as f64 * height as f64) / 1e6
    );

    println!("── 베이크: 획을 확정할 때 ④-워커가 하는 일 ──");
    for count in [1usize, 5, 20, 60, 300] {
        let mut canvas = canvas_with(count);
        let label = format!("  render::bake — {count}획 (래스터+PNG)");
        time(&label, || {
            let request = canvas.bake_request();
            render::bake(&request)
        });
    }

    println!("\n── 베이크 안쪽 분해(20획) ──");
    let strokes = canvas_with(20).doc().strokes().to_vec();
    let pixmap = shape::render_ink(&strokes, Size::A4, scale);
    time("  shape::render_ink (잉크 래스터)", || {
        shape::render_ink(&strokes, Size::A4, scale)
    });
    time("  shape::to_png (PNG 인코딩)", || {
        shape::to_png(shape::render_ink(&strokes, Size::A4, scale))
    });
    time("  shape::ink_coverage (전체 픽셀 스캔)", || {
        shape::ink_coverage(&pixmap)
    });
    time("  shape::render_on_white (내보내기 기준)", || {
        shape::render_on_white(&strokes, Size::A4, None, scale)
    });

    println!("\n── 라이브 도형: 포인터가 움직일 때 ④-UI가 하는 일 ──");
    let mut long = shape_stroke(Pt::new(60.0, 400.0));
    for step in 1..600 {
        long.push(InkPoint::new(
            Pt::new(60.0 + step as f32, 400.0 + (step % 9) as f32),
            1.0,
        ));
    }
    let ink = shape::live_ink(&long, scale);
    println!("  600점 획 → 도형 {}개", ink.shape_count());
    time("  shape::live_ink — 600점", || {
        shape::live_ink(&long, scale)
    });
    let tap = shape_stroke(Pt::new(100.0, 100.0));
    time("  shape::live_ink — 점 1개(탭)", || {
        shape::live_ink(&tap, scale)
    });

    println!("\n── 프레임마다 도는 부수 비용 ──");
    let mut canvas = canvas_with(20);
    time(
        "  Canvas::bake_request (워커에 넘길 스냅샷)",
        || canvas.bake_request(),
    );
    time("  Canvas::frame (④에 넘길 재료)", || canvas.frame());
    let frame = canvas.frame();
    time("  Frame clone (프레임마다 Rc 복사)", || {
        frame.clone()
    });
    let mut fresh = canvas_with(20);
    time("  Canvas::refresh — 꼬리 20획 재계산", || {
        fresh.refresh(None)
    });
    time("  Canvas::refresh — 진행 중 획만 갱신", || {
        fresh.refresh(Some(&long))
    });
    println!(
        "  구운 접두사 {}획 · 꼬리 도형 {}개 (예산 {})",
        fresh.baked_count(),
        fresh.live_shapes(),
        LIVE_SHAPE_BUDGET
    );
}

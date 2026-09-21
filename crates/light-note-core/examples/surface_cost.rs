//! 계측 도구 — 표면 한 장에 드는 비용(필기감의 근거).
//!
//! UI 스레드에서 도는 두 가지를 잰다:
//! - **정적 레이어 재생성**(`InkSurface::build`) — 획을 확정할 때 한 번. 페이지 전체를
//!   CPU로 래스터화하고 PNG로 인코딩한다.
//! - **라이브 선분**(`live_lines`) — 포인터가 움직일 때마다. 진행 중인 획 하나만.
//!
//! 실행: `cargo run --release -p light-note-core --example surface_cost`

use std::time::Instant;

use light_note_core::doc::Document;
use light_note_core::geom::Size;
use light_note_core::ink::{InkPoint, StrokeStyle, Tool};
use light_note_core::raster::{self, ViewTransform};
use light_note_core::surface::{live_lines, pending_lines, InkSurface};
use light_note_core::ui::NoteViewModel;

/// 한 번 실행하고 걸린 시간(ms)을 출력한다.
fn time<T>(label: &str, mut body: impl FnMut() -> T) -> f64 {
    let start = Instant::now();
    let value = body();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!("{label:<44} {ms:>9.1} ms");
    std::hint::black_box(value);
    ms
}

/// A4 한 장에 획 `count`개를 그린 문서.
fn page_with(count: usize) -> Document {
    let mut document = Document::blank(Size::A4);
    for index in 0..count {
        let y = 60.0 + (index % 60) as f32 * 12.0;
        document.begin(
            Tool::Pen,
            StrokeStyle::for_tool(Tool::Pen),
            InkPoint::new(60.0, y),
        );
        for step in 1..80 {
            document.extend(InkPoint::new(
                60.0 + step as f32 * 6.0,
                y + (step % 7) as f32,
            ));
        }
        document.finish();
    }
    document
}

fn main() {
    let scale = ViewTransform::DEFAULT_SCALE;
    let (width, height) = raster::page_pixel_size(Size::A4, scale);
    println!(
        "A4 @ scale {scale} = {width}x{height} px ({:.2} Mpx)\n",
        (width as f64 * height as f64) / 1e6
    );

    println!("── 정적 레이어(획 확정 때 UI 스레드에서) ──");
    for count in [1usize, 5, 20, 60] {
        let document = page_with(count);
        let committed = document.committed_strokes();
        let label = format!("InkSurface::build — {count}획 (래스터+PNG)");
        time(&label, || {
            InkSurface::build(Size::A4, scale, committed, None)
        });
    }

    println!("\n── 정적 레이어 안쪽 분해(20획) ──");
    let document = page_with(20);
    let committed = document.committed_strokes();
    time("  raster::render_ink (잉크 래스터)", || {
        raster::render_ink(committed, Size::A4, scale)
    });
    let pixmap = raster::render_ink(committed, Size::A4, scale);
    time("  ink_coverage (전체 픽셀 스캔)", || {
        raster::ink_coverage(&pixmap)
    });
    time("  to_png(render_ink(..)) — 둘 합계", || {
        raster::to_png(raster::render_ink(committed, Size::A4, scale))
    });
    time("  render_page_on_white (여백 포함)", || {
        raster::render_page_on_white(document.active_page(), None, scale)
    });

    println!("\n── 라이브 레이어(포인터 이동마다) ──");
    let mut in_progress = page_with(1);
    in_progress.begin(
        Tool::Pen,
        StrokeStyle::for_tool(Tool::Pen),
        InkPoint::new(60.0, 400.0),
    );
    for step in 1..600 {
        in_progress.extend(InkPoint::new(60.0 + step as f32, 400.0 + (step % 9) as f32));
    }
    let live = in_progress.live_stroke().expect("진행 중인 획");
    println!("진행 중인 획: {}점", live.points().len());
    time("live_lines — 600점", || live_lines(live, scale));

    println!("\n── 획을 끝낸 직후 UI 스레드가 하는 일 ──");
    // 정적 레이어는 백그라운드로 갔으므로 UI 스레드에는 **꼬리 선분 몇 개**만 남는다.
    // (이 예제가 재는 `InkSurface::build`는 워커에서 도는 비용이다.)
    let committed = page_with(20);
    let strokes = committed.committed_strokes();
    let count = strokes.len();
    time("pending_lines — 방금 확정한 획 하나(꼬리)", || {
        pending_lines(&strokes[..count - 1], count - 1, None, scale)
    });

    println!("\n── 프레임마다 도는 부수 비용 ──");
    let document = page_with(20);
    time("NoteViewModel::from_document", || {
        NoteViewModel::from_document(&document, Tool::Pen, 1.8, 100.0)
    });
    let surface = InkSurface::build(Size::A4, scale, document.committed_strokes(), None);
    println!("정적 PNG 크기: {} KB", surface.static_bytes() / 1024);
    time("InkSurface clone(프레임마다 PNG 복사)", || {
        surface.clone()
    });
    time("stage_surface용 PNG 복사", || surface.static_png.clone());
    time("decode_static(PNG 디코드 — 참고)", || {
        surface.decode_static()
    });
}

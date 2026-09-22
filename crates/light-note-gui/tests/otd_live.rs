//! 실물 검증 — **이 PC의** OTD 플러그인이 쓰는 링을 직접 읽는다.
//!
//! `#[ignore]`인 이유: 태블릿·데몬·플러그인이 설치된 PC에서만 의미가 있다(CI에는 없다).
//! 순수 로직은 각 모듈의 단위 테스트가 이미 덮고, 여기서 확인하는 것은 **계약이 실물과
//! 맞는가** 하나다 — 특히 `magic`/`version`/오프셋이 C# 쪽과 어긋나면 여기서만 드러난다.
//!
//! ```text
//! cargo test --test otd_live -- --ignored --nocapture
//! ```

#![cfg(windows)]

use std::time::{Duration, Instant};

use light_note_gui::otd::shm;
use light_note_gui::otd::wire::TabletSpec;

#[test]
#[ignore = "OTD 데몬과 플러그인이 설치된 PC에서만 돈다"]
fn the_plugin_ring_is_where_we_think_it_is() {
    let Some(mut ring) = shm::Ring::open() else {
        panic!(
            "공유 메모리를 열 수 없다 — 플러그인을 설치했는가? \
             (OTD.SharedMemoryOutput/install.ps1 -Enable)"
        );
    };
    let mut out = Vec::new();
    let (header, _) = ring.poll(&mut out).expect("헤더가 우리 것이 아니다");
    println!("tablet    = {:?}", header.tablet_name);
    println!("write_seq = {}", header.write_seq);
    println!("heartbeat = {}", header.heartbeat);
    println!(
        "range     = X {} Y {} P {} (0이면 앱이 RPC로 채운다)",
        header.max_x, header.max_y, header.max_pressure
    );
    assert!(header.is_alive(), "플러그인이 '살아 있다'고 말하지 않는다");
    assert!(
        header.write_seq > 0,
        "표본을 하나도 쓰지 않았다 — 데몬이 태블릿 리포트를 받고 있는가?"
    );
}

#[test]
#[ignore = "OTD 데몬과 플러그인이 설치된 PC에서만 돈다"]
fn samples_arrive_and_the_cursor_advances() {
    // 3초 동안 읽어 **초당 표본 수**와 처음/마지막 표본을 보여준다(펜을 태블릿 위에 두고 실행).
    let mut ring = shm::Ring::open().expect("공유 메모리");
    let mut out = Vec::new();
    let start = Instant::now();
    let (mut first, mut last) = (None, None);
    let (mut total, mut skipped) = (0usize, 0u64);
    while start.elapsed() < Duration::from_secs(3) {
        out.clear();
        if let Some((_, readout)) = ring.poll(&mut out) {
            total += out.len();
            skipped += readout.skipped;
            for sample in &out {
                first.get_or_insert(*sample);
                last = Some(*sample);
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let seconds = start.elapsed().as_secs_f32();
    println!(
        "samples = {total} ({:.0}/s), skipped = {skipped}",
        total as f32 / seconds
    );
    if let (Some(first), Some(last)) = (first, last) {
        println!("first = {first:?}");
        println!("last  = {last:?}");
    }
    assert!(
        total > 0,
        "표본이 하나도 안 왔다 — 펜을 태블릿 위에 올려 두고 다시 실행하라"
    );
    assert_eq!(
        skipped, 0,
        "1 ms 폴링으로 밀렸다면 프레이밍이나 커서가 틀렸다"
    );
}

#[test]
#[ignore = "OTD 데몬이 돌고 있는 PC에서만 돈다"]
fn the_rpc_answers_with_the_tablet_range() {
    // 공유 메모리가 범위를 모를 때 앱이 부르는 바로 그 경로다.
    let spec: TabletSpec = light_note_gui::otd::pipe::tablet_spec().expect("태블릿 범위");
    println!("{spec:?}");
    assert!(spec.max_x > 0.0 && spec.max_y > 0.0 && spec.max_pressure > 0.0);
}

#[test]
#[ignore = "OTD 데몬과 플러그인이 설치된 PC에서만 돈다"]
fn the_session_thread_delivers_samples_and_the_range() {
    // **앱이 쓰는 바로 그 경로**다(전용 스레드 + mpsc). 앱에서 표본이 0으로 보이는 일이
    // 있었다 — 그때 이 테스트가 어디서 멈추는지 말해 준다.
    let stream = light_note_gui::otd::session::start();
    let deadline = Instant::now() + Duration::from_secs(5);
    let (mut samples, mut live, mut spec, mut problem) = (0usize, None, None, None);
    // **앱과 같은 패턴**이다: 통로를 복제해 백그라운드로 넘기고, 묶음마다 **다시 건다**.
    // (수신자를 옮겨 버리면 첫 묶음 뒤에 멈춘다 — 그 실수를 이 루프가 잡는다.)
    while Instant::now() < deadline {
        let waiting = stream.clone();
        let batch = std::thread::spawn(move || waiting.next(Duration::from_millis(50)))
            .join()
            .expect("대기 스레드");
        samples += batch.samples.len();
        live = batch.live.or(live);
        spec = batch.spec.or(spec);
        problem = batch.problem.or(problem);
    }
    println!("samples = {samples}");
    println!("live    = {live:?}");
    println!("spec    = {spec:?}");
    println!("problem = {problem:?}");
    assert!(live.is_some(), "연결조차 못 했다 — 플러그인이 없는가?");
    assert!(
        samples > 0,
        "표본이 안 온다 — 펜을 태블릿 위에 두고 다시 실행하라"
    );
    assert!(spec.is_some(), "태블릿 범위가 안 온다: {problem:?}");
}

#[test]
#[ignore = "OTD 데몬과 플러그인이 설치된 PC에서만 돈다"]
fn the_ring_cursor_advances() {
    // 붙기(첫 폴링은 커서만 맞춘다) → 1초 뒤 다시 폴링. 여기서 `read`가 0이면 커서나
    // 표식이 어긋난 것이다 — 어느 쪽인지 숫자로 보이게 둔다.
    let mut ring = shm::Ring::open().expect("공유 메모리");
    let mut out = Vec::new();
    let (first, readout) = ring.poll(&mut out).expect("헤더가 우리 것이 아니다");
    println!(
        "attach: write_seq={} capacity={} sample_size={} read={} skipped={}",
        first.write_seq, first.capacity, first.sample_size, readout.read, readout.skipped
    );
    std::thread::sleep(Duration::from_millis(1000));
    out.clear();
    let (second, readout) = ring.poll(&mut out).expect("헤더가 우리 것이 아니다");
    println!(
        "poll:   write_seq={} read={} skipped={}",
        second.write_seq, readout.read, readout.skipped
    );
    println!("first sample = {:?}", out.first());
    assert!(
        second.write_seq > first.write_seq,
        "플러그인이 표본을 쓰지 않고 있다 — 데몬이 리포트를 받고 있는가?"
    );
    assert!(readout.read > 0, "새 표본이 있는데 못 읽었다");
}

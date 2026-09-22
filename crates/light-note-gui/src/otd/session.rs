//! 표본 스트림 — **전용 스레드 하나**가 링을 읽어 UI로 보낸다.
//!
//! ## 왜 전용 스레드인가
//! 폴링(1 ms)과 RPC 호출(`GetTablets`)은 **기다리는 일**이다. UI 스레드에서 하면 필기가 멈추고,
//! 리액터의 백그라운드 풀에서 하면 오래 붙들게 된다(그 풀은 ④-워커의 베이크도 쓴다) →
//! 그래서 전용 스레드다. UI로 가는 것은 `mpsc` 하나뿐이고, UI 쪽은 리액터의
//! `spawn_background`로 **한 묶음씩** 받는다(짧은 타임아웃으로 끊어서 풀을 오래 잡지 않는다).
//!
//! ## 왜 1 ms 폴링인가 (그리고 왜 고해상도 타이머인가)
//! 400 Hz 표본은 2.5 ms마다 온다: 폴링이 느리면 표본이 늦게 도착해 그리는 느낌이 밀리고,
//! 빠르면 CPU를 태운다. `Sleep(1)`은 **기본 타이머 해상도(약 15.6 ms)**에 묶여 15 ms가 된다 —
//! `CreateWaitableTimerExW`의 `HIGH_RESOLUTION` 플래그를 쓰면 1 ms가 실제로 1 ms다.
//! 시스템 전체 해상도를 바꾸는 `timeBeginPeriod`는 쓰지 않는다(다른 앱의 전력에도 영향이 간다).
//!
//! ## 태블릿 범위는 왜 RPC로 묻는가
//! 공유 메모리 헤더의 `max_*`는 **플러그인이 모르면 0**이다(필터는 `IOutputMode.Tablet`에
//! 접근할 수 없다). 범위가 없으면 좌표를 페이지로 옮길 수 없으므로 그때만
//! [`crate::otd::pipe::tablet_spec`]을 부른다 — 한 번 성공하면 다시 묻지 않는다.
//! 실패하면 **점점 드물게** 다시 시도한다(1 → 2 → 4 → 8초): 데몬이 없는 사용자에게
//! 1초마다 파이프를 여는 것은 소음이다.
//!
//! **이 일은 별도 스레드가 한다**: RPC는 데몬이 대답하지 않으면 멈출 수 있고, 폴링 스레드가
//! 거기 걸리면 **펜까지 멈춘다**(실제로 겪었다). 그래서 스레드가 둘이고, 진단 표의 `Polls`가
//! 폴링이 살아 있는지를 숫자로 보여준다.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::otd::pipe;
use crate::otd::shm::{self, PenSample};
use crate::otd::wire::TabletSpec;

/// 표본을 폴링하는 간격(ms) — 표본 간격(400 Hz = 2.5 ms)보다 짧아야 한다.
const POLL_MS: u32 = 1;
/// 플러그인이 없을 때 다시 찾는 간격(ms).
const RETRY_MS: u32 = 1000;
/// 범위(RPC) 재시도의 최대 간격(ms) — 실패할수록 물러난다.
const SPEC_RETRY_MAX_MS: u32 = 8000;
/// 상태만 알리는 주기(폴링 횟수) — 표본이 없어도 화면이 최신이게.
const STATUS_EVERY: u32 = 250;

/// 폴링 간격은 표본 간격(400 Hz = 2.5 ms)보다 **짧아야** 한다 — 아니면 표본이 늦게 온다.
/// 컴파일 시점에 못박는다(상수라 런타임 테스트는 늘 참이 된다).
const _: () = assert!(POLL_MS < 2, "폴링이 표본 간격보다 느리다");
/// 상태 알림이 0이면 화면이 낡은 값을 붙들고 있게 된다.
const _: () = assert!(STATUS_EVERY >= 1, "상태 알림 주기가 0이다");

/// UI로 보내는 한 묶음 — **`Send`한 값만** 든다(문서·도구는 넘어오지 않는다).
#[derive(Clone, Debug, Default)]
pub struct Batch {
    /// 이번에 꺼낸 표본(빈 벡터면 상태만 온 것이다).
    pub samples: Vec<PenSample>,
    /// 연결됐으면 지금 상태.
    pub live: Option<Live>,
    /// 이번에 **알아낸** 태블릿 범위(공유 메모리가 0일 때 RPC로 채운다).
    pub spec: Option<TabletSpec>,
    /// 연결·RPC가 실패한 이유 — 진단 표에 그대로 올린다(문장이 아니라 사실이다).
    pub problem: Option<String>,
}

/// 살아 있는 연결의 상태 — 사람이 읽는 문장은 UI가 만든다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Live {
    /// 플러그인이 알려준 태블릿 이름(모르면 빈 문자열).
    pub tablet: String,
    /// 플러그인이 살아 있는가(헤더의 비트).
    pub alive: bool,
    /// 지금까지 기록한 표본 수 — 이 수가 멈추면 스트림이 멈춘 것이다.
    pub write_seq: u64,
    /// 링이 밀려서 **못 꺼낸** 수(0이 아니면 앱이 못 따라간 순간이 있었다).
    pub skipped: u64,
    /// 폴링 횟수 — **이 수가 멈추면 스레드가 멈춘 것**이다(진단의 마지막 근거).
    pub ticks: u64,
}

/// "다시 연결하라"를 보내는 손잡이 — [`Stream`]이 대기 중이어도 쓸 수 있다(복제 가능).
///
/// 손잡이가 **둘**인 이유: 링(플러그인)과 범위(RPC)를 **다른 스레드**가 맡는다. 하나를 쓰면
/// 한쪽이 가져간 요청을 다른 쪽이 못 본다 — 버튼 하나로 둘 다 다시 보게 한다.
#[derive(Clone, Default)]
pub struct Kick {
    ring: Arc<AtomicBool>,
    range: Arc<AtomicBool>,
}

impl Kick {
    /// 링과 범위를 **둘 다** 다시 시도하게 한다.
    pub fn kick(&self) {
        self.ring.store(true, Ordering::Relaxed);
        self.range.store(true, Ordering::Relaxed);
    }
}

/// 표본을 받는 통로 — UI가 하나씩 꺼내 메시지로 만든다.
///
/// **복제해도 같은 통로다**(`Arc<Mutex<…>>`): 대기는 한 번에 하나뿐이므로 잠금이 경합하지
/// 않는다. 이 구조가 필요한 이유는 실패에서 배웠다: 수신자를 백그라운드 클로저로 **옮기면**
/// 첫 묶음 뒤에 다시 기다릴 수 없어 화면이 그 자리에서 멈춘다(실제로 겪었다).
#[derive(Clone)]
pub struct Stream {
    batches: Arc<Mutex<Receiver<Batch>>>,
    kick: Kick,
}

impl Stream {
    /// 다음 묶음을 기다린다(최대 `timeout`).
    ///
    /// **막히지 않게 짧게 끊는다**: 리액터의 백그라운드 풀 스레드를 오래 붙들면 베이크가
    /// 밀린다. 시간이 지나면 빈 묶음이 온다(화면은 "조용하다"만 알면 된다).
    pub fn next(&self, timeout: Duration) -> Batch {
        let Ok(batches) = self.batches.lock() else {
            return Batch::default();
        };
        batches.recv_timeout(timeout).unwrap_or_default()
    }

    /// 다시 연결을 요청하는 손잡이 — UI가 복제해 둔다.
    pub fn kick_handle(&self) -> Kick {
        self.kick.clone()
    }
}

/// 스레드를 띄운다 — **프로세스가 끝날 때까지 산다**(앱이 닫히면 같이 사라진다).
///
/// 스레드가 **둘**이다: ①표본 폴링(절대 기다리지 않는다 — 펜이 최우선이다) ②태블릿 범위
/// RPC(막힐 수 있다). 하나로 합치면 데몬이 대답하지 않을 때 **펜까지 멈춘다** — 실제로 겪었다.
pub fn start() -> Stream {
    let (sender, batches) = mpsc::channel();
    let kick = Kick::default();
    let spawn = |name: &str, body: Box<dyn FnOnce() + Send>| {
        let spawned = std::thread::Builder::new()
            .name(name.to_string())
            .spawn(body);
        if let Err(error) = spawned {
            // 스레드를 못 만들면 필기가 안 된다 — 이유는 상태줄이 말한다.
            eprintln!("light-note: {name} thread failed to start: {error}");
        }
    };
    let poll_sender = sender.clone();
    let poll_kick = kick.ring.clone();
    spawn("otd-samples", Box::new(move || run(poll_sender, poll_kick)));
    let range_kick = kick.range.clone();
    spawn(
        "otd-range",
        Box::new(move || ask_for_range(sender, range_kick)),
    );
    Stream {
        batches: Arc::new(Mutex::new(batches)),
        kick,
    }
}

/// 스레드 본체 — 연결 → 폴링. **여기서는 아무것도 기다리지 않는다**(범위는 다른 스레드가 묻는다).
fn run(sender: Sender<Batch>, kick: Arc<AtomicBool>) {
    let waiter = platform::Waiter::new();
    let mut out: Vec<PenSample> = Vec::new();
    let mut ticks: u64 = 0;
    let mut since_status: u32 = 0;

    loop {
        let Some(mut ring) = shm::Ring::open() else {
            // 플러그인이 없다(또는 데몬이 꺼졌다) — 드물게 다시 본다.
            let _ = sender.send(Batch::default());
            waiter.wait(RETRY_MS);
            ticks = 0;
            since_status = 0;
            continue;
        };

        loop {
            out.clear();
            let Some((header, readout)) = ring.poll(&mut out) else {
                // 우리 매핑이 아니게 됐다(플러그인이 다시 만들었다) — 다시 연다.
                break;
            };
            ticks += 1;
            since_status += 1;

            // 표본이 왔거나, 상태를 알릴 때가 됐거나, 사용자가 **다시 연결**을 눌렀다.
            let forced = kick.swap(false, Ordering::Relaxed);
            if readout.read > 0 || forced || since_status >= STATUS_EVERY {
                since_status = 0;
                let _ = sender.send(Batch {
                    samples: out.clone(),
                    live: Some(Live {
                        tablet: header.tablet_name.clone(),
                        alive: header.is_alive(),
                        write_seq: header.write_seq,
                        skipped: readout.skipped,
                        ticks,
                    }),
                    spec: None,
                    problem: None,
                });
            }

            waiter.wait(POLL_MS);
        }
        waiter.wait(RETRY_MS);
    }
}

/// 태블릿 범위를 **따로** 묻는 스레드 — 폴링을 절대 막지 않는다.
///
/// 성공하면 다시 묻지 않고 **다시 연결 요청**만 기다린다(데몬을 재시작했거나 태블릿을 바꿔
/// 꽂았을 때 쓴다). 실패하면 점점 드물게 다시 시도한다(1 → 2 → 4 → 8초).
fn ask_for_range(sender: Sender<Batch>, kick: Arc<AtomicBool>) {
    let waiter = platform::Waiter::new();
    let mut retry_ms = RETRY_MS;
    let mut learned = false;
    loop {
        if learned && !kick.swap(false, Ordering::Relaxed) {
            // 이미 알았다 — 다시 연결 요청이 올 때까지 느긋하게 기다린다.
            waiter.wait(RETRY_MS);
            continue;
        }
        match pipe::tablet_spec() {
            Ok(spec) => {
                learned = true;
                retry_ms = RETRY_MS;
                let _ = sender.send(Batch {
                    spec: Some(spec),
                    ..Default::default()
                });
            }
            Err(reason) => {
                let _ = sender.send(Batch {
                    problem: Some(reason),
                    ..Default::default()
                });
                waiter.wait(retry_ms);
                retry_ms = (retry_ms * 2).min(SPEC_RETRY_MAX_MS);
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::SECURITY_ATTRIBUTES;
    use windows::Win32::System::Threading::{
        CreateWaitableTimerExW, SetWaitableTimer, WaitForSingleObject,
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, TIMER_ALL_ACCESS,
    };

    /// 대기용 타이머 — 스레드마다 하나. 못 만들면 `sleep`으로 내려간다(해상도만 나빠진다).
    pub struct Waiter {
        timer: HANDLE,
    }

    impl Waiter {
        pub fn new() -> Self {
            let created = unsafe {
                CreateWaitableTimerExW(
                    None::<*const SECURITY_ATTRIBUTES>,
                    PCWSTR::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS.0,
                )
            };
            Self {
                timer: created.unwrap_or_default(),
            }
        }

        /// `millis`만큼 기다린다 — 상대 시각은 **음수 100 ns 단위**다(Windows 규칙).
        pub fn wait(&self, millis: u32) {
            let due = -((millis as i64) * 10_000);
            let armed = !self.timer.is_invalid()
                && unsafe { SetWaitableTimer(self.timer, &due, 0, None, None, false).is_ok() };
            if armed {
                unsafe {
                    WaitForSingleObject(self.timer, INFINITE);
                }
                return;
            }
            // 타이머가 없으면 평범한 잠이다 — 해상도가 나쁠 뿐 필기는 된다.
            std::thread::sleep(std::time::Duration::from_millis(millis as u64));
        }
    }

    impl Drop for Waiter {
        fn drop(&mut self) {
            if !self.timer.is_invalid() {
                unsafe {
                    let _ = CloseHandle(self.timer);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    /// 다른 플랫폼에는 OTD가 없다 — 대기만 흉내 낸다(계획 층 테스트가 도는 것으로 충분하다).
    pub struct Waiter;

    impl Waiter {
        pub fn new() -> Self {
            Self
        }

        pub fn wait(&self, millis: u32) {
            std::thread::sleep(std::time::Duration::from_millis(millis as u64));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_batch_means_nothing_happened() {
        // 스트림이 조용할 때 UI가 받는 것 — 표본도 범위도 문제도 없다.
        let batch = Batch::default();
        assert!(batch.samples.is_empty());
        assert!(batch.live.is_none());
        assert!(batch.spec.is_none());
        assert!(batch.problem.is_none());
    }

    #[test]
    fn a_kick_reaches_both_handles_once() {
        // 버튼 하나가 **링과 범위 둘 다** 다시 시도하게 한다 — 그리고 요청은 한 번만 쓰인다.
        let kick = Kick::default();
        assert!(!kick.ring.load(Ordering::Relaxed));
        kick.kick();
        assert!(kick.ring.swap(false, Ordering::Relaxed));
        assert!(kick.range.swap(false, Ordering::Relaxed));
        assert!(!kick.ring.swap(false, Ordering::Relaxed), "한 번만 쓰인다");
    }
}

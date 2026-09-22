//! OTD 어댑터 — OpenTabletDriver에서 **직접** 태블릿 데이터를 받는다.
//!
//! ## 두 경로
//! - **공유 메모리(권장, 지연 ~0)**: `OTD.SharedMemoryOutput` 플러그인이 쓰는 링을 읽는다.
//!   계약은 저장소의 `OTD.SharedMemoryOutput/PROTOCOL.md` 하나뿐이다(양쪽이 그 문서를 따른다).
//! - **JSON-RPC(폴백)**: 플러그인 없이도 필기가 되어야 한다. `\\.\pipe\OpenTabletDriver.Daemon`에
//!   `Content-Length` 프레이밍으로 말하고, `DeviceReport` 알림에서 필요한 값만 표적 스캔한다.
//!
//! ## 왜 JSON-RPC가 폴백인가 (실측)
//! `DeviceReport` 알림에는 리포트마다 태블릿 전체 정보(`TabletReference`) 약 1 KB가 따라온다.
//! 이 PC에서 그 페이로드를 그대로 받아 확인했다 — 400 리포트/초면 초당 수백 KB를 직렬화·파싱한다.
//! 공유 메모리는 고정 64 B를 그대로 쓰고 읽으므로 **1000배 이상** 싸다. 그래서 순서가 이렇다:
//! 공유 메모리가 있으면 그것을 쓰고, 없으면 RPC로 내려간다(느리지만 동작한다).
//!
//! ## 지금 있는 것
//! - [`frame`] — 파이프 프레이밍(`Content-Length`)
//! - [`wire`] — RPC JSON 파싱(serde)
//! - [`map`] — 태블릿 → 페이지 맞춤
//! - [`shm`] — 플러그인이 쓰는 공유 메모리 링(핵심 경로, 지연 ~0)
//! - [`feed`] — 표본 → 위상 있는 [`crate::input::Sample`]
//! - [`pipe`] — RPC 한 번 묻기(`GetTablets` — 태블릿 범위)
//! - [`session`] — 전용 스레드가 폴링해 UI로 묶음으로 보낸다

pub mod feed;
pub mod frame;
pub mod map;
pub mod pipe;
pub mod session;
pub mod shm;
pub mod wire;

pub use feed::Feed;
pub use map::Mapper;
pub use session::{Batch, Stream};
pub use shm::{Header, PenSample};
pub use wire::{Kind, Report, TabletSpec};

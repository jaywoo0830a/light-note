//! OTD 공유 메모리 리더 — 플러그인이 쓰는 링을 **읽기만** 한다.
//!
//! 계약은 저장소의 `OTD.SharedMemoryOutput/PROTOCOL.md` **하나뿐**이다: 128 B 헤더 +
//! 4096 × 64 B 링, 슬롯 표식은 `2*seq`(완성) / `2*seq+1`(쓰는 중)이다.
//!
//! ## 왜 이 파일의 핵심이 `windows` 밖에 있는가
//! 링을 해석하는 규칙(헤더 검증·커서·seqlock)은 **바이트 배열 위의 순수 함수**다
//! ([`header`], [`Cursor::drain`]). Windows API는 [`Ring`]에만 있다 — 매핑을 열어 그 바이트를
//! 넘겨줄 뿐이다. 그래서 규칙은 WinUI도 태블릿도 없이 `cargo test`로 검증되고, 실물 검증은
//! 이 PC의 링을 직접 여는 `tests/otd_live.rs`(`--ignored`)가 한다.
//!
//! ## seqlock을 **두 번** 보는 이유
//! 계약은 "표식이 정확히 `2*seq`일 때만 채택"이다. 그런데 표식을 확인한 **뒤** 필드를 읽는
//! 동안에도 플러그인은 같은 슬롯을 덮어쓸 수 있다(링이 4096개라 보통은 아니다 — 하지만 앱이
//! 오래 멈췄다가 깨어나면 실제로 일어난다). 그래서 필드를 다 읽은 뒤 표식을 **한 번 더** 보고
//! 같을 때만 채택한다. 두 번째 검사가 없으면 서로 다른 두 표본이 섞인 점 하나가 획에 들어간다
//! — 눈에는 튀는 점 하나로 보이고, 원인은 찾기 어렵다.
//!
//! ## 처음 붙을 때 과거를 되감지 않는다
//! 링에는 이미 최대 4096개(400 Hz에서 10초)의 표본이 들어 있다. 그것을 전부 그리면 앱을 켠
//! 순간 **유령 획**이 생긴다. 그래서 커서는 처음에 `write_seq`에 맞춘다([`Cursor::drain`]).
//! 그 대가는 정직하게: 앱을 켜는 순간 펜이 이미 화면에 닿아 있었다면 그 획의 앞부분은 없다.
//!
//! ## 쓰지 않는 것 (계약에 있지만 의미가 약하거나 없는 것)
//! - `hover_distance`(v1 미사용, 플러그인이 0을 쓴다), `rotation`(0.6.7에 리포트가 없다).
//! - `flags`의 `PROXIMITY` 비트: 플러그인은 `IProximityReport` **구현 여부**로 세운다(리포트
//!   종류의 성질이지 "지금 가까이 있다"가 아니다) — 그래서 안 쓴다.
//! - `t`(QPC 100 ns): 표본의 시각은 앱이 `Instant::now()`로 잡는다. 두 시계를 섞으면 속도
//!   추정이 음수가 되는 자리가 생긴다.
//! - `tilt`: 계약이 "보고하지 않으면 0"이므로 `(0, 0)`은 **안 보고한 것**으로 본다. 진짜 0도
//!   기울기를 구분할 수 없다 — 틸트는 지금 상태 문구에만 쓰이므로 그 대가가 싸다.

/// 매핑 이름 — 플러그인과 **같은 문자열**이어야 한다(PROTOCOL.md).
pub const NAME: &str = "light-note.otd.shm";
/// .NET의 `CreateOrOpen`은 **세션 네임스페이스**에 만든다 → 이것을 먼저 시도한다.
pub const SESSION_NAME: &str = r"Local\light-note.otd.shm";

/// `"LNOTDSM1"` — 다른 매핑과 구별하는 값.
pub const MAGIC: u64 = 0x4C4E_4F54_4453_4D31;
/// 계약 버전. **다르면 읽지 않는다**(낡은 플러그인 = 없는 것과 같다).
pub const VERSION: u32 = 1;
/// 슬롯 하나의 크기.
pub const SAMPLE_SIZE: u32 = 64;
/// 헤더 크기.
pub const HEADER_SIZE: usize = 128;
/// 링의 슬롯 수 — 400 Hz에서 약 10초.
pub const CAPACITY: u32 = 4096;
/// 매핑 전체 크기 — `MapViewOfFile`에 넘기는 값.
pub const TOTAL: usize = HEADER_SIZE + CAPACITY as usize * SAMPLE_SIZE as usize;

// ── 헤더 오프셋 (PROTOCOL.md의 표 그대로) ─────────────────────────────
const OFF_MAGIC: usize = 0x00;
const OFF_VERSION: usize = 0x08;
const OFF_SAMPLE_SIZE: usize = 0x0C;
const OFF_CAPACITY: usize = 0x10;
const OFF_HEADER_FLAGS: usize = 0x14;
const OFF_WRITE_SEQ: usize = 0x18;
const OFF_MAX_X: usize = 0x28;
const OFF_MAX_Y: usize = 0x2C;
const OFF_MAX_PRESSURE: usize = 0x30;
const OFF_HEARTBEAT: usize = 0x38;
const OFF_NAME: usize = 0x40;
const NAME_BYTES: usize = 64;

// ── 슬롯 오프셋 (슬롯 시작 기준) ──────────────────────────────────────
const OFF_SLOT_SEQ: usize = 0x00;
const OFF_X: usize = 0x08;
const OFF_Y: usize = 0x0C;
const OFF_PRESSURE: usize = 0x10;
const OFF_TILT_X: usize = 0x14;
const OFF_TILT_Y: usize = 0x18;
const OFF_FLAGS: usize = 0x20;

// ── 샘플 flags 비트 ──────────────────────────────────────────────────
/// 펜이 화면에 **닿았다**(플러그인은 `pressure > 0`으로 세운다).
pub const FLAG_TIP: u32 = 1 << 0;
/// 지우개 끝(`IEraserReport.Eraser`).
pub const FLAG_ERASER: u32 = 1 << 1;
/// 태블릿 **범위를 벗어났다** — 좌표는 무효이고, 진행 중인 획을 끝내는 신호다.
pub const FLAG_OUT_OF_RANGE: u32 = 1 << 2;
/// 리포트 종류가 근접 정보를 가진다 — **의미가 약해 안 쓴다**(모듈 문서 참고).
pub const FLAG_PROXIMITY: u32 = 1 << 3;
/// 펜 옆 버튼의 시작 비트.
const BUTTON_SHIFT: u32 = 4;

/// 헤더가 말하는 것 — **매 폴링마다 다시 읽는다**(살아 있는 값이다).
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub sample_size: u32,
    pub capacity: u32,
    pub flags: u32,
    /// 지금까지 기록한 표본 수(단조 증가) — 읽는 쪽의 **유일한 커서**다.
    pub write_seq: u64,
    /// 태블릿 범위. **0이면 플러그인이 모른다** → 앱이 OTD RPC로 채운다.
    pub max_x: f32,
    pub max_y: f32,
    pub max_pressure: f32,
    /// `Stopwatch.GetTimestamp()` — 값이 변하면 플러그인이 살아 있다.
    pub heartbeat: u64,
    /// 플러그인이 알면 태블릿 이름, 모르면 빈 문자열.
    pub tablet_name: String,
}

impl Header {
    /// bit0 = 플러그인이 살아 있다.
    pub fn is_alive(&self) -> bool {
        self.flags & 1 != 0
    }

    /// 태블릿 범위를 플러그인이 아는가 — 모르면 앱이 RPC `GetTablets`로 채운다.
    pub fn has_range(&self) -> bool {
        self.max_x > 0.0 && self.max_y > 0.0 && self.max_pressure > 0.0
    }
}

/// 링에서 꺼낸 펜 표본 하나 — **앱이 쓰는 것만**.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenSample {
    /// 이 표본의 번호(플러그인의 `write_seq`).
    pub seq: u64,
    /// 태블릿 좌표(장치 단위, **변환 전**).
    pub x: f32,
    pub y: f32,
    /// 원시 필압(장치 단위, 0 = 안 닿음).
    pub pressure: f32,
    /// 틸트(도) — 플러그인이 보고했을 때만(`(0, 0)`은 "안 보고함"이다).
    pub tilt: Option<(f32, f32)>,
    /// 지우개 끝으로 쓰는 중인가.
    pub eraser: bool,
    /// 펜 옆 버튼 — 비트 0이 첫 버튼.
    pub buttons: u8,
    /// 화면에 닿았는가(`FLAG_TIP`).
    pub contact: bool,
    /// 태블릿 범위를 벗어났는가 — 이 표본의 좌표는 무효다.
    pub out_of_range: bool,
}

impl PenSample {
    /// 진행 중인 획을 끝내는 표본인가(범위 이탈).
    pub fn ends_the_stroke(&self) -> bool {
        self.out_of_range
    }
}

/// 한 번의 폴링 결과 — **앱이 세어서 화면에 보여주는** 값까지.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Readout {
    /// 이번에 본 `write_seq`.
    pub latest: u64,
    /// 꺼낸 표본 수.
    pub read: u32,
    /// 링이 밀려서 **못 꺼낸** 수. 0이 아니면 앱이 못 따라간 것이다.
    pub skipped: u64,
}

/// 매핑 바이트 → 헤더. **우리 것이 아니면 `None`**(magic·version·크기 검사).
///
/// 낡은 플러그인(다른 version)이나 남의 매핑을 읽으면 좌표가 엉뚱해진다 — 그래서 여기서
/// 세 가지를 본다: `magic`, `version`, 그리고 `capacity * sample_size`가 우리 뷰 안에 들어오는가.
pub fn header(bytes: &[u8]) -> Option<Header> {
    if read_u64(bytes, OFF_MAGIC)? != MAGIC {
        return None;
    }
    if read_u32(bytes, OFF_VERSION)? != VERSION {
        return None;
    }
    let sample_size = read_u32(bytes, OFF_SAMPLE_SIZE)?;
    // 계약보다 작은 슬롯이면 필드가 잘려 있다 — 읽으면 엉뚱한 값을 쓴다.
    if sample_size < SAMPLE_SIZE {
        return None;
    }
    let capacity = read_u32(bytes, OFF_CAPACITY)?;
    if capacity == 0 {
        return None;
    }
    // 우리 뷰가 담을 수 있는 크기인가(플러그인이 더 큰 링을 썼다면 읽지 않는다).
    let needed = HEADER_SIZE + capacity as usize * sample_size as usize;
    if needed > bytes.len() {
        return None;
    }
    let name = bytes
        .get(OFF_NAME..OFF_NAME + NAME_BYTES)?
        .split(|byte| *byte == 0)
        .next()
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .unwrap_or_default();
    Some(Header {
        sample_size,
        capacity,
        flags: read_u32(bytes, OFF_HEADER_FLAGS)?,
        write_seq: read_u64(bytes, OFF_WRITE_SEQ)?,
        max_x: read_f32(bytes, OFF_MAX_X)?,
        max_y: read_f32(bytes, OFF_MAX_Y)?,
        max_pressure: read_f32(bytes, OFF_MAX_PRESSURE)?,
        heartbeat: read_u64(bytes, OFF_HEARTBEAT)?,
        tablet_name: name,
    })
}

/// 읽는 쪽의 커서 — **이 리더의 유일한 상태**다.
///
/// `last`가 `None`이면 아직 붙지 않은 것이다: 처음 폴링에서 `write_seq`에 맞추고 **과거를
/// 되감지 않는다**(모듈 문서 참고).
#[derive(Clone, Copy, Debug, Default)]
pub struct Cursor {
    last: Option<u64>,
}

impl Cursor {
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// 지금까지 꺼낸 마지막 번호 — 진단용.
    pub fn last(&self) -> Option<u64> {
        self.last
    }

    /// 아직 안 읽은 표본을 꺼낸다. `header`는 **이번 폴링에서 읽은 것**을 넘긴다.
    ///
    /// 밀려서 건너뛴 수는 [`Readout::skipped`]로 돌려준다 — 그리는 앱에서 옛 좌표를 따라
    /// 그리면 더 나쁘므로, 버린 것을 **세어서** 화면에 보여준다.
    pub fn drain(&mut self, bytes: &[u8], header: &Header, out: &mut Vec<PenSample>) -> Readout {
        let latest = header.write_seq;
        let Some(last) = self.last else {
            // **처음 붙었다**: 링에 남은 과거는 그리지 않는다(유령 획 방지).
            self.last = Some(latest);
            return Readout {
                latest,
                read: 0,
                skipped: 0,
            };
        };
        if latest <= last {
            return Readout {
                latest,
                read: 0,
                skipped: 0,
            };
        }
        // 링이 밀려서 덮여 쓴 번호는 건너뛴다: 가장 오래 남은 번호가 `latest - capacity + 1`이다.
        let oldest = latest.saturating_sub(header.capacity as u64 - 1);
        let from = (last + 1).max(oldest);
        let mut skipped = from - last - 1;
        let mut read = 0;
        let mut last_done = last;
        for seq in from..=latest {
            if let Some(sample) = slot(bytes, header, seq) {
                out.push(sample);
                read += 1;
                last_done = seq;
                continue;
            }
            // 슬롯이 "쓰는 중"이거나(표식이 `2*seq+1`) 우리가 읽는 동안 덮여 썼다.
            if seq + header.capacity as u64 <= latest {
                // 한 바퀴를 다 돌아 덮여 썼다 — 다시 볼 기회가 없다(버린 것을 센다).
                skipped += 1;
                last_done = seq;
                continue;
            }
            // **아직 쓰는 중이다** — 커서를 여기서 멈추고 다음 폴링에서 완성된 것을 읽는다.
            // 커서를 넘겨 버리면 표본 하나를 그냥 잃는다(400 Hz에서 2.5 ms짜리 점 하나).
            break;
        }
        self.last = Some(last_done);
        Readout {
            latest,
            read,
            skipped,
        }
    }
}

/// 슬롯 하나 — **표식이 정확히 `2*seq`일 때만** 채택한다(쓰는 중이거나 덮여 썼으면 `None`).
fn slot(bytes: &[u8], header: &Header, seq: u64) -> Option<PenSample> {
    let base = HEADER_SIZE + (seq % header.capacity as u64) as usize * header.sample_size as usize;
    let marker = read_u64(bytes, base + OFF_SLOT_SEQ)?;
    if marker != seq * 2 {
        return None;
    }
    let flags = read_u32(bytes, base + OFF_FLAGS)?;
    let tilt_x = read_f32(bytes, base + OFF_TILT_X)?;
    let tilt_y = read_f32(bytes, base + OFF_TILT_Y)?;
    let sample = PenSample {
        seq,
        x: read_f32(bytes, base + OFF_X)?,
        y: read_f32(bytes, base + OFF_Y)?,
        pressure: read_f32(bytes, base + OFF_PRESSURE)?,
        // 계약: 보고하지 않으면 0이다 — `(0, 0)`을 "안 보고함"으로 본다.
        tilt: (tilt_x != 0.0 || tilt_y != 0.0).then_some((tilt_x, tilt_y)),
        eraser: flags & FLAG_ERASER != 0,
        buttons: ((flags >> BUTTON_SHIFT) & 0xFF) as u8,
        contact: flags & FLAG_TIP != 0,
        out_of_range: flags & FLAG_OUT_OF_RANGE != 0,
    };
    // **두 번째 표식**: 필드를 읽는 동안 덮여 썼으면 버린다(모듈 문서 참고).
    (read_u64(bytes, base + OFF_SLOT_SEQ)? == marker).then_some(sample)
}

/// `N`바이트를 **휘발성으로** 읽는다.
///
/// `[u8; N]`의 정렬 요구는 1이라 어떤 오프셋에서도 안전하다(슬롯 안의 f32/u64 오프셋이
/// 8의 배수가 아닐 수 있는 것과 무관하다). 휘발성인 이유: 이 바이트는 **다른 프로세스가
/// 계속 바꾸는** 공유 메모리다 — 컴파일러가 값을 캐시하거나 표식 검사 사이로 재배치하면
/// seqlock이 무의미해진다.
fn read_bytes<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    let slice = bytes.get(at..at + N)?;
    // SAFETY: `slice`는 정확히 N바이트이고(위에서 확인), `[u8; N]`의 정렬은 1이다.
    Some(unsafe { std::ptr::read_volatile(slice.as_ptr() as *const [u8; N]) })
}

fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    read_bytes::<8>(bytes, at).map(u64::from_le_bytes)
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    read_bytes::<4>(bytes, at).map(u32::from_le_bytes)
}

fn read_f32(bytes: &[u8], at: usize) -> Option<f32> {
    read_u32(bytes, at).map(f32::from_bits)
}

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS,
};

/// 매핑을 연 것 — **Windows API는 여기에만** 있다.
///
/// 이름을 둘 다 시도한다: .NET의 `MemoryMappedFile.CreateOrOpen`은 **세션 네임스페이스**에
/// 만들므로 `Local\…`로 보이고, 어떤 경로로 만들면 이름 그대로일 수 있다. 둘 다 실패하면
/// 플러그인이 없다는 뜻이다(= RPC 폴백으로 내려간다).
#[cfg(windows)]
pub struct Ring {
    /// 뷰의 시작 주소. 우리가 만든 뷰라 드롭 전까지 유효하다.
    view: *mut core::ffi::c_void,
    len: usize,
    mapping: HANDLE,
    cursor: Cursor,
}

#[cfg(windows)]
impl Ring {
    /// 열 수 있는 매핑을 찾아 연다 — **헤더가 우리 것이 아니면 열지 않는다**.
    pub fn open() -> Option<Self> {
        [SESSION_NAME, NAME].into_iter().find_map(Self::open_named)
    }

    fn open_named(name: &str) -> Option<Self> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let mapping = OpenFileMappingW(FILE_MAP_READ.0, false, PCWSTR(wide.as_ptr())).ok()?;
            // 실패하면 `.Value`가 null이다(`MapViewOfFile`은 `Result`가 아니라 주소를 돌려준다).
            let address = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, TOTAL);
            if address.Value.is_null() {
                let _ = CloseHandle(mapping);
                return None;
            }
            let ring = Self {
                view: address.Value,
                len: TOTAL,
                mapping,
                cursor: Cursor::new(),
            };
            // 남의 매핑(낡은 플러그인)을 붙들고 있을 이유가 없다 — 드롭이 닫는다.
            header(ring.bytes())?;
            Some(ring)
        }
    }

    /// 폴링 한 번 — 헤더를 **다시 읽고**(살아 있는 값이다) 새 표본을 꺼낸다.
    ///
    /// 우리 매핑이 아니게 되면 `None`이다(앱이 다시 열거나 RPC로 내려간다).
    pub fn poll(&mut self, out: &mut Vec<PenSample>) -> Option<(Header, Readout)> {
        // 필드를 나눠 빌린다: 뷰를 읽는 동안 커서를 빌릴 수 있어야 한다.
        let Self {
            view, len, cursor, ..
        } = self;
        // SAFETY: `bytes`와 같은 근거다(뷰는 `len`바이트이고 드롭 전까지 유효하다).
        let bytes = unsafe { std::slice::from_raw_parts(*view as *const u8, *len) };
        let header = header(bytes)?;
        let readout = cursor.drain(bytes, &header, out);
        Some((header, readout))
    }

    /// 매핑 바이트 — **다른 프로세스가 계속 바꾸는** 메모리다(모든 읽기가 휘발성이다).
    fn bytes(&self) -> &[u8] {
        // SAFETY: 뷰는 `len`바이트이고 우리가 소유한다(드롭 전까지 유효하다). 내용이 언제든
        // 바뀌는 것은 이 계약의 전제다 — 읽는 쪽은 표식(seqlock)으로 반쪽을 걸러낸다.
        unsafe { std::slice::from_raw_parts(self.view as *const u8, self.len) }
    }
}

#[cfg(windows)]
impl Drop for Ring {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: self.view });
            let _ = CloseHandle(self.mapping);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 계약대로 헤더를 채운 매핑 하나 — 테스트는 **바이트만** 만든다(Windows 없이).
    fn mapping(capacity: u32) -> Vec<u8> {
        let len = HEADER_SIZE + capacity as usize * SAMPLE_SIZE as usize;
        let mut bytes = vec![0u8; len];
        put_u64(&mut bytes, OFF_MAGIC, MAGIC);
        put_u32(&mut bytes, OFF_VERSION, VERSION);
        put_u32(&mut bytes, OFF_SAMPLE_SIZE, SAMPLE_SIZE);
        put_u32(&mut bytes, OFF_CAPACITY, capacity);
        put_u32(&mut bytes, OFF_HEADER_FLAGS, 1);
        bytes[OFF_NAME..OFF_NAME + 5].copy_from_slice(b"deco\0");
        bytes
    }

    fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_f32(bytes: &mut [u8], at: usize, value: f32) {
        put_u32(bytes, at, value.to_bits());
    }

    /// 슬롯의 시작 오프셋 — 계약의 배치 규칙 그대로.
    fn slot_at(capacity: u32, seq: u64) -> usize {
        HEADER_SIZE + (seq % capacity as u64) as usize * SAMPLE_SIZE as usize
    }

    /// 플러그인처럼 한 표본을 쓴다 — **표식 순서도 계약이다**(쓰는 중 → 필드 → 완성).
    fn write_sample(bytes: &mut [u8], capacity: u32, seq: u64, flags: u32, x: f32, y: f32, p: f32) {
        let base = slot_at(capacity, seq);
        put_u64(bytes, base + OFF_SLOT_SEQ, seq * 2 + 1);
        put_f32(bytes, base + OFF_X, x);
        put_f32(bytes, base + OFF_Y, y);
        put_f32(bytes, base + OFF_PRESSURE, p);
        put_u32(bytes, base + OFF_FLAGS, flags);
        put_u64(bytes, base + OFF_SLOT_SEQ, seq * 2);
        put_u64(bytes, OFF_WRITE_SEQ, seq);
    }

    #[test]
    fn a_foreign_mapping_is_not_read() {
        // 남의 매핑·낡은 플러그인을 읽으면 좌표가 엉뚱해진다 — 세 검사가 그것을 막는다.
        let mut bytes = mapping(CAPACITY);
        assert!(header(&bytes).is_some(), "계약대로면 읽힌다");

        let mut wrong_magic = bytes.clone();
        put_u64(&mut wrong_magic, OFF_MAGIC, 0);
        assert_eq!(header(&wrong_magic), None, "magic이 다르다");

        let mut wrong_version = bytes.clone();
        put_u32(&mut wrong_version, OFF_VERSION, VERSION + 1);
        assert_eq!(header(&wrong_version), None, "version이 다르다");

        let mut small_slot = bytes.clone();
        put_u32(&mut small_slot, OFF_SAMPLE_SIZE, 32);
        assert_eq!(header(&small_slot), None, "슬롯이 계약보다 작다");

        // 플러그인이 더 큰 링을 썼다면 우리 뷰에 안 들어온다.
        put_u32(&mut bytes, OFF_CAPACITY, CAPACITY * 2);
        assert_eq!(header(&bytes), None, "우리 뷰보다 큰 링이다");
    }

    #[test]
    fn the_header_carries_what_the_app_needs() {
        let mut bytes = mapping(CAPACITY);
        put_u64(&mut bytes, OFF_WRITE_SEQ, 954);
        put_f32(&mut bytes, OFF_MAX_X, 51196.0);
        put_f32(&mut bytes, OFF_MAX_Y, 31826.0);
        put_f32(&mut bytes, OFF_MAX_PRESSURE, 16383.0);
        put_u64(&mut bytes, OFF_HEARTBEAT, 46_081_143_799);

        let spec = header(&bytes).expect("헤더");
        assert_eq!(spec.write_seq, 954);
        assert_eq!(spec.tablet_name, "deco");
        assert!(spec.is_alive());
        assert!(spec.has_range());
        assert_eq!(spec.heartbeat, 46_081_143_799);

        // 범위가 0이면 플러그인이 모르는 것이다(앱이 RPC로 채운다) — "0 범위"로 나누면 안 된다.
        put_f32(&mut bytes, OFF_MAX_X, 0.0);
        assert!(!header(&bytes).expect("헤더").has_range());
    }

    #[test]
    fn a_sample_is_decoded_with_the_contract_offsets() {
        // 계약의 오프셋이 곧 이 테스트다 — 한 칸이라도 밀리면 값이 뒤바뀐다.
        let mut bytes = mapping(CAPACITY);
        write_sample(&mut bytes, CAPACITY, 1, FLAG_TIP, 1.0, 2.0, 3.0);
        let spec = header(&bytes).expect("헤더");
        let mut cursor = Cursor::new();
        let mut out = Vec::new();
        assert_eq!(cursor.drain(&bytes, &spec, &mut out).read, 0, "붙기");

        // 접촉 + 지우개 + 두 번째 버튼 + 틸트를 한 표본에 담아 본다.
        let base = slot_at(CAPACITY, 2);
        write_sample(&mut bytes, CAPACITY, 2, FLAG_TIP, 28388.0, 10286.0, 8192.0);
        put_u32(
            &mut bytes,
            base + OFF_FLAGS,
            FLAG_TIP | FLAG_ERASER | (1 << (BUTTON_SHIFT + 1)),
        );
        put_f32(&mut bytes, base + OFF_TILT_X, 12.5);
        put_f32(&mut bytes, base + OFF_TILT_Y, -30.0);

        let mut events = Vec::new();
        let readout = cursor.drain(&bytes, &header(&bytes).expect("헤더"), &mut events);
        assert_eq!((readout.read, readout.skipped), (1, 0));
        let sample = events[0];
        assert_eq!(sample.seq, 2);
        assert_eq!((sample.x, sample.y), (28388.0, 10286.0));
        assert_eq!(sample.pressure, 8192.0);
        assert!(sample.contact);
        assert!(sample.eraser);
        assert_eq!(sample.buttons, 0b10, "두 번째 버튼이 비트 1이다");
        assert_eq!(sample.tilt, Some((12.5, -30.0)));

        // 틸트를 보고하지 않은 표본은 `(0, 0)`이고, 계약대로 `None`이 된다.
        write_sample(&mut bytes, CAPACITY, 3, 0, 5.0, 5.0, 0.0);
        let mut hover = Vec::new();
        cursor.drain(&bytes, &header(&bytes).expect("헤더"), &mut hover);
        assert_eq!(hover[0].tilt, None, "(0, 0)은 '안 보고함'이다");
        assert!(!hover[0].contact, "호버는 접촉이 아니다");
    }

    #[test]
    fn a_slot_being_written_is_read_on_the_next_poll() {
        // 플러그인이 쓰는 **도중**에 폴링하면 표식이 `2*seq+1`이다 — 그때 커서를 넘겨 버리면
        // 표본 하나를 잃는다. 다음 폴링에서 완성된 것을 읽어야 한다.
        let mut bytes = mapping(CAPACITY);
        write_sample(&mut bytes, CAPACITY, 1, FLAG_TIP, 10.0, 20.0, 30.0);
        let spec = header(&bytes).expect("헤더");
        let mut cursor = Cursor::new();
        let mut out = Vec::new();
        cursor.drain(&bytes, &spec, &mut out); // 붙기

        // 2번을 "쓰는 중"으로 만든다(필드는 아직 안 썼다).
        let base = slot_at(CAPACITY, 2);
        put_u64(&mut bytes, OFF_WRITE_SEQ, 2);
        put_u64(&mut bytes, base + OFF_SLOT_SEQ, 2 * 2 + 1);

        let mut half = Vec::new();
        let readout = cursor.drain(&bytes, &header(&bytes).expect("헤더"), &mut half);
        assert_eq!(readout.read, 0, "반쪽은 읽지 않는다");
        assert!(half.is_empty());

        // 플러그인이 완성했다 — 이번에는 읽힌다(커서가 2번에 멈춰 있었으므로).
        put_f32(&mut bytes, base + OFF_X, 11.0);
        put_u32(&mut bytes, base + OFF_FLAGS, FLAG_TIP);
        put_u64(&mut bytes, base + OFF_SLOT_SEQ, 2 * 2);

        let mut done = Vec::new();
        assert_eq!(
            cursor
                .drain(&bytes, &header(&bytes).expect("헤더"), &mut done)
                .read,
            1
        );
        assert_eq!(done[0].seq, 2);
        assert_eq!(done[0].x, 11.0);
    }

    #[test]
    fn a_ring_that_lapped_counts_what_it_lost() {
        // 앱이 오래 멈춰 링이 한 바퀴를 돌면 옛 표본은 덮여 썼다 — **버린 것을 세어서**
        // 화면에 보여준다(옛 좌표를 따라 그리면 더 나쁘다).
        let capacity = 4;
        let mut bytes = mapping(capacity);
        let mut cursor = Cursor::new();
        let mut out = Vec::new();
        write_sample(&mut bytes, capacity, 1, FLAG_TIP, 0.0, 0.0, 0.0);
        // 붙기 — 헤더를 **다시 읽는다**(살아 있는 값이다): 이래야 커서가 1번에 선다.
        cursor.drain(&bytes, &header(&bytes).expect("헤더"), &mut out);

        // 10번까지 갔다 — 남아 있는 것은 7, 8, 9, 10이고 2..=6은 덮여 썼다.
        for seq in 2..=10 {
            write_sample(&mut bytes, capacity, seq, FLAG_TIP, seq as f32, 0.0, 0.0);
        }
        let mut kept = Vec::new();
        let readout = cursor.drain(&bytes, &header(&bytes).expect("헤더"), &mut kept);
        assert_eq!(readout.latest, 10);
        assert_eq!(readout.skipped, 5, "2..=6을 잃었다");
        assert_eq!(
            kept.iter().map(|sample| sample.seq).collect::<Vec<_>>(),
            vec![7, 8, 9, 10]
        );
    }

    #[test]
    fn out_of_range_ends_the_stroke() {
        // 범위 이탈은 좌표가 무효다(플러그인이 0을 쓴다) — 종류만 말한다.
        let mut bytes = mapping(CAPACITY);
        write_sample(&mut bytes, CAPACITY, 1, FLAG_OUT_OF_RANGE, 0.0, 0.0, 0.0);
        let spec = header(&bytes).expect("헤더");
        let mut cursor = Cursor::new();
        let mut out = Vec::new();
        cursor.drain(&bytes, &spec, &mut out); // 붙기(write_seq = 1)

        write_sample(&mut bytes, CAPACITY, 2, FLAG_OUT_OF_RANGE, 0.0, 0.0, 0.0);
        let mut events = Vec::new();
        cursor.drain(&bytes, &header(&bytes).expect("헤더"), &mut events);
        assert_eq!(events.len(), 1);
        assert!(events[0].ends_the_stroke());
        assert!(!events[0].contact, "이탈은 접촉이 아니다");
    }
}

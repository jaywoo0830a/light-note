//! OTD RPC 클라이언트 — **한 번 접속해 한 번 묻는다**.
//!
//! ## 왜 스트리밍이 아니라 한 번 묻기인가
//! 표본(필기)은 공유 메모리로 온다 — RPC의 `DeviceReport`는 리포트마다 태블릿 정보 약 1 KB를
//! 직렬화하므로 1000배 비싸다(`PROTOCOL.md`의 비교표). RPC가 필요한 자리는 **딱 하나**다:
//! 태블릿 범위(`GetTablets`). 필터는 `IOutputMode.Tablet`에 접근할 수 없어서 공유 메모리
//! 헤더의 `max_*`가 0으로 남기 때문이다(그러면 좌표를 페이지로 옮길 수 없다).
//!
//! ## 프레이밍은 `frame.rs`가 한다
//! OTD 0.6.7은 `StreamJsonRpc`로 말한다: `Content-Length: N\r\n\r\n{json}`. 응답을 기다릴 때
//! **알림이 섞여 올 수 있다**(다른 클라이언트가 디버그 스트림을 켰거나 로그 알림이 오거나) —
//! 그래서 우리 `id`의 응답만 채택하고 나머지는 버린다.
//!
//! ## 왜 여기서 타임아웃을 만들지 않는가
//! 파이프를 비동기(overlapped)로 열고 기다리는 장치는 이 한 번의 호출에 비해 너무 크다.
//! 대신 **호출하는 쪽이 전용 스레드**다([`crate::otd::session`]) — 리액터의 백그라운드 풀을
//! 붙들지 않으므로, 데몬이 대답하지 않아도 UI와 베이크는 멀쩡하다.

use crate::otd::frame::Framer;
use crate::otd::wire::{self, TabletSpec};

/// 요청 하나의 `id` — 우리는 한 번에 하나만 묻는다.
const ID: u32 = 1;
/// 응답을 기다리며 버릴 수 있는 알림의 상한 — 넘으면 프레이밍이 어긋난 것이다.
const MAX_MESSAGES: usize = 64;

/// OTD 데몬의 파이프 이름(실측).
pub const PIPE: &str = r"\\.\pipe\OpenTabletDriver.Daemon";

/// 태블릿 범위를 OTD에 묻는다 — 실패하면 **이유를 사람이 읽을 수 있게** 돌려준다.
pub fn tablet_spec() -> Result<TabletSpec, String> {
    let body = call("GetTablets", "[]")?;
    wire::tablet(&body).ok_or_else(|| {
        "OTD reported no tablet (is a tablet plugged in and is the daemon running?)".to_string()
    })
}

/// 요청 하나를 보내고 **그 `id`의 응답 본문**을 받는다.
pub fn call(method: &str, params: &str) -> Result<Vec<u8>, String> {
    let handle = platform::open()?;
    let pipe = platform::Pipe(handle);
    let request = format!(r#"{{"jsonrpc":"2.0","id":{ID},"method":"{method}","params":{params}}}"#);
    platform::write(pipe.0, &frame(request.as_bytes()))?;

    let mut framer = Framer::new();
    let mut buffer = vec![0u8; 8 * 1024];
    for _ in 0..MAX_MESSAGES {
        let read = platform::read(pipe.0, &mut buffer)?;
        if read == 0 {
            return Err("OTD closed the pipe before answering".to_string());
        }
        framer.push(&buffer[..read]);
        while let Some(body) = framer.next_body() {
            if is_our_response(body) {
                return Ok(body.to_vec());
            }
        }
    }
    Err("OTD kept sending other messages — the framing is off".to_string())
}

/// `{"jsonrpc":"2.0","id":1,"result":…}` — **우리 id의 응답**인가(알림과 구분한다).
fn is_our_response(body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    // `result`나 `error`가 있고 `id`가 우리 것일 때만 응답이다(오류도 받아서 이유를 보여준다).
    (value.get("result").is_some() || value.get("error").is_some())
        && value.get("id").and_then(serde_json::Value::as_u64) == Some(ID as u64)
}

/// 스트림이 요구하는 프레임 하나 — `Content-Length: N\r\n\r\n{json}`.
fn frame(body: &[u8]) -> Vec<u8> {
    let mut bytes = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

#[cfg(windows)]
mod platform {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };

    /// 열린 파이프 — 드롭에서 닫는다.
    pub struct Pipe(pub HANDLE);

    impl Drop for Pipe {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// 데몬에 접속한다 — 데몬이 없으면 여기서 실패한다(그것도 정상 경로의 하나다).
    pub fn open() -> Result<HANDLE, String> {
        let name: Vec<u16> = super::PIPE
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
            .map_err(|error| format!("OTD daemon is not reachable ({error})"))
        }
    }

    pub fn write(handle: HANDLE, bytes: &[u8]) -> Result<(), String> {
        let mut written = 0u32;
        unsafe { WriteFile(handle, Some(bytes), Some(&mut written), None) }
            .map_err(|error| format!("OTD write failed ({error})"))?;
        if written as usize != bytes.len() {
            return Err("OTD took only part of the request".to_string());
        }
        Ok(())
    }

    /// 한 번 읽는다 — **막힐 수 있다**(데몬이 대답하지 않으면 여기서 멈춘다: 호출자가 전용 스레드다).
    pub fn read(handle: HANDLE, buffer: &mut [u8]) -> Result<usize, String> {
        let mut read = 0u32;
        unsafe { ReadFile(handle, Some(buffer), Some(&mut read), None) }
            .map_err(|error| format!("OTD read failed ({error})"))?;
        Ok(read as usize)
    }
}

#[cfg(not(windows))]
mod platform {
    use windows::Win32::Foundation::HANDLE;

    pub struct Pipe(pub HANDLE);

    pub fn open() -> Result<HANDLE, String> {
        Err("OTD is Windows only".to_string())
    }

    pub fn write(_handle: HANDLE, _bytes: &[u8]) -> Result<(), String> {
        Err("OTD is Windows only".to_string())
    }

    pub fn read(_handle: HANDLE, _buffer: &mut [u8]) -> Result<usize, String> {
        Err("OTD is Windows only".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_is_content_length_then_the_body() {
        // 계약은 `frame.rs`와 같다 — 두 곳이 다른 프레임을 쓰면 조용히 깨진다.
        assert_eq!(frame(b"{}"), b"Content-Length: 2\r\n\r\n{}");
    }

    #[test]
    fn only_our_id_is_a_response() {
        assert!(is_our_response(br#"{"jsonrpc":"2.0","id":1,"result":[]}"#));
        // 다른 id(다른 클라이언트의 응답)와 알림은 우리 것이 아니다.
        assert!(!is_our_response(br#"{"jsonrpc":"2.0","id":2,"result":[]}"#));
        assert!(!is_our_response(
            br#"{"jsonrpc":"2.0","method":"Message","params":[]}"#
        ));
        // 오류 응답도 받아서 이유를 보여준다.
        assert!(is_our_response(
            br#"{"jsonrpc":"2.0","id":1,"error":{"code":-1}}"#
        ));
        // JSON이 아니면 조용히 `false`다(깨진 스트림이 앱을 멈추게 두지 않는다).
        assert!(!is_our_response(b"Content-Length: 2"));
    }

    #[test]
    fn a_tablet_list_becomes_a_spec() {
        // 실측 페이로드로 파싱까지 확인한다(파이프 없이).
        let body = include_str!("../../tests/fixtures/otd/get_tablets.json");
        let spec = wire::tablet(body.as_bytes()).expect("태블릿");
        assert_eq!(spec.max_x, 51196.0);
        assert_eq!(spec.max_pressure, 16383.0);
    }
}

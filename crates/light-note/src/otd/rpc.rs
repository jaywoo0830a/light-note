//! The OTD daemon's pipe: **one question, asked once**.
//!
//! ## Why the app talks to OTD at all
//! Pen samples arrive through shared memory.  The one thing shared memory cannot
//! say is the tablet's range, because a `PreTransform` filter never sees
//! `IOutputMode.Tablet` (`PROTOCOL.md`) — and without the range a sample is just a
//! pair of device units with nowhere to go.  So this is the app's only RPC.
//!
//! ## Why the call runs on its own thread
//! `ReadFile` on a pipe **blocks** until the daemon answers, and a daemon that is
//! busy (or gone) may never answer.  The caller is therefore a dedicated thread
//! ([`super::reader`]), never the reader loop — a stuck pipe there would stop the
//! pen itself, which is exactly the kind of dependency this app is built to avoid.

use crate::otd::spec::TabletSpec;
use crate::otd::tablets::{self, Framer, ID};

/// The daemon's pipe name (measured on OTD 0.6.7).
pub const PIPE: &str = r"\\.\pipe\OpenTabletDriver.Daemon";

/// How many messages to skip before deciding the framing is off.  The stream can
/// carry notifications before our answer; it cannot carry this many.
const MAX_MESSAGES: usize = 64;

/// Asks the daemon for the tablet's range, or explains why it could not be read.
pub fn tablet_spec() -> Result<TabletSpec, String> {
    let body = call("GetTablets", "[]")?;
    tablets::parse_tablet(&body)
        .ok_or_else(|| "OTD reports no tablet (is one plugged in and configured?)".to_string())
}

/// Sends one request and returns the body of *our* answer.
pub fn call(method: &str, params: &str) -> Result<Vec<u8>, String> {
    let pipe = Pipe::open()?;
    pipe.write(&tablets::frame_request(ID, method, params))?;

    let mut framer = Framer::new();
    let mut buffer = vec![0u8; 8 * 1024];
    for _ in 0..MAX_MESSAGES {
        let read = pipe.read(&mut buffer)?;
        if read == 0 {
            return Err("OTD closed the pipe before answering".to_string());
        }
        framer.push(&buffer[..read]);
        while let Some(body) = framer.next_body() {
            if tablets::is_response_for(ID, body) {
                return Ok(body.to_vec());
            }
        }
    }
    Err("OTD kept sending other messages — the framing is off".to_string())
}

/// An open pipe.  Dropping it closes the handle.
struct Pipe(windows::Win32::Foundation::HANDLE);

impl Pipe {
    /// Connects to the daemon; a daemon that is not running is a normal outcome.
    fn open() -> Result<Self, String> {
        use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows::core::PCWSTR;

        let name: Vec<u16> = PIPE.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        };
        handle
            .map(Self)
            .map_err(|error| format!("the OTD daemon is not reachable ({error})"))
    }

    fn write(&self, bytes: &[u8]) -> Result<(), String> {
        use windows::Win32::Storage::FileSystem::WriteFile;

        let mut written = 0u32;
        unsafe { WriteFile(self.0, Some(bytes), Some(&mut written), None) }
            .map_err(|error| format!("writing to OTD failed ({error})"))?;
        if written as usize != bytes.len() {
            return Err("OTD took only part of the request".to_string());
        }
        Ok(())
    }

    /// One read.  **This can block**: the caller is a dedicated thread.
    fn read(&self, buffer: &mut [u8]) -> Result<usize, String> {
        use windows::Win32::Storage::FileSystem::ReadFile;

        let mut read = 0u32;
        unsafe { ReadFile(self.0, Some(buffer), Some(&mut read), None) }
            .map_err(|error| format!("reading from OTD failed ({error})"))?;
        Ok(read as usize)
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

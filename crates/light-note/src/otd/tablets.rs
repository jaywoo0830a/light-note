//! The daemon's JSON-RPC wire — as much of it as the app needs: **one question**.
//!
//! `OTD.SharedMemoryOutput/PROTOCOL.md` states why: a `PreTransform` filter cannot
//! reach `IOutputMode.Tablet`, so the shared-memory header's `max_*` are 0 and the
//! app asks the daemon for the range instead (`GetTablets`).  That call is the only
//! RPC in the app — pen samples come through shared memory, which costs about a
//! thousand times less than the RPC's per-report JSON (the table in `PROTOCOL.md`).
//!
//! Everything here is pure — bytes in, values out — so the protocol is tested
//! without a daemon: `tests/otd_tablets.rs` runs against a `GetTablets` answer
//! captured from OTD 0.6.7 on this machine.  The pipe is [`super::rpc`].
//!
//! ## The two things that must not be got wrong
//!
//! 1. **Framing.** OTD 0.6.7 hosts `StreamJsonRpc`, whose default framing is
//!    `Content-Length: N\r\n\r\n{json}`.  A different frame is not an error the
//!    daemon reports — it simply never answers.
//! 2. **Whose answer is it.** The same stream carries notifications (`Message`,
//!    `DeviceReport`), so an answer is only ours when it has `result`/`error`
//!    *and* our id.

use serde::Deserialize;

use super::map::TabletSpec;

/// The id the app's one request uses, so its answer is recognisable.
pub const ID: u32 = 1;

/// One request, framed the way `StreamJsonRpc` expects.
pub fn frame_request(id: u32, method: &str, params: &str) -> Vec<u8> {
    let body = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#);
    let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    framed.extend_from_slice(body.as_bytes());
    framed
}

/// Is `body` the answer to *our* request?  (Notifications and other clients'
/// answers are not, and neither is anything that is not JSON.)
pub fn is_response_for(id: u32, body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    (value.get("result").is_some() || value.get("error").is_some())
        && value.get("id").and_then(serde_json::Value::as_u64) == Some(id as u64)
}

/// The tablet's range in a `GetTablets` answer, or `None` when there is none
/// (no tablet plugged in, an error answer, or a stream that is not JSON).
///
/// The names come from .NET's default serialization, so they are `PascalCase`
/// (`Properties`, `Digitizer`, `MaxX`) — the structs say that, instead of every
/// name being spelled out here.
pub fn parse_tablet(body: &[u8]) -> Option<TabletSpec> {
    let answer: Answer = serde_json::from_slice(body).ok()?;
    let tablet = answer.result.into_iter().next()?;
    let properties = tablet.properties;
    let digitizer = properties.specifications.digitizer;
    let pen = properties.specifications.pen;

    // A tablet that reports no pressure is "always full pressure": a maximum of
    // 0 would divide by zero on every sample (and the ink would never appear).
    let max_pressure = if pen.max_pressure > 0.0 {
        pen.max_pressure
    } else {
        1.0
    };
    Some(TabletSpec::new(
        properties.name,
        digitizer.max_x,
        digitizer.max_y,
        max_pressure,
    ))
}

/// The parts of an answer the app reads.  Unknown fields are ignored (serde's
/// default), so a newer daemon that adds fields does not break this.
#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    result: Vec<Tablet>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Tablet {
    #[serde(default)]
    properties: Properties,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct Properties {
    #[serde(default)]
    name: String,
    #[serde(default)]
    specifications: Specifications,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct Specifications {
    #[serde(default)]
    digitizer: Digitizer,
    #[serde(default)]
    pen: Pen,
}

/// The active area in device units — what a sample's `x`/`y` are measured in.
#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct Digitizer {
    #[serde(default)]
    max_x: f32,
    #[serde(default)]
    max_y: f32,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct Pen {
    #[serde(default)]
    max_pressure: f32,
}

/// Cuts a stream of bytes into messages.
///
/// A pipe read does not respect message boundaries: one read can carry two
/// messages, or half of one.  So the bytes are accumulated and only **complete**
/// messages are handed out — consuming half a message would misalign everything
/// after it.
#[derive(Debug, Default)]
pub struct Framer {
    buffer: Vec<u8>,
    /// The first byte that has not been handed out yet.
    start: usize,
}

impl Framer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds freshly read bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        self.compact();
        self.buffer.extend_from_slice(bytes);
    }

    /// The next complete message body, or `None` when more bytes are needed.
    pub fn next_body(&mut self) -> Option<&[u8]> {
        // Destructured so the returned slice borrows the buffer rather than all
        // of `self` (the cursor has to move at the same time).
        let Self { buffer, start } = self;
        let pending = &buffer[*start..];
        let header_end = find(pending, b"\r\n\r\n")?;
        let length = content_length(&pending[..header_end])?;
        let body_start = header_end + 4;
        let body_end = body_start.checked_add(length)?;
        if pending.len() < body_end {
            return None;
        }
        let body = &pending[body_start..body_end];
        *start += body_end;
        Some(body)
    }

    /// Bytes read but not handed out yet — if this only grows, the framing is off.
    pub fn pending(&self) -> usize {
        self.buffer.len() - self.start
    }

    /// Drops what has been handed out.  The cursor moves instead of the buffer
    /// being drained per message, and the buffer is shifted only once most of it
    /// is consumed (a memmove per message would be the expensive part).
    fn compact(&mut self) {
        if self.start == 0 {
            return;
        }
        if self.start == self.buffer.len() {
            self.buffer.clear();
            self.start = 0;
            return;
        }
        if self.start * 2 >= self.buffer.len() {
            self.buffer.copy_within(self.start.., 0);
            let rest = self.buffer.len() - self.start;
            self.buffer.truncate(rest);
            self.start = 0;
        }
    }
}

/// `Content-Length: N` — the header name is case-insensitive, like HTTP.
fn content_length(header: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(header).ok()?;
    for line in text.split("\r\n") {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value.trim().parse().ok();
        }
    }
    None
}

/// The first position of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

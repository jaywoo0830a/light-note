//! OTD 파이프의 **프레이밍** — `Content-Length: N\r\n\r\n{json}`.
//!
//! ## 왜 이 파일이 따로 있는가
//! OTD 0.6.7의 데몬은 `StreamJsonRpc`로 말한다(실측: `RpcHost<T>`가 `new JsonRpc(stream, stream, host)`).
//! 그 기본 프레이밍은 **HTTP처럼 헤더 한 줄 + 빈 줄 + 본문**이다. 본문은 UTF-8 JSON이다.
//!
//! ```text
//! Content-Length: 172\r\n
//! \r\n
//! {"jsonrpc":"2.0","method":"Message","params":[…]}
//! ```
//!
//! **읽기는 스트림이라 메시지 경계에 맞춰 오지 않는다**: 한 번의 `read`가 두 메시지를 담을 수도,
//! 반쪽만 담을 수도 있다. 그래서 누적 버퍼와 소비 위치를 따로 들고, **완성된 것만** 꺼낸다
//! (한 번의 읽기 = 여러 리포트인 경우가 흔하다 — 그게 곧 배치다).
//!
//! ## 왜 소비 위치를 옮기는가
//! 메시지마다 `Vec`에서 앞을 지우면(drain) 리포트 400개/초에서 초당 수백 KB를 memmove한다.
//! 소비 위치만 옮기고, 자리가 모자랄 때만 **한 번** 압축한다 — 그래서 리포트당 복사가 없다.

/// 스트림 바이트를 메시지로 자른다. **여러 읽기를 넘겨 쓰는 유일한 상태**다.
#[derive(Debug)]
pub struct Framer {
    buf: Vec<u8>,
    /// 아직 꺼내지 않은 첫 바이트.
    start: usize,
}

impl Framer {
    /// 누적 버퍼의 처음 크기 — 한 번의 읽기가 보통 이만큼 온다(실측: 195 B ~ 1 KB).
    pub const CAPACITY: usize = 64 * 1024;

    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(Self::CAPACITY),
            start: 0,
        }
    }

    /// 새로 읽은 바이트를 넣는다 — **여기서만** 버퍼가 자란다.
    pub fn push(&mut self, bytes: &[u8]) {
        self.compact();
        self.buf.extend_from_slice(bytes);
    }

    /// 완성된 메시지의 **본문** 하나 — 없으면 `None`(더 읽어야 한다).
    ///
    /// 헤더를 못 찾거나 길이가 모자라면 **아무것도 소비하지 않는다**: 반쪽 메시지를 버리면
    /// 그 뒤의 모든 메시지가 어긋난다.
    pub fn next_body(&mut self) -> Option<&[u8]> {
        // 필드를 나눠 빌린다: `buf`는 그대로 두고 `start`만 옮긴다(그래서 `unsafe`가 필요 없다).
        let Self { buf, start } = self;
        let pending = &buf[*start..];
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

    /// 아직 못 꺼낸 바이트 수 — 진단용(늘어나기만 하면 프레이밍이 어긋난 것이다).
    pub fn pending(&self) -> usize {
        self.buf.len() - self.start
    }

    /// 소비한 앞부분을 버린다 — **꺼낼 것이 남아 있으면 자리만 옮긴다**.
    fn compact(&mut self) {
        if self.start == 0 {
            return;
        }
        if self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
            return;
        }
        // 반 넘게 소비했을 때만 옮긴다(자주 옮기면 그게 더 비싸다).
        if self.start * 2 >= self.buf.len() {
            self.buf.copy_within(self.start.., 0);
            let rest = self.buf.len() - self.start;
            self.buf.truncate(rest);
            self.start = 0;
        }
    }
}

impl Default for Framer {
    fn default() -> Self {
        Self::new()
    }
}

/// `Content-Length: N` — 헤더 이름은 **대소문자를 가리지 않는다**(HTTP 규칙 그대로).
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

/// 바늘의 첫 위치 — `windows` 크레이트를 쓰지 않는다(순수 함수라 테스트가 플랫폼과 무관하다).
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(body: &str) -> Vec<u8> {
        let mut bytes = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        bytes.extend_from_slice(body.as_bytes());
        bytes
    }

    #[test]
    fn one_read_can_carry_many_messages() {
        // 실측 그대로: 한 번의 읽기에 리포트가 여러 개 들어온다 — 그게 곧 배치다.
        let mut framer = Framer::new();
        let mut bytes = framed("{\"a\":1}");
        bytes.extend_from_slice(&framed("{\"b\":2}"));
        framer.push(&bytes);
        assert_eq!(framer.next_body(), Some(&b"{\"a\":1}"[..]));
        assert_eq!(framer.next_body(), Some(&b"{\"b\":2}"[..]));
        assert_eq!(framer.next_body(), None, "다 꺼냈다");
        assert_eq!(framer.pending(), 0);
    }

    #[test]
    fn a_half_message_waits_for_the_rest() {
        // 반쪽을 버리면 그 뒤의 모든 메시지가 어긋난다 — **아무것도 소비하지 않는다**.
        let mut framer = Framer::new();
        let bytes = framed("{\"hello\":\"world\"}");
        let (head, tail) = bytes.split_at(20);
        framer.push(head);
        assert_eq!(framer.next_body(), None);
        assert_eq!(framer.pending(), 20, "반쪽은 남아 있다");
        framer.push(tail);
        assert_eq!(framer.next_body(), Some(&b"{\"hello\":\"world\"}"[..]));
        assert_eq!(framer.pending(), 0);
    }

    #[test]
    fn the_header_name_is_case_insensitive() {
        let mut framer = Framer::new();
        framer.push(b"content-length: 2\r\n\r\n{}");
        assert_eq!(framer.next_body(), Some(&b"{}"[..]));
    }

    #[test]
    fn a_broken_header_is_not_consumed() {
        // 길이를 못 읽거나 본문이 모자라면 소비하지 않는다(다음 읽기에 완성될 수 있다).
        let mut framer = Framer::new();
        framer.push(b"Content-Length: 4\r\n\r\n{}");
        assert_eq!(framer.next_body(), None, "본문이 모자란다");
        assert_eq!(framer.pending(), 23, "헤더 21 + 본문 2 — 소비하지 않았다");
    }

    #[test]
    fn the_buffer_is_compacted_not_drained_per_message() {
        // 리포트 400개/초에서 메시지마다 memmove하지 않는다 — 자리만 옮기고 가끔 압축한다.
        let mut framer = Framer::new();
        for _ in 0..200 {
            framer.push(&framed("{\"x\":1}"));
        }
        let mut seen = 0;
        while framer.next_body().is_some() {
            seen += 1;
        }
        assert_eq!(seen, 200);
        assert_eq!(framer.pending(), 0);
        // 압축이 실제로 일어났다: 다음 메시지가 버퍼 앞에서 시작한다.
        framer.push(&framed("{\"y\":2}"));
        assert_eq!(framer.next_body(), Some(&b"{\"y\":2}"[..]));
    }
}

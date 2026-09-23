//! The app's half of the OTD contract: the tablet range the plugin cannot know.
//!
//! `OTD.SharedMemoryOutput/PROTOCOL.md` states the rule this file exists for: a
//! `PreTransform` filter cannot reach `IOutputMode.Tablet`, so the header's
//! `max_*` are **0** and *the app fills them in from the daemon's `GetTablets`
//! RPC*.  Without that range a pen sample is just a pair of device units — there
//! is no way to turn it into a point on the page, which is exactly what "the
//! tablet is not detected" looks like from the outside.
//!
//! This is the **only** reason the app speaks to OTD's pipe: samples arrive
//! through shared memory, which costs ~1000x less than the RPC's per-report JSON
//! (the comparison table is in `PROTOCOL.md`).  Everything here is pure — framing,
//! parsing and the header fill — so the whole file runs with no daemon, no pipe
//! and no waiting.  The pipe itself is Windows glue in `otd::rpc`.

mod support;

use light_note::otd::tablets::{self, Framer};
use light_note::otd::{MAGIC, TabletSpec, decode_header, fill_tablet};

/// A real `GetTablets` answer, captured from OTD 0.6.7 on this machine.
const TABLETS: &str = include_str!("fixtures/otd/get_tablets.json");

fn captured() -> TabletSpec {
    tablets::parse_tablet(TABLETS.as_bytes()).expect("the captured answer has a tablet")
}

#[test]
fn a_request_is_framed_with_its_length() {
    // StreamJsonRpc (what OTD 0.6.7 hosts) frames like HTTP: header, blank line,
    // body.  A different frame is a silent failure — the daemon just never answers.
    let request = tablets::frame_request(1, "GetTablets", "[]");
    let text = String::from_utf8(request).expect("utf-8");
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"GetTablets","params":[]}"#;

    assert_eq!(text, format!("Content-Length: {}\r\n\r\n{body}", body.len()));
}

#[test]
fn only_the_answer_to_our_id_is_an_answer() {
    // Notifications (`Message`, `DeviceReport`) share the stream: taking one of
    // them for our answer would parse the wrong JSON.
    assert!(tablets::is_response_for(
        1,
        br#"{"jsonrpc":"2.0","id":1,"result":[]}"#
    ));
    assert!(!tablets::is_response_for(
        1,
        br#"{"jsonrpc":"2.0","id":2,"result":[]}"#
    ));
    assert!(!tablets::is_response_for(
        1,
        br#"{"jsonrpc":"2.0","method":"Message","params":[]}"#
    ));
    // An error is an answer too — the reason is worth showing.
    assert!(tablets::is_response_for(
        1,
        br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601}}"#
    ));
    // Garbage is not: a broken stream must not stop the app.
    assert!(!tablets::is_response_for(1, b"Content-Length: 2"));
}

#[test]
fn a_half_message_waits_for_the_rest() {
    // A pipe read does not respect message boundaries.  Consuming half a message
    // would misalign every message after it.
    let mut framer = Framer::new();
    let bytes = tablets::frame_request(1, "GetTablets", "[]");
    let (head, tail) = bytes.split_at(20);

    framer.push(head);
    assert_eq!(framer.next_body(), None, "the body is not all here yet");
    assert_eq!(framer.pending(), 20, "nothing was consumed");

    framer.push(tail);
    let body = framer.next_body().expect("the whole request came back");
    assert!(String::from_utf8_lossy(body).contains("GetTablets"));
    assert_eq!(framer.pending(), 0);
}

#[test]
fn one_read_can_carry_two_messages() {
    // Notifications and the answer can arrive together (measured: the daemon
    // sends log notifications while it answers).
    let mut framer = Framer::new();
    let first = tablets::frame_request(1, "Message", "[]");
    let second = tablets::frame_request(1, "GetTablets", "[]");
    framer.push(&[first, second].concat());

    let one = framer.next_body().expect("first");
    assert!(String::from_utf8_lossy(one).contains("Message"));
    let two = framer.next_body().expect("second");
    assert!(String::from_utf8_lossy(two).contains("GetTablets"));
    assert_eq!(framer.next_body(), None);
}

#[test]
fn the_tablet_range_comes_from_get_tablets() {
    let spec = captured();

    assert_eq!(spec.name, "XP-Pen Deco 01 V3 (Variant 2)");
    assert_eq!(spec.max_x, 51196.0);
    assert_eq!(spec.max_y, 31826.0);
    assert_eq!(spec.max_pressure, 16383.0);
    assert!(spec.is_known(), "a range the page can be mapped into");
}

#[test]
fn a_daemon_without_a_tablet_reports_no_range() {
    // No tablet plugged in: an empty list is not a range, and inventing one
    // would put the ink in the wrong place.
    assert_eq!(
        tablets::parse_tablet(br#"{"jsonrpc":"2.0","id":1,"result":[]}"#),
        None
    );
    assert_eq!(tablets::parse_tablet(b"not json at all"), None);
    // An error answer has no `result` either.
    assert_eq!(
        tablets::parse_tablet(br#"{"jsonrpc":"2.0","id":1,"error":{"code":-1}}"#),
        None
    );
}

#[test]
fn a_tablet_that_reports_no_pressure_does_not_divide_by_zero() {
    // Some tablets report position only.  A maximum pressure of 0 would make the
    // range unusable (or divide by zero), so it is reported as "one unit".
    let spec = tablets::parse_tablet(
        TABLETS
            .replace("\"MaxPressure\":16383", "\"MaxPressure\":0")
            .as_bytes(),
    )
    .expect("tablet");

    assert_eq!(spec.max_pressure, 1.0);
    assert_eq!(spec.max_x, 51196.0, "the rest of the range is untouched");
}

#[test]
fn the_header_gains_the_range_the_plugin_cannot_know() {
    // The plugin leaves `max_*` at zero on purpose; filling them in is the app's
    // job, and the result must be a header any reader can use.
    let mut shm = support::Shm::live(0.0, 0.0, 0.0);

    assert!(fill_tablet(&mut shm.bytes, &captured()), "the header has room");

    let header = decode_header(&shm.bytes).expect("still readable");
    assert_eq!(header.max_x, 51196.0);
    assert_eq!(header.max_y, 31826.0);
    assert_eq!(header.max_pressure, 16383.0);
    assert_eq!(header.tablet_name, "XP-Pen Deco 01 V3 (Variant 2)");
    // The fill must not disturb what the plugin owns: a reader that trusted a
    // rewritten magic or cursor would drop the whole stream.
    assert_eq!(header.magic, MAGIC);
    assert_eq!(header.write_seq, 0);
    assert!(header.is_readable());
}

#[test]
fn a_buffer_too_small_for_a_header_is_refused() {
    // The mapping is 262272 B in practice, but a caller must not be able to
    // panic the reader thread with a short slice.
    let mut bytes = vec![0u8; 16];
    assert!(!fill_tablet(&mut bytes, &captured()));
    assert_eq!(bytes, vec![0u8; 16], "nothing was written");
}

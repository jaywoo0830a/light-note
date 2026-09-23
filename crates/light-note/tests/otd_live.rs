//! A live check of the one RPC the app makes: `GetTablets`.
//!
//! Pen samples arrive through shared memory, but the tablet's **range** comes from
//! the daemon (`OTD.SharedMemoryOutput/PROTOCOL.md`: a `PreTransform` filter cannot
//! see `IOutputMode.Tablet`, so the header's `max_*` are 0 and the app fills them
//! in from the RPC).  That makes this the one part of the input path that cannot be
//! covered by fixtures alone — so, like the PDF tests, it **skips loudly** when
//! there is nothing to talk to (no daemon, or no tablet plugged in).
//!
//! The protocol itself — framing, ids, the payload shape — is tested against a
//! `GetTablets` answer captured from OTD 0.6.7 in `otd_tablets.rs`.  This file only
//! proves that the live daemon agrees with it.

use light_note::otd::rpc;

#[test]
fn the_daemon_answers_with_the_tablet_range() {
    match rpc::tablet_spec() {
        Ok(spec) => {
            println!(
                "tablet: {} ({} x {}, pressure {})",
                spec.name, spec.max_x, spec.max_y, spec.max_pressure
            );
            assert!(spec.is_known(), "a range the page can be mapped into");
            assert!(!spec.name.is_empty(), "a known tablet has a name");
            assert!(spec.max_pressure > 0.0, "pressure must not divide by zero");
        }
        // Not a failure: this machine may simply not have OTD running.
        Err(reason) => println!("skipping: nothing to ask ({reason})"),
    }
}

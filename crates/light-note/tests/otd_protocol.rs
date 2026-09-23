//! OTD shared-memory contract (v1) — the reader side of
//! `OTD.SharedMemoryOutput/PROTOCOL.md`.
//!
//! The plugin writes 64-byte samples into a 4096-slot ring and publishes a
//! monotonically increasing `write_seq`.  Two rules make this safe without any
//! lock:
//!
//! * a slot's marker is `2 * seq` when the sample is complete and
//!   `2 * seq + 1` while it is being written — a reader adopts a sample only
//!   when the marker matches **exactly**;
//! * the reader keeps its own cursor, and if it fell behind it counts the
//!   samples it will never see instead of drawing a stale line.

mod support;

use light_note::otd::{
    CAPACITY, HEADER_LEN, MAP_NAME, MAGIC, ReadStats, RingReader, SAMPLE_LEN, ShmError, VERSION,
    decode_header, decode_sample,
};
use support::flag;

#[test]
fn the_contract_constants_match_the_documented_layout() {
    assert_eq!(MAP_NAME, "light-note.otd.shm");
    assert_eq!(MAGIC, 0x4C4E_4F54_4453_4D31); // "LNOTDSM1"
    assert_eq!(VERSION, 1);
    assert_eq!(SAMPLE_LEN, 64);
    assert_eq!(CAPACITY, 4096);
    assert_eq!(HEADER_LEN, 128);
    assert_eq!(HEADER_LEN + CAPACITY * SAMPLE_LEN, 262_272);
}

#[test]
fn a_live_header_decodes_field_by_field() {
    let shm = support::Shm::live(63_460.0, 39_660.0, 8192.0);
    let header = decode_header(&shm.bytes).expect("v1 header");

    assert_eq!(header.magic, MAGIC);
    assert_eq!(header.version, VERSION);
    assert_eq!(header.sample_size, SAMPLE_LEN as u32);
    assert_eq!(header.capacity, CAPACITY as u32);
    assert!(header.is_alive(), "bit0 says the plugin is running");
    assert_eq!(header.write_seq, 0);
    assert_eq!(header.max_x, 63_460.0);
    assert_eq!(header.max_y, 39_660.0);
    assert_eq!(header.max_pressure, 8192.0);
    assert_eq!(header.tablet_name, "Wacom Intuos Pro M");
    assert!(header.is_readable());
}

#[test]
fn a_stale_mapping_is_readable_but_not_alive() {
    // The plugin was unloaded without clearing the mapping.  The app must say
    // "no tablet" instead of pretending the last coordinates are live.
    let shm = support::Shm::stale();
    let header = decode_header(&shm.bytes).expect("v1 header");

    assert!(header.is_readable());
    assert!(!header.is_alive());
}

#[test]
fn a_header_from_another_version_is_refused() {
    let shm = support::Shm::wrong_version(2);

    assert_eq!(
        decode_header(&shm.bytes),
        Err(ShmError::BadVersion { found: 2 }),
        "an unknown version is \"no plugin\", never parsed optimistically"
    );
}

#[test]
fn a_header_with_the_wrong_sample_size_is_refused() {
    let mut shm = support::Shm::live(0.0, 0.0, 0.0);
    shm.put_u32(0x0C, 32);

    assert_eq!(
        decode_header(&shm.bytes),
        Err(ShmError::BadSampleSize { found: 32 })
    );
}

#[test]
fn a_mapping_with_the_wrong_magic_or_size_is_refused() {
    let mut shm = support::Shm::live(0.0, 0.0, 0.0);
    shm.put_u64(0x00, 0xDEAD_BEEF);

    assert_eq!(decode_header(&shm.bytes), Err(ShmError::BadMagic));
    assert_eq!(decode_header(&[0u8; 16]), Err(ShmError::TooSmall));
}

#[test]
fn a_completed_sample_decodes_and_a_torn_one_is_rejected() {
    let mut shm = support::Shm::live(63_460.0, 39_660.0, 8192.0);
    shm.write_sample(1234.0, 5678.0, 4096.0, flag::TIP | flag::PROXIMITY);

    let sample = decode_sample(&shm.bytes, 1).expect("settled sample");
    assert_eq!(sample.seq, 1);
    assert_eq!(sample.x, 1234.0);
    assert_eq!(sample.y, 5678.0);
    assert_eq!(sample.pressure, 4096.0);
    assert!(sample.touches());
    assert!(!sample.is_eraser());
    assert!(!sample.out_of_range());
    assert_eq!(sample.buttons(), 0);

    // Half-written: the marker is odd, so the fields are still moving.
    shm.write_torn_sample(9.0, 9.0, 9.0, flag::TIP);
    assert_eq!(decode_sample(&shm.bytes, 2), None);

    // A marker from a *different* sample (the slot was overwritten meanwhile)
    // is not adopted either.
    shm.write_sample(1.0, 1.0, 1.0, flag::TIP);
    assert_eq!(decode_sample(&shm.bytes, 7), None, "overwritten slot");
}

#[test]
fn sample_flags_decode_tip_eraser_buttons_and_range() {
    let mut shm = support::Shm::live(1.0, 1.0, 1.0);
    shm.write_sample(0.0, 0.0, 0.0, flag::ERASER | flag::OUT_OF_RANGE | flag::BUTTON_1 | (1 << 5));

    let sample = decode_sample(&shm.bytes, 1).expect("sample");
    assert!(!sample.touches());
    assert!(sample.is_eraser());
    assert!(sample.out_of_range());
    assert_eq!(sample.buttons(), 0b11, "bits 4..=11 are pen buttons");
}

#[test]
fn the_reader_returns_new_samples_in_order_and_only_once() {
    let mut shm = support::Shm::live(63_460.0, 39_660.0, 8192.0);
    for step in 0..3 {
        shm.write_sample(step as f32 * 10.0, 0.0, 100.0, flag::TIP);
    }

    let mut reader = RingReader::new();
    let mut out = Vec::new();
    let stats = reader.read_new(&shm.bytes, 3, &mut out);

    assert_eq!(
        stats,
        ReadStats {
            read: 3,
            skipped: 0
        }
    );
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].x, 0.0);
    assert_eq!(out[2].x, 20.0);

    // Reading again with nothing new costs nothing and yields nothing.
    out.clear();
    let stats = reader.read_new(&shm.bytes, 3, &mut out);
    assert_eq!(stats.read, 0);
    assert!(out.is_empty());
}

#[test]
fn a_backlog_inside_the_ring_is_read_completely() {
    // The reader does not decide what is "too old" — it reports every sample
    // that is still in the ring, with its timestamp, and the app decides (a
    // large time gap means a new stroke rather than a straight line).
    let mut shm = support::Shm::live(63_460.0, 39_660.0, 8192.0);
    shm.write_sample(0.0, 0.0, 100.0, flag::TIP);
    let mut reader = RingReader::new();
    let mut out = Vec::new();
    reader.read_new(&shm.bytes, 1, &mut out);

    for _ in 0..50 {
        shm.write_sample(0.0, 0.0, 100.0, flag::TIP);
    }
    out.clear();
    let stats = reader.read_new(&shm.bytes, 51, &mut out);

    assert_eq!(stats.skipped, 0, "nothing fell out of the ring");
    assert_eq!(stats.read, 50);
    assert_eq!(out.first().expect("first").seq, 2);
    assert_eq!(out.last().expect("last").seq, 51);
    assert!(out.iter().all(|sample| sample.time > 0), "timestamps are kept");
}

#[test]
fn a_gap_larger_than_the_ring_is_clamped_to_the_ring() {
    let mut shm = support::Shm::live(63_460.0, 39_660.0, 8192.0);
    let latest = CAPACITY as u64 * 3;
    shm.put_u64(0x18, latest);
    // Fill the whole ring, so "the window is one ring" is what is measured.
    for seq in (latest - CAPACITY as u64 + 1)..=latest {
        let slot = HEADER_LEN + (seq as usize % CAPACITY) * SAMPLE_LEN;
        shm.put_u64(slot, seq * 2);
        shm.put_f32(slot + 0x08, seq as f32);
        shm.put_u32(slot + 0x20, flag::TIP);
    }

    let mut reader = RingReader::new();
    let mut out = Vec::new();
    let stats = reader.read_new(&shm.bytes, latest, &mut out);

    assert_eq!(stats.read, CAPACITY, "at most one ring of samples is read");
    assert_eq!(stats.skipped, latest - CAPACITY as u64, "the rest is reported, not drawn");
    assert_eq!(out.first().expect("first").seq, latest - CAPACITY as u64 + 1);
    assert_eq!(out.last().expect("last").seq, latest);
}

#[test]
fn the_ring_wraps_without_losing_samples() {
    let mut shm = support::Shm::live(1.0, 1.0, 1.0);
    let mut reader = RingReader::new();
    let mut out = Vec::new();

    // Cross the ring boundary.
    for _ in 0..(CAPACITY + 5) {
        shm.write_sample(1.0, 2.0, 3.0, flag::TIP);
    }
    let latest = shm.u64(0x18);
    reader.read_new(&shm.bytes, latest, &mut out);

    // The ring holds `CAPACITY` samples: the first five were overwritten by the
    // last five, so "everything present" is the whole ring — in order, no holes.
    assert_eq!(out.len(), CAPACITY, "the first read sees the whole ring");
    assert_eq!(
        out.first().expect("first").seq,
        latest - CAPACITY as u64 + 1,
        "the oldest sample still in the ring comes first"
    );
    assert_eq!(out.last().expect("last").seq, latest);
    assert_eq!(out.last().expect("last").pressure, 3.0);
}

#[test]
fn the_cursor_never_moves_backwards() {
    let mut shm = support::Shm::live(1.0, 1.0, 1.0);
    let mut reader = RingReader::new();
    let mut out = Vec::new();

    shm.write_sample(0.0, 0.0, 1.0, flag::TIP);
    reader.read_new(&shm.bytes, 1, &mut out);
    assert_eq!(reader.cursor(), 1);

    // A *lower* latest (the plugin restarted and reset its counter) must not
    // rewind the cursor into already-consumed slots.
    out.clear();
    let stats = reader.read_new(&shm.bytes, 0, &mut out);
    assert_eq!(stats.read, 0);
    assert_eq!(reader.cursor(), 1);
}

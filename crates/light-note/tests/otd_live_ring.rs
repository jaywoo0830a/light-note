//! A live probe of the pen stream: what the tablet *actually* reports.
//!
//! The drawing symptoms ("the line breaks into dashes", "it stutters") can come
//! from two very different places: the input (OTD/the plugin/the tablet) or the
//! app (the stroke model, the commit rules, the bake pipeline).  This test reads
//! the shared-memory ring **directly** — no app code, no batches, no mapping —
//! and prints the raw evidence, so the two can be told apart:
//!
//! * a `dt` gap larger than 20 ms inside a stream the pen is drawing with means
//!   the input is dropping reports (the app can only react to what it is given);
//! * a `flags` that clears `tip` in the middle of a stroke means the plugin is
//!   reporting a hover sample where the app expects a touch (that *is* the app's
//!   rule to fix, in `otd::shm::Sample::touches`);
//! * a stream that is continuous and tip-down everywhere means neither of those
//!   is the cause and the bug is downstream (commits, bake, display).
//!
//! It skips loudly when there is nothing to read (no daemon, no plugin), exactly
//! like `otd_live.rs` — this machine may simply not have OTD running.

use light_note::otd::{decode_header, decode_sample, reader::Mapping};

/// How many of the newest samples to look at.
const WINDOW: u64 = 3000;

#[test]
fn the_raw_pen_stream_is_continuous() {
    let Ok(mapping) = Mapping::open() else {
        println!("skipping: no OTD shared memory (is the daemon + plugin running?)");
        return;
    };
    let bytes = mapping.bytes();
    let Ok(header) = decode_header(bytes) else {
        println!("skipping: the mapping is not a light-note OTD mapping");
        return;
    };

    let latest = header.write_seq;
    if latest == 0 {
        println!("skipping: the plugin has not written a sample yet");
        return;
    }
    let from = latest.saturating_sub(WINDOW - 1);
    let samples: Vec<_> = (from..=latest)
        .filter_map(|seq| decode_sample(bytes, seq))
        .collect();
    if samples.is_empty() {
        println!("skipping: every slot in the window was overwritten");
        return;
    }

    println!(
        "tablet '{}' · seq {}..={} · {} samples still in the ring",
        header.tablet_name,
        samples.first().map(|s| s.seq).unwrap_or(0),
        latest,
        samples.len()
    );

    // Timestamp deltas: 100 ns QPC ticks (Stopwatch.Frequency is 10 MHz here).
    let mut deltas: Vec<f64> = Vec::with_capacity(samples.len());
    for pair in samples.windows(2) {
        deltas.push((pair[1].time.saturating_sub(pair[0].time)) as f64 / 10_000.0);
    }
    let mut sorted = deltas.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let max = sorted.last().copied().unwrap_or(0.0);
    let min = sorted.first().copied().unwrap_or(0.0);
    let over_20 = deltas.iter().filter(|dt| **dt > 20.0).count();
    let over_50 = deltas.iter().filter(|dt| **dt > 50.0).count();
    let backwards = samples
        .windows(2)
        .filter(|pair| pair[1].time < pair[0].time)
        .count();
    println!(
        "dt ms: min {min:.2} · median {median:.2} · max {max:.2} · >20ms {over_20} · >50ms {over_50} \
         · non-monotonic {backwards} (of {} deltas)",
        deltas.len()
    );

    // Flags: what the plugin said the pen was doing.
    let mut tip = 0usize;
    let mut no_tip = 0usize;
    let mut out_of_range = 0usize;
    let mut pressure_zero_while_tip = 0usize;
    let mut flag_values = std::collections::BTreeMap::<u32, usize>::new();
    for sample in &samples {
        *flag_values.entry(sample.flags).or_default() += 1;
        if sample.touches() {
            tip += 1;
            if sample.pressure <= 0.0 {
                pressure_zero_while_tip += 1;
            }
        } else {
            no_tip += 1;
        }
        if sample.out_of_range() {
            out_of_range += 1;
        }
    }
    println!("tip set {tip} · tip clear {no_tip} · out-of-range {out_of_range} · tip with pressure 0 {pressure_zero_while_tip}");
    println!("flags histogram: {flag_values:?}");

    // The interesting part: what happens around the biggest gaps.
    let worst: Vec<usize> = {
        let mut indexed: Vec<usize> = (0..deltas.len()).collect();
        indexed.sort_by(|a, b| deltas[*b].partial_cmp(&deltas[*a]).unwrap_or(std::cmp::Ordering::Equal));
        indexed.into_iter().take(6).collect()
    };
    for index in worst {
        let before = index.saturating_sub(2);
        let after = (index + 3).min(samples.len());
        println!("── gap of {:.2} ms after seq {}", deltas[index], samples[index].seq);
        for sample in &samples[before..after] {
            println!(
                "   seq {:>6} flags 0x{:02x} p {:.3} x {:.0} y {:.0}",
                sample.seq, sample.flags, sample.pressure, sample.x, sample.y
            );
        }
    }

    // The newest few, so the tail of the session is visible.
    println!("── newest samples");
    for sample in samples.iter().rev().take(12).rev() {
        println!(
            "   seq {:>6} flags 0x{:02x} p {:.3} x {:.0} y {:.0} t {}",
            sample.seq, sample.flags, sample.pressure, sample.x, sample.y, sample.time
        );
    }

    // Runs of the same "the tip is down" answer.  A stroke ends once per run of
    // *not* touching samples (the app calls `commit_live` on the first one and
    // then has nothing left to commit), so this — not the number of samples — is
    // what turns into "the line broke into dashes".
    let mut runs: Vec<(bool, usize, u64, u64)> = Vec::new();
    for sample in &samples {
        let down = sample.touches() && !sample.out_of_range();
        match runs.last_mut() {
            Some((state, len, _, last)) if *state == down => {
                *len += 1;
                *last = sample.seq;
            }
            _ => runs.push((down, 1, sample.seq, sample.seq)),
        }
    }
    let endings = runs.iter().filter(|(down, ..)| !*down).count();
    let down_runs: Vec<&(bool, usize, u64, u64)> = runs.iter().filter(|(down, ..)| *down).collect();
    let longest = down_runs.iter().map(|(_, len, ..)| *len).max().unwrap_or(0);
    let shortest = down_runs.iter().map(|(_, len, ..)| *len).min().unwrap_or(0);
    println!(
        "runs: {} total · {} tip-down · {} would end a stroke · tip-down length min {shortest} \
         median {} max {longest}",
        runs.len(),
        down_runs.len(),
        endings,
        down_runs.get(down_runs.len() / 2).map(|(_, len, ..)| *len).unwrap_or(0),
    );
    // The decisive question: when the tip flag drops in the middle of a motion,
    // did the pen leave the tablet, or did the tablet stop reporting pressure?
    //
    // If the coordinates keep flowing smoothly across the transition — the step
    // per report stays the same order of magnitude as it was while the tip was
    // down — then the pen never lifted: the app's "no tip flag = the pen is up"
    // rule is what cut the stroke.  If the pen really left, the up-run's motion
    // is unrelated to the stroke (a jump, or a much faster sweep away).
    let step = |from: &light_note::otd::Sample, to: &light_note::otd::Sample| -> f32 {
        ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt()
    };
    println!("── transitions (up-run between two tip-down runs)");
    let mut index = 0usize;
    while index < runs.len() {
        let (down, len, first, last) = runs[index];
        if down || index == 0 || index + 1 >= runs.len() {
            index += 1;
            continue;
        }
        let before: Vec<&light_note::otd::Sample> = samples
            .iter()
            .filter(|sample| sample.seq >= first.saturating_sub(24) && sample.seq < first)
            .collect();
        let after: Vec<&light_note::otd::Sample> = samples
            .iter()
            .filter(|sample| sample.seq > last && sample.seq <= last + 24)
            .collect();
        let up: Vec<&light_note::otd::Sample> = samples
            .iter()
            .filter(|sample| sample.seq >= first && sample.seq <= last)
            .collect();
        if let (Some(entry), Some(exit)) = (before.last(), after.first()) {
            let last_down = before.last().copied().unwrap_or(entry);
            let first_down = after.first().copied().unwrap_or(exit);
            let step_before = if before.len() > 1 {
                let mut steps: Vec<f32> = before.windows(2).map(|pair| step(pair[0], pair[1])).collect();
                steps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                steps[steps.len() / 2]
            } else {
                0.0
            };
            let step_after = if after.len() > 1 {
                let mut steps: Vec<f32> = after.windows(2).map(|pair| step(pair[0], pair[1])).collect();
                steps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                steps[steps.len() / 2]
            } else {
                0.0
            };
            let jump = step(last_down, first_down);
            let up_path: f32 = up.windows(2).map(|pair| step(pair[0], pair[1])).sum();
            let verdict = if step_before > 0.0 && jump <= step_before * 3.0 + step_after * 3.0 {
                "CONTINUOUS — the pen never lifted (the tablet dropped pressure)"
            } else {
                "a real lift or a jump (check the numbers)"
            };
            println!(
                "   up-run seq {first}..{last} len {len} · step before {step_before:.0} · step after \
                 {step_after:.0} · jump across it {jump:.0} · path during it {up_path:.0} units · \
                 {verdict}"
            );
        }
        index += 1;
    }
    println!("── runs (only the short ones — a short tip-down run is a dash)");
    let mut printed = 0usize;
    for (down, len, first, last) in &runs {
        if printed >= 40 {
            println!("   … ({} more runs)", runs.len() - printed);
            break;
        }
        if *len > 80 {
            continue;
        }
        printed += 1;
        let head = samples
            .iter()
            .find(|sample| sample.seq == *first)
            .map(|sample| (sample.x, sample.y, sample.flags, sample.pressure));
        let tail = samples
            .iter()
            .find(|sample| sample.seq == *last)
            .map(|sample| (sample.x, sample.y, sample.flags, sample.pressure));
        if let (Some(head), Some(tail)) = (head, tail) {
            println!(
                "   {:>4} seq {:>6}..{:<6} len {:>4} flags 0x{:02x}..0x{:02x} p {:.0}..{:.0} \
                 x {:.0}..{:.0} y {:.0}..{:.0}",
                if *down { "DOWN" } else { "up" },
                first,
                last,
                len,
                head.2,
                tail.2,
                head.3,
                tail.3,
                head.0,
                tail.0,
                head.1,
                tail.1,
            );
        }
    }
    assert!(
        samples.windows(2).all(|pair| pair[1].seq > pair[0].seq),
        "the ring hands out samples in order"
    );
}

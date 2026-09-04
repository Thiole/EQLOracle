//! Tests for `replay`. Kept out of the production module by
//! convention: src/ contains shipping code only.

use eqlp_source::clock::VirtualClock;
use eqlp_source::replay::{Replay, Speed};

const LOG: &[u8] = b"[Wed Aug 06 21:14:33 2025] one\r\n\
                     [Wed Aug 06 21:14:33 2025] two\r\n\
                     [Wed Aug 06 21:14:38 2025] three\r\n\
                     no timestamp here\r\n\
                     [Wed Aug 06 21:15:03 2025] four\r\n";

#[test]
fn clock_follows_the_log_not_the_wall() {
    let c = VirtualClock::new(0);
    let mut r = Replay::new(LOG, Speed::Instant);
    let start = r.first_timestamp_ms().unwrap();
    c.set_at_least(start);

    let mut got = Vec::new();
    r.run_to_end(&c, |l| got.push(String::from_utf8_lossy(l).into_owned()));

    assert_eq!(got.len(), 5);
    assert_eq!(r.undated_lines, 1, "undated lines are emitted, not dropped");
    // 21:14:33 -> 21:15:03 is exactly 30s of log time.
    assert_eq!(c.now_ms() - start, 30_000);
}

#[test]
fn run_until_stops_before_events_from_the_future() {
    let c = VirtualClock::new(0);
    let mut r = Replay::new(LOG, Speed::Instant);
    let start = r.first_timestamp_ms().unwrap();
    c.set_at_least(start);

    let mut got = Vec::new();
    // Advance 10s: the two at :33, the one at :38, and the undated line
    // that follows it -- an undated line is a continuation of the event
    // above it and carries that event's time, so it must come through on
    // the same side of the boundary. Only :15:03 is in the future.
    assert!(r.run_until(&c, start + 10_000, |l| got
        .push(String::from_utf8_lossy(l).into_owned())));
    assert_eq!(got.len(), 4, "{got:?}");
    assert!(got.last().unwrap().contains("no timestamp"));
    assert_eq!(c.now_ms(), start + 10_000);

    got.clear();
    assert!(!r.run_until(&c, start + 60_000, |l| got
        .push(String::from_utf8_lossy(l).into_owned())));
    assert_eq!(got.len(), 1, "only the :15:03 line remains");
}

/// The property the whole strategy rests on: same fixture, same output,
/// every time, regardless of speed setting or wall clock.
#[test]
fn replay_is_deterministic_across_speeds() {
    let run = |speed| {
        let c = VirtualClock::new(0);
        let mut r = Replay::new(LOG, speed);
        c.set_at_least(r.first_timestamp_ms().unwrap());
        let mut out = Vec::new();
        r.run_to_end(&c, |l| out.push(l.to_vec()));
        (out, c.now_ms())
    };
    let a = run(Speed::Instant);
    let b = run(Speed::Realtime);
    let d = run(Speed::Scaled(8.0));
    assert_eq!(a, b);
    assert_eq!(a, d);
}

#[test]
fn speed_controls_wall_delay_only() {
    assert_eq!(Speed::Instant.wall_delay_ms(5000), 0);
    assert_eq!(Speed::Realtime.wall_delay_ms(5000), 5000);
    assert_eq!(Speed::Scaled(10.0).wall_delay_ms(5000), 500);
    // A backwards jump in the log must never become a negative sleep.
    assert_eq!(Speed::Realtime.wall_delay_ms(-5000), 0);
}

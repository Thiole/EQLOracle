//! Tests for `rolling`. Kept out of the production module by
//! convention: src/ contains shipping code only.

use eqlp_session::rolling::Rolling;

#[test]
fn ramps_in_instead_of_spiking() {
    let mut r = Rolling::new(10_000);
    // One big hit at t=0 must not read as an enormous DPS.
    r.push(0, 600);
    assert!(r.dps(0) <= 400.0, "spiked to {}", r.dps(0));
    assert!(r.dps(500) <= 400.0);
}

#[test]
fn steady_rate_reads_as_that_rate() {
    // 100 damage every second, sampled well after the window has filled.
    // Anything other than ~100 here is a fencepost bug.
    let mut r = Rolling::new(5_000);
    for i in 0..30 {
        r.push(i * 1000, 100);
    }
    let d = r.dps(29_000);
    assert!((d - 100.0).abs() < 1.0, "steady 100/s read as {d}");

    let mut r10 = Rolling::new(10_000);
    for i in 0..30 {
        r10.push(i * 1000, 100);
    }
    let d10 = r10.dps(29_000);
    assert!((d10 - 100.0).abs() < 1.0, "steady 100/s read as {d10}");
}

#[test]
fn window_slides_and_decays_to_zero() {
    let mut r = Rolling::new(10_000);
    for i in 0..10 {
        r.push(i * 1000, 100);
    }
    // 1000 damage, 9s elapsed since the first hit -> 111. The window has
    // not filled yet, so this is elapsed-rate, not window-rate.
    let at9 = r.dps(9_000);
    assert!((at9 - 111.0).abs() < 5.0, "{at9}");
    // Fight goes quiet: the number must fall, not freeze.
    assert!(r.dps(15_000) < at9);
    assert_eq!(r.dps(30_000), 0.0);
}

#[test]
fn overall_differs_from_window() {
    let mut r = Rolling::new(5_000);
    for i in 0..20 {
        r.push(i * 1000, 100);
    }
    let w = r.dps(19_000);
    let o = r.dps_overall(19_000);
    assert!((w - 100.0).abs() < 1.0, "window {w}");
    assert!((o - 105.0).abs() < 10.0, "overall {o}");
    assert_eq!(r.total, 2000);
}

#[test]
fn memory_is_bounded_by_the_window() {
    let mut r = Rolling::new(10_000);
    for i in 0..100_000i64 {
        r.push(i * 100, 1);
    }
    assert!(r.buffered() <= 101, "leaked {} events", r.buffered());
    assert_eq!(r.total, 100_000);
}

//! Tests for `clock`. Kept out of the production module by
//! convention: src/ contains shipping code only.

use eqlp_source::clock::VirtualClock;

#[test]
fn virtual_clock_never_goes_backwards() {
    let c = VirtualClock::at_unix_secs(1000);
    c.set_at_least(2000 * 1000);
    assert_eq!(c.now_secs(), 2000);
    c.set_at_least(500 * 1000); // a backwards timestamp in the log
    assert_eq!(c.now_secs(), 2000, "time must not rewind");
}

#[test]
fn advance_is_exact() {
    let c = VirtualClock::new(0);
    for _ in 0..1000 {
        c.advance_ms(7);
    }
    assert_eq!(c.now_ms(), 7000);
}

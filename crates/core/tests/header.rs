//! Tests for `header`. Kept out of the production module by
//! convention: src/ contains shipping code only.

use eqlp_core::header::{BracketCtime, HeaderParser};

#[test]
fn parses_zero_padded() {
    let (ts, off) = BracketCtime
        .parse(b"[Wed Aug 06 21:14:33 2025] You hit it.")
        .unwrap();
    assert_eq!(off, 27);
    // 2025-08-06T21:14:33
    assert_eq!(ts.0, 1_754_514_873);
}

#[test]
fn parses_space_padded_day() {
    let (a, _) = BracketCtime.parse(b"[Wed Aug  6 21:14:33 2025] x").unwrap();
    let (b, _) = BracketCtime.parse(b"[Wed Aug 06 21:14:33 2025] x").unwrap();
    assert_eq!(a, b);
}

#[test]
fn rejects_garbage_without_panicking() {
    for bad in [
        &b""[..],
        b"[",
        b"[Wed Aug 06 21:14:33 2025",
        b"[Wed Xyz 06 21:14:33 2025] x",
        b"[Wed Aug 06 99:14:33 2025] x",
        b"[Wed Aug 00 21:14:33 2025] x",
        b"----------------------------",
    ] {
        assert!(BracketCtime.parse(bad).is_none(), "accepted {:?}", bad);
    }
}

#[test]
fn monotonic_across_year_boundary() {
    let a = BracketCtime
        .parse(b"[Wed Dec 31 23:59:59 2025] x")
        .unwrap()
        .0;
    let b = BracketCtime
        .parse(b"[Thu Jan 01 00:00:00 2026] x")
        .unwrap()
        .0;
    assert_eq!(b.0 - a.0, 1);
}

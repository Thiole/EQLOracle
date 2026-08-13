//! Tests for `field`. Kept out of the production module by
//! convention: src/ contains shipping code only.

use eqlp_core::field::parse_u64;

#[test]
fn u64_edges() {
    assert_eq!(parse_u64(b"0"), Some(0));
    assert_eq!(parse_u64(b"1,234"), Some(1234));
    assert_eq!(parse_u64(b""), None);
    assert_eq!(parse_u64(b"12a"), None);
    assert_eq!(parse_u64(b"18446744073709551615"), Some(u64::MAX));
    assert_eq!(parse_u64(b"18446744073709551616"), None);
}

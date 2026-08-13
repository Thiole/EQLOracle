//! Tests for `frame`. Kept out of the production module by
//! convention: src/ contains shipping code only.

use eqlp_core::frame::Framer;

fn collect_chunked(data: &[u8], chunk: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut fr = Framer::default();
    for c in data.chunks(chunk) {
        fr.push(c, |l| out.push(l.to_vec()));
    }
    fr.flush(|l| out.push(l.to_vec()));
    out
}

#[test]
fn chunking_is_invisible() {
    let data = b"alpha\r\nbeta\ngamma\n\ndelta";
    let whole = collect_chunked(data, data.len());
    for n in 1..=data.len() {
        assert_eq!(collect_chunked(data, n), whole, "chunk size {n}");
    }
    assert_eq!(whole.len(), 5);
    assert_eq!(whole[0], b"alpha");
    assert_eq!(whole[3], b"");
    assert_eq!(whole[4], b"delta");
}

#[test]
fn overlong_line_is_bounded_and_resyncs() {
    let mut fr = Framer::new(64);
    let mut out = Vec::new();
    let junk = vec![b'x'; 10_000];
    fr.push(&junk, |l| out.push(l.len()));
    fr.push(b"\nok\n", |l| out.push(l.len()));
    assert!(fr.pending() == 0);
    assert_eq!(fr.truncated, 1);
    assert_eq!(out.last().copied(), Some(2));
    assert!(out.iter().all(|&n| n <= 64));
}

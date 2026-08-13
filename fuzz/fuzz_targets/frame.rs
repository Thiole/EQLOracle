#![no_main]
//! Framing must not depend on chunk boundaries, and must stay bounded.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (Vec<u8>, u8)| {
    let (data, chunk) = input;
    let chunk = (chunk as usize).max(1);

    let batch: Vec<Vec<u8>> = eqlp_core::frame::lines(&data).map(|l| l.to_vec()).collect();

    let mut fr = eqlp_core::frame::Framer::default();
    let mut got: Vec<Vec<u8>> = Vec::new();
    for c in data.chunks(chunk) {
        fr.push(c, |l| got.push(l.to_vec()));
        assert!(fr.pending() <= eqlp_core::frame::DEFAULT_MAX_LINE);
    }
    fr.flush(|l| got.push(l.to_vec()));

    if fr.truncated == 0 {
        assert_eq!(got, batch, "chunking changed the result at chunk={chunk}");
    }
});

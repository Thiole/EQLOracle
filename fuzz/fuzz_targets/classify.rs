#![no_main]
//! Arbitrary bytes through the classifier. Asserts totality and span safety:
//! every line yields exactly one outcome, and no span escapes its buffer.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let eng = eqlp_fuzz::engine();
    let mut m = eng.matcher();
    for line in eqlp_core::frame::lines(data) {
        match m.classify(line) {
            eqlp_core::Outcome::Matched(mm) => {
                assert!(mm.body.end as usize <= line.len());
                assert!(mm.body.start <= mm.body.end);
                for c in mm.caps.iter().flatten() {
                    assert!(c.start >= mm.body.start && c.end <= mm.body.end);
                    let _ = c.slice(line);
                }
            }
            eqlp_core::Outcome::Unmatched { body, .. }
            | eqlp_core::Outcome::Headerless { body } => {
                assert!(body.end as usize <= line.len());
                let _ = body.slice(line);
            }
            eqlp_core::Outcome::Blank => {}
        }
    }
});

#![no_main]
//! Arbitrary TOML into the pack loader. Must reject cleanly, never panic, and
//! never build an engine that then panics on real input.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|s: String| {
    if let Ok(p) = eqlp_core::Pack::from_toml(&s) {
        if let Ok(r) = eqlp_core::ResolvedPack::layer(vec![p]) {
            if let Ok(eng) = eqlp_core::Engine::build(&r) {
                let mut m = eng.matcher();
                let _ = m.classify(b"[Wed Aug 06 21:14:33 2025] You hit a rat for 1 point of damage.");
            }
        }
    }
});

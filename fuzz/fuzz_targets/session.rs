#![no_main]
//! Encounter invariants under arbitrary event orderings, including out-of-order
//! and duplicated timestamps.
use libfuzzer_sys::fuzz_target;
use eqlp_session::Tracker;

#[derive(arbitrary::Arbitrary, Debug)]
enum Ev {
    Damage { ts: u32, src: u8, tgt: u8, amt: u16 },
    Death { ts: u32, tgt: u8 },
    Tick { ts: u32 },
}

fuzz_target!(|evs: Vec<Ev>| {
    let mut t = Tracker::default();
    for e in evs {
        match e {
            Ev::Damage { ts, src, tgt, amt } => {
                t.damage(ts as i64, &format!("s{src}"), &format!("m{tgt}"), amt as u64)
            }
            Ev::Death { ts, tgt } => t.death(ts as i64, &format!("m{tgt}")),
            Ev::Tick { ts } => t.tick(ts as i64),
        }
    }
    t.close_all(i64::from(u32::MAX));
    for e in &t.done {
        assert!(e.end_ms.is_some());
        let by: u64 = e.by_source.values().map(|r| r.total).sum();
        assert_eq!(by, e.total, "per-source split lost damage");
    }
});

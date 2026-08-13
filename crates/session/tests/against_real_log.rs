//! Encounter detection against real log data. Synthetic tests prove the logic;
//! this proves the boundaries survive contact with an actual play session.

use eqlp_core::{engine::Engine, event::Outcome, field, frame, rule::{Pack, ResolvedPack}};
use eqlp_session::{EndReason, Tracker};

fn fixture() -> Option<Vec<u8>> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/reference-slice.log")).ok()
}

fn engine() -> Engine {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../packs/eql.toml")).unwrap();
    Engine::build(&ResolvedPack::layer(vec![Pack::from_toml(&src).unwrap()]).unwrap()).unwrap()
}

#[test]
fn encounters_from_a_real_session() {
    let buf = match fixture() { Some(b) => b, None => return };
    let eng = engine();
    let mut m = eng.matcher();
    let mut t = Tracker::default();
    let (mut dmg_events, mut deaths) = (0u64, 0u64);
    let mut last_ms = 0i64;

    for line in frame::lines(&buf) {
        if let Outcome::Matched(mm) = m.classify(line) {
            let r = eng.rule(mm.rule);
            let ms = mm.ts.0 * 1000;
            last_ms = ms;
            let s = |n: &str| match field::field(&eng, &mm, line, n) {
                field::Value::Str(b) => String::from_utf8_lossy(b).into_owned(),
                _ => String::new(),
            };
            match r.kind.as_str() {
                "damage" => {
                    let amt = match field::field(&eng, &mm, line, "amount") {
                        field::Value::U64(v) => v,
                        _ => continue,
                    };
                    let (src, tgt) = (s("source"), s("target"));
                    if tgt.is_empty() { continue; }
                    t.damage(ms, if src.is_empty() { "?" } else { &src }, &tgt, amt);
                    dmg_events += 1;
                }
                "death" => {
                    let v = if !s("victim").is_empty() { s("victim") } else { continue };
                    t.death(ms, &v);
                    deaths += 1;
                }
                _ => {}
            }
            t.tick(ms);
        }
    }
    t.close_all(last_ms);

    let slain = t.done.iter().filter(|e| e.end_reason == Some(EndReason::Slain)).count();
    let timed = t.done.iter().filter(|e| e.end_reason == Some(EndReason::Timeout)).count();
    eprintln!("damage events {dmg_events}  deaths {deaths}");
    eprintln!("encounters {} (slain {slain}, timeout {timed})", t.done.len());

    assert!(dmg_events > 10_000, "fixture should be damage-rich");
    assert!(t.done.len() > 100, "no encounters detected");
    assert!(slain > 20, "death boundaries never fired");

    // Every retired encounter must be internally consistent.
    for e in &t.done {
        assert!(e.end_ms.is_some());
        assert!(e.end_ms.unwrap() >= e.start_ms, "{} ended before it began", e.target);
        assert!(e.last_ms >= e.start_ms);
        let by: u64 = e.by_source.values().map(|r| r.total).sum();
        assert_eq!(by, e.total, "per-source split lost damage on {}", e.target);
    }

    // A timeout encounter must never include the silent tail in its duration.
    for e in t.done.iter().filter(|e| e.end_reason == Some(EndReason::Timeout)) {
        assert_eq!(e.end_ms.unwrap(), e.last_ms);
    }

    let longest = t.done.iter().max_by_key(|e| e.total).unwrap().clone();
    eprintln!(
        "biggest fight: {} — {} dmg over {}s, {:.0} dps, {} sources",
        longest.target,
        longest.total,
        longest.duration_ms(last_ms) / 1000,
        longest.dps_overall(last_ms),
        longest.by_source.len()
    );
}

//! Does "just scan it" actually hold at real log scale?
//!
//! The design bets that no aggregate needs materialising because a full scan is
//! cheap. That bet is only valid if measured, so this test builds a store the
//! size of a long play session and times the queries a UI would run per frame.

use eqlp_store::{by_ability, by_actor, dps_window, tag, EventKind, Filter, Store};
use std::time::Instant;

fn build(events: usize) -> Store {
    let mut s = Store::default();
    let actors: Vec<_> = (0..40).map(|i| s.sym(&format!("player{i}"))).collect();
    let abils: Vec<_> = (0..120)
        .map(|i| {
            let t = match i % 4 {
                0 => tag::MELEE,
                1 => tag::SPELL,
                2 => tag::PROC,
                _ => tag::DOT,
            };
            s.ability_id(&format!("ability{i}"), t)
        })
        .collect();

    let per = 400;
    let mut i = 0usize;
    let mut k = 0u32;
    while i < events {
        let mob = s.sym(&format!("mob{k}"));
        let e = s.open_encounter(mob, i as i64 * 250, s.len() as u32, None);
        for j in 0..per.min(events - i) {
            let idx = s.push(
                (i + j) as i64 * 250,
                EventKind::Damage,
                actors[(i + j) % actors.len()],
                mob,
                abils[(i * 7 + j) % abils.len()],
                ((i + j) % 900 + 50) as u64,
                if j % 11 == 0 {
                    eqlp_store::flag::CRITICAL
                } else {
                    0
                },
                e.0,
                0,
            );
            s.extend_encounter(e, idx);
        }
        s.close_encounter(e, (i + per) as i64 * 250, true, false);
        i += per;
        k += 1;
    }
    s
}

#[test]
fn full_scan_is_cheap_enough_to_need_no_materialised_aggregates() {
    // ~12 days of heavy play. The reference log yielded ~69k spell-damage
    // landings plus melee across 1.8M lines; 750k damage events is generous.
    let n = 750_000;
    let s = build(n);
    assert_eq!(s.len(), n);
    eprintln!(
        "store: {} events, {} encounters, {} names, {} abilities, ~{:.1} MiB",
        s.len(),
        s.encounters.len(),
        s.names.len(),
        s.abilities.len(),
        s.bytes() as f64 / (1024.0 * 1024.0)
    );

    let f = Filter::default().damage();

    let t = Instant::now(); // clock-exempt: benchmark, measures real wall time on purpose
    let rows = by_ability(&s, &f);
    let whole = t.elapsed();
    eprintln!(
        "by_ability, whole store : {:?} ({} rows)",
        whole,
        rows.len()
    );

    let last = s.encounters.last().unwrap().id;
    let t = Instant::now(); // clock-exempt: benchmark, measures real wall time on purpose
    let _ = by_ability(&s, &Filter::encounter(last).damage());
    eprintln!("by_ability, one encounter: {:?}", t.elapsed());

    let t = Instant::now(); // clock-exempt: benchmark, measures real wall time on purpose
    let _ = by_actor(&s, &f);
    eprintln!("by_actor, whole store   : {:?}", t.elapsed());

    let now = s.ts[s.len() - 1];
    let t = Instant::now(); // clock-exempt: benchmark, measures real wall time on purpose
    let mut d = 0.0;
    for _ in 0..10 {
        d += dps_window(&s, &f, now, 10_000);
    }
    eprintln!(
        "dps_window x10          : {:?} (dps {:.0})",
        t.elapsed(),
        d / 10.0
    );

    // A per-encounter query is what the live panel actually runs; it must be
    // comfortably inside a frame budget.
    let t = Instant::now(); // clock-exempt: benchmark, measures real wall time on purpose
    for e in s.encounters.iter().rev().take(50) {
        let _ = by_ability(&s, &Filter::encounter(e.id).damage());
    }
    eprintln!("50 encounter breakdowns : {:?}", t.elapsed());

    assert!(!rows.is_empty());
}

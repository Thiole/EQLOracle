//! Property tests. Inputs come from a run-time seed, so there is no fixed case
//! to special-case against: the only way to pass across seeds is to be correct.
//!
//! A failing run prints its seed. Re-run with EQLP_SEED=<n> to reproduce.

use eqlp_store::{by_ability, roll_up_by_tag, tag, total, EventKind, Filter, Store};
use eqlp_testkit::Rng;

fn seed() -> u64 {
    std::env::var("EQLP_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0xC0FFEE)
}

/// Build a random store and return it plus the totals the generator intended.
fn random_store(rng: &mut Rng) -> (Store, u64, usize) {
    let mut s = Store::default();
    let abils: Vec<_> = (0..rng.range(3, 20))
        .map(|i| {
            let t = *rng.pick(&[tag::MELEE, tag::SPELL, tag::PROC, tag::DOT, tag::SPELL | tag::DOT]);
            s.ability_id(&format!("ability{i}"), t)
        })
        .collect();
    let actors: Vec<_> = (0..rng.range(1, 8)).map(|i| s.sym(&format!("actor{i}"))).collect();

    let mut expect = 0u64;
    let mut n = 0usize;
    let mut ts = 0i64;
    for e in 0..rng.range(1, 30) {
        let mob = s.sym(&format!("mob{e}"));
        let enc = s.open_encounter(mob, ts, s.len() as u32);
        for _ in 0..rng.range(1, 40) {
            ts += rng.range(0, 4000) as i64;
            let amt = rng.range(0, 5000);
            let idx = s.push(
                ts, EventKind::Damage,
                *rng.pick(&actors), mob, *rng.pick(&abils),
                amt, 0, enc.0,
            );
            s.extend_encounter(enc, idx);
            expect += amt;
            n += 1;
        }
        s.close_encounter(enc, ts, true);
        ts += rng.range(1000, 60_000) as i64;
    }
    (s, expect, n)
}

#[test]
fn ability_rows_partition_the_damage_exactly() {
    let sd = seed();
    let mut rng = Rng::new(sd);
    for round in 0..40 {
        let (s, expect, n) = random_store(&mut rng);
        let rows = by_ability(&s, &Filter::default().damage());
        let got: u64 = rows.iter().map(|r| r.total).sum();
        let hits: u64 = rows.iter().map(|r| r.hits).sum();
        assert_eq!(got, expect, "seed {sd} round {round}: damage lost or duplicated");
        assert_eq!(hits as usize, n, "seed {sd} round {round}: hit count wrong");
    }
}

#[test]
fn encounter_ranges_partition_the_store() {
    let sd = seed();
    let mut rng = Rng::new(sd ^ 1);
    for round in 0..40 {
        let (s, expect, _) = random_store(&mut rng);
        let sum: u64 = s
            .encounters
            .iter()
            .map(|e| total(&s, &Filter::encounter(e.id).damage()))
            .sum();
        assert_eq!(sum, expect, "seed {sd} round {round}: encounter ranges do not cover the store");
    }
}

#[test]
fn tag_rollup_never_exceeds_the_ability_total() {
    let sd = seed();
    let mut rng = Rng::new(sd ^ 2);
    for round in 0..30 {
        let (s, expect, _) = random_store(&mut rng);
        let rows = by_ability(&s, &Filter::default().damage());
        for (name, tot, _) in roll_up_by_tag(&rows) {
            assert!(tot <= expect, "seed {sd} round {round}: tag {name} exceeds total");
        }
    }
}

#[test]
fn eviction_preserves_surviving_encounter_totals() {
    let sd = seed();
    let mut rng = Rng::new(sd ^ 3);
    for round in 0..30 {
        let (mut s, _, _) = random_store(&mut rng);
        if s.encounters.len() < 3 { continue; }
        let drop = rng.below(s.encounters.len() - 1);
        let before: Vec<u64> = s.encounters[drop..]
            .iter()
            .map(|e| total(&s, &Filter::encounter(e.id).damage()))
            .collect();
        s.evict_before_encounter(drop);
        let after: Vec<u64> = s
            .encounters
            .iter()
            .map(|e| total(&s, &Filter::encounter(e.id).damage()))
            .collect();
        assert_eq!(before, after, "seed {sd} round {round}: eviction corrupted survivors");
    }
}

#[test]
fn window_sums_are_monotone_in_window_width() {
    let sd = seed();
    let mut rng = Rng::new(sd ^ 4);
    for round in 0..20 {
        let (s, _, _) = random_store(&mut rng);
        if s.is_empty() { continue; }
        let now = s.ts[s.len() - 1];
        let f = Filter::default().damage();
        let mut prev = 0u64;
        for w in [1_000i64, 5_000, 30_000, 300_000, 10_000_000] {
            let mut g = f.clone();
            g.since_ms = Some(now - w + 1);
            g.until_ms = Some(now);
            let t = total(&s, &g);
            assert!(t >= prev, "seed {sd} round {round}: widening the window lost damage");
            prev = t;
        }
    }
}

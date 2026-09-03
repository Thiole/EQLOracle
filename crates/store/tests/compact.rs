//! why: Store::compact_before folds a finished fight's combat rows into
//! one row per key with `count` -- totals and counts must read the
//! same through the query layer, open fights and loot rows stay raw,
//! encounter ranges still land on their own rows.

use eqlp_store::{by_ability, by_actor, flag, total, EventKind, Filter, Store};

#[test]
fn a_finished_fights_rows_fold_and_still_add_up() {
    let mut s = Store::default();
    let you = s.sym("You");
    let rat = s.sym("a rat");
    let bat = s.sym("a bat");
    let slash = s.ability_id("Slash", 0);
    let loot = s.ability_id("Rusty Dagger", 0);
    let e0 = s.open_encounter(rat, 1_000, 0, None);
    for i in 0..10i64 {
        let ts = 1_000 + i * 500;
        let fl = if i % 2 == 0 { flag::CRITICAL } else { 0 };
        let idx = s.push(
            ts,
            EventKind::Damage,
            you,
            rat,
            slash,
            10 + i as u64,
            fl,
            e0.0,
            1,
        );
        s.extend_encounter(e0, idx);
    }
    let idx = s.push(6_000, EventKind::Loot, you, rat, loot, 1, 0, e0.0, 1);
    s.extend_encounter(e0, idx);
    s.close_encounter(e0, 6_500, true, false);
    let e1 = s.open_encounter(bat, 7_000, s.len() as u32, None);
    for i in 0..4i64 {
        let idx = s.push(
            7_000 + i * 500,
            EventKind::Damage,
            you,
            bat,
            slash,
            5,
            0,
            e1.0,
            1,
        );
        s.extend_encounter(e1, idx);
    }
    let before_total = total(&s, &Filter::encounter(e0).damage());
    let before_rows = by_ability(&s, &Filter::encounter(e0).damage());
    assert_eq!(before_rows[0].hits, 10);
    assert_eq!(before_rows[0].crits, 5);

    let removed = s.compact_before(10_000);
    // why: 10 rows -> 2 (crit and non-crit keys); loot and the open fight untouched
    assert_eq!(removed, 8, "len now {}", s.len());
    assert_eq!(s.len(), 2 + 1 + 4);
    let rows = by_ability(&s, &Filter::encounter(e0).damage());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].hits, 10);
    assert_eq!(rows[0].crits, 5);
    assert_eq!(rows[0].total, before_total);
    assert_eq!(total(&s, &Filter::encounter(e0).damage()), before_total);
    let actors = by_actor(&s, &Filter::encounter(e0).damage());
    assert_eq!(actors[0].2, 10, "hits weigh by count");
    assert_eq!(total(&s, &Filter::encounter(e0).kind(EventKind::Loot)), 1);
    assert_eq!(
        total(&s, &Filter::encounter(e1).damage()),
        20,
        "open fight stays raw"
    );
    assert_eq!(s.encounter(e1).map(|e| e.range().len()), Some(4));
    // why: idempotent -- a second pass finds nothing to fold
    assert_eq!(s.compact_before(10_000), 0);
}

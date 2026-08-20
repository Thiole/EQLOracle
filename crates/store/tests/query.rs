//! Store and query behaviour.

use eqlp_store::{by_ability, roll_up_by_tag, tag, EventKind, Filter, Store};

fn seed() -> Store {
    let mut s = Store::default();
    let you = s.sym("You");
    let mob = s.sym("a gnoll");

    let backstab = s.ability_id("Backstab", tag::MELEE);
    let slash = s.ability_id("Slash", tag::MELEE);
    let burn = s.ability_id("Burn", tag::PROC | tag::DOT);
    let nuke = s.ability_id("Ice Comet", tag::SPELL);

    let e = s.open_encounter(mob, 0, 0, None);
    for (i, (ab, amt, fl)) in [
        (backstab, 900, eqlp_store::flag::CRITICAL),
        (backstab, 400, 0),
        (slash, 100, 0),
        (slash, 120, 0),
        (burn, 40, 0),
        (burn, 40, 0),
        (burn, 40, 0),
        (nuke, 700, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let idx = s.push(
            i as i64 * 1000,
            EventKind::Damage,
            you,
            mob,
            ab,
            amt,
            fl,
            e.0,
            0,
        );
        s.extend_encounter(e, idx);
    }
    s.close_encounter(e, 8000, true, false);
    s
}

#[test]
fn rows_are_abilities_not_mechanisms() {
    let s = seed();
    let rows = by_ability(&s, &Filter::default().damage());
    let names: Vec<&str> = rows.iter().map(|r| s.ability_name(r.ability)).collect();
    // Backstab and Burn are separate rows despite both being "damage", and
    // Backstab is not folded into a generic melee bucket.
    assert!(names.contains(&"Backstab"));
    assert!(names.contains(&"Burn"));
    assert!(names.contains(&"Slash"));
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].total, 1300, "sorted by total, Backstab first");
}

#[test]
fn a_melee_ability_is_directly_comparable_to_a_proc() {
    let s = seed();
    let rows = by_ability(&s, &Filter::default().damage());
    let find = |n: &str| {
        rows.iter()
            .find(|r| s.ability_name(r.ability) == n)
            .unwrap()
    };
    let bs = find("Backstab");
    let burn = find("Burn");
    assert!(bs.tags & tag::MELEE != 0);
    assert!(burn.tags & tag::PROC != 0);
    // Same shape of row, so a UI can put them side by side.
    assert_eq!(bs.hits, 2);
    assert_eq!(burn.hits, 3);
    assert!(bs.mean() > burn.mean());
    assert_eq!(bs.crits, 1);
}

#[test]
fn tag_rollup_is_derived_and_cannot_disagree() {
    let s = seed();
    let rows = by_ability(&s, &Filter::default().damage());
    let roll = roll_up_by_tag(&rows);
    let melee = roll.iter().find(|(n, _, _)| *n == "melee").unwrap();
    assert_eq!(melee.1, 900 + 400 + 100 + 120);
    // Burn carries two tags and is counted under both, by design.
    assert_eq!(roll.iter().find(|(n, _, _)| *n == "proc").unwrap().1, 120);
    assert_eq!(roll.iter().find(|(n, _, _)| *n == "dot").unwrap().1, 120);
}

#[test]
fn tag_filter_selects_without_a_second_index() {
    let s = seed();
    let f = Filter::default().damage().with_tags(tag::MELEE);
    let rows = by_ability(&s, &f);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().map(|r| r.total).sum::<u64>(), 1520);
}

#[test]
fn encounter_is_a_range_and_stores_no_damage() {
    let s = seed();
    let e = s.encounter(eqlp_store::EncounterId(0)).unwrap();
    assert_eq!(e.range(), 0..8);
    assert!(e.slain);
    // The encounter struct has no total field; the number comes from the range.
    assert_eq!(
        eqlp_store::total(&s, &Filter::encounter(e.id).damage()),
        2340
    );
}

#[test]
fn window_dps_needs_no_ring_buffer() {
    let s = seed();
    let f = Filter::default().damage();
    // events at t=0..7s. Trailing 3s window at t=7s covers t=5,6,7.
    let d = eqlp_store::dps_window(&s, &f, 7000, 3000);
    assert_eq!(d, (40 + 40 + 700) as f64 / 3.0);
}

#[test]
fn eviction_never_splits_an_encounter() {
    let mut s = Store::default();
    let a = s.sym("You");
    let ab = s.ability_id("Slash", tag::MELEE);
    for k in 0..5 {
        let m = s.sym(&format!("mob{k}"));
        let e = s.open_encounter(m, k * 10_000, s.len() as u32, None);
        for j in 0..4 {
            let idx = s.push(k * 10_000 + j, EventKind::Damage, a, m, ab, 10, 0, e.0, 0);
            s.extend_encounter(e, idx);
        }
        s.close_encounter(e, k * 10_000 + 9, true, false);
    }
    assert_eq!(s.len(), 20);
    s.evict_before_encounter(2);
    assert_eq!(s.encounters.len(), 3);
    assert_eq!(s.len(), 12);
    // Surviving encounters still index their own events correctly.
    for e in &s.encounters {
        assert_eq!(e.range().len(), 4);
        assert!(e.range().end <= s.len());
    }
    let first = s.encounters[0].id;
    assert_eq!(
        eqlp_store::total(&s, &Filter::encounter(first).damage()),
        40
    );
}

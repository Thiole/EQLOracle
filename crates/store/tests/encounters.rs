//! Encounter open/close lifecycle, including the stale-encounter safety net.

use eqlp_store::{tag, EventKind, Store};

#[test]
fn close_stale_encounters_closes_only_what_is_actually_idle() {
    let mut s = Store::default();
    let mob = s.sym("a gnoll");
    let you = s.sym("You");
    let hit = s.ability_id("Slash", tag::MELEE);

    // Stale: last touched at ts=0, nothing since.
    let stale = s.open_encounter(mob, 0, 0, None);
    let idx = s.push(0, EventKind::Damage, you, mob, hit, 5, 0, stale.0, 0);
    s.extend_encounter(stale, idx);

    // Fresh: last touched at ts=590_000 (just under 10 minutes).
    let fresh = s.open_encounter(mob, 590_000, 1, None);
    let idx = s.push(590_000, EventKind::Damage, you, mob, hit, 5, 0, fresh.0, 0);
    s.extend_encounter(fresh, idx);

    // now = 600_000 (10 minutes): stale is 10min idle, fresh is ~10s idle.
    s.close_stale_encounters(600_000, 5 * 60 * 1000);

    assert_eq!(
        s.encounter(stale).unwrap().end_ms,
        Some(0),
        "closes at its own last touch, not now"
    );
    assert!(
        s.encounter(fresh).unwrap().end_ms.is_none(),
        "still within the idle window, must stay open"
    );
}

#[test]
fn an_already_closed_encounter_is_left_alone() {
    let mut s = Store::default();
    let mob = s.sym("a gnoll");
    let e = s.open_encounter(mob, 0, 0, None);
    s.close_encounter(e, 100, true, false);
    s.close_stale_encounters(1_000_000, 5 * 60 * 1000);
    assert_eq!(
        s.encounter(e).unwrap().end_ms,
        Some(100),
        "real close time must not be overwritten"
    );
}

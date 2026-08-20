//! Tests for `cast`. Kept out of the production module by convention: src/
//! contains shipping code only.

use eqlp_session::cast::{Outcome, Resolver, RESOLUTION_TIMEOUT_MS};

const SRC: u32 = 1;
const SPELL: u32 = 100;

#[test]
fn resist_closes_the_matching_cast() {
    let mut r = Resolver::default();
    r.begin(0, SRC, SPELL);
    r.resolve(500, SRC, SPELL, Outcome::Resisted);
    let out = r.resolved();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].outcome, Outcome::Resisted);
    assert_eq!(out[0].start_ms, 0);
    assert_eq!(out[0].end_ms, 500);
}

#[test]
fn mismatched_spell_does_not_close_it() {
    let mut r = Resolver::default();
    r.begin(0, SRC, SPELL);
    r.resolve(500, SRC, SPELL + 1, Outcome::Resisted);
    assert!(r.resolved().is_empty());
}

#[test]
fn a_second_begin_force_resolves_the_first_as_unconfirmed() {
    let mut r = Resolver::default();
    r.begin(0, SRC, SPELL);
    r.begin(2_000, SRC, SPELL + 1);
    let out = r.resolved();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].outcome, Outcome::Unconfirmed);
    assert_eq!(out[0].spell, SPELL);
}

#[test]
fn expire_force_resolves_stale_casts_only() {
    let mut r = Resolver::default();
    r.begin(0, SRC, SPELL);
    r.expire(RESOLUTION_TIMEOUT_MS);
    assert!(
        r.resolved().is_empty(),
        "exactly at the timeout should not yet expire"
    );
    r.expire(RESOLUTION_TIMEOUT_MS + 1);
    let out = r.resolved();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].outcome, Outcome::Unconfirmed);
}

#[test]
fn landed_confirms_via_damage() {
    let mut r = Resolver::default();
    r.begin(0, SRC, SPELL);
    r.confirm_landed(300, SRC, SPELL);
    assert_eq!(r.resolved()[0].outcome, Outcome::Landed);
}

#[test]
fn drain_empties_resolved_without_disturbing_still_open_casts() {
    let mut r = Resolver::default();
    r.begin(0, SRC, SPELL);
    r.resolve(100, SRC, SPELL, Outcome::Fizzled);
    assert_eq!(r.drain_resolved().len(), 1);
    assert!(r.resolved().is_empty());

    // A cast opened after the drain resolves normally -- draining the
    // finished list must not have touched the open-cast tracking.
    r.begin(200, SRC, SPELL);
    r.resolve(300, SRC, SPELL, Outcome::Landed);
    assert_eq!(r.resolved().len(), 1);
    assert_eq!(r.resolved()[0].outcome, Outcome::Landed);
}

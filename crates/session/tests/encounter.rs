//! Tests for `encounter`. Kept out of the production module by
//! convention: src/ contains shipping code only.

use eqlp_session::encounter::{EndReason, Tracker, Ttk};

#[test]
fn death_closes_and_records_exactly() {
    let mut t = Tracker::default();
    t.damage(0, "You", "a rat", 100);
    t.damage(2000, "You", "a rat", 100);
    t.death(3000, "a rat");
    assert_eq!(t.done.len(), 1);
    let e = &t.done[0];
    assert_eq!(e.end_reason, Some(EndReason::Slain));
    assert_eq!(e.total, 200);
    assert_eq!(e.duration_ms(0), 3000);
}

#[test]
fn silence_closes_by_timeout_at_last_damage_not_now() {
    let mut t = Tracker::new(12_000, 10_000);
    t.damage(0, "You", "a rat", 500);
    t.tick(5_000);
    assert_eq!(t.done.len(), 0, "closed too early");
    t.tick(60_000);
    let e = &t.done[0];
    assert_eq!(e.end_reason, Some(EndReason::Timeout));
    // Duration must not include the silence, or DPS is diluted by a minute
    // of nothing.
    assert_eq!(e.duration_ms(60_000), 0);
}

#[test]
fn ttk_needs_a_baseline_then_predicts() {
    let mut t = Tracker::default();
    for k in 0..3 {
        let b = k * 100_000;
        t.damage(b, "You", "a rat", 1000);
        // Death lines capitalise; this must still close the fight.
        t.death(b + 10_000, "A rat");
    }
    assert_eq!(t.hp.estimate("a rat"), Some(1000));

    let b = 500_000;
    for i in 0..5 {
        t.damage(b + i * 1000, "You", "a rat", 100);
    }
    match t.ttk("a rat", b + 4000) {
        // 500 done of 1000, ~125 dps -> ~4s
        Ttk::Seconds(s) => assert!(s > 1.0 && s < 12.0, "{s}"),
        o => panic!("{o:?}"),
    }
}

#[test]
fn ttk_is_honest_when_it_cannot_know() {
    let mut t = Tracker::default();
    t.damage(0, "You", "a boss", 10);
    assert_eq!(t.ttk("a boss", 1000), Ttk::NoBaseline);
    assert_eq!(t.ttk("never seen", 1000), Ttk::NoBaseline);
}

#[test]
fn timeout_kills_are_excluded_from_the_hp_model() {
    let mut t = Tracker::new(5_000, 10_000);
    // Three real kills at 1000.
    for k in 0..3 {
        let b = k * 100_000;
        t.damage(b, "You", "a rat", 1000);
        t.death(b + 1000, "a rat");
    }
    // A fight that timed out after 50 damage must not drag the estimate.
    t.damage(900_000, "You", "a rat", 50);
    t.tick(950_000);
    assert_eq!(t.hp.estimate("a rat"), Some(1000));
}

#[test]
fn sentence_case_death_closes_the_encounter() {
    let mut t = Tracker::default();
    t.damage(0, "You", "an armadillo", 100);
    t.death(1000, "An armadillo");
    assert_eq!(t.done.len(), 1, "capitalised death line failed to close");
    assert_eq!(t.done[0].end_reason, Some(EndReason::Slain));
    // Display name keeps the form we first saw, not the folded key.
    assert_eq!(t.done[0].target, "an armadillo");
}

#[test]
fn folding_does_not_merge_distinct_names() {
    let mut t = Tracker::default();
    t.damage(0, "You", "a gnoll", 10);
    t.damage(0, "You", "Gnoll Commander", 10);
    assert_eq!(t.open_count(), 2);
}

#[test]
fn per_source_split() {
    let mut t = Tracker::default();
    t.damage(0, "You", "a rat", 300);
    t.damage(0, "Sidhe", "a rat", 100);
    let e = t.get("a rat").unwrap();
    let v = e.dps_by_source(1000);
    assert_eq!(v[0].0, "You");
    assert_eq!(v[0].2, 300);
    assert_eq!(v.len(), 2);
}

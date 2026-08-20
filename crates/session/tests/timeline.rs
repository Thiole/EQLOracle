//! State reconstruction at arbitrary instants, and damage series for plotting.

use eqlp_session::{series, Cause, State, Timeline};

#[test]
fn mez_does_not_end_combat() {
    // A mezzed mob is still in the fight and still on the aggro list. Mez
    // delays actions; it does not remove an entity from the field.
    assert!(State::Mezzed.in_combat());
    assert!(State::Engaged.in_combat());
    assert!(State::Charmed.in_combat());
    assert!(!State::Dead.in_combat());
    assert!(!State::Lost.in_combat());
}

#[test]
fn an_encounter_with_only_a_mezzed_mob_is_still_live() {
    let mut t = Timeline::default();
    t.observed(1000, 7, State::Mezzed);
    assert!(t.any_in_combat(&[7], 5000), "mez must not close the fight");
    t.observed(9000, 7, State::Dead);
    assert!(!t.any_in_combat(&[7], 9000));
}

#[test]
fn state_is_queryable_at_any_instant() {
    let mut t = Timeline::default();
    t.observed(1_000, 7, State::Mezzed);
    t.observed(5_000, 7, State::Engaged);
    t.observed(9_000, 7, State::Dead);

    assert_eq!(t.state_at(7, 0), None, "before any evidence");
    assert_eq!(
        t.state_at(7, 1_000).unwrap().0,
        State::Mezzed,
        "inclusive at the instant"
    );
    assert_eq!(t.state_at(7, 4_999).unwrap().0, State::Mezzed);
    assert_eq!(t.state_at(7, 5_000).unwrap().0, State::Engaged);
    assert_eq!(
        t.state_at(7, 100_000).unwrap().0,
        State::Dead,
        "holds after the last"
    );
}

#[test]
fn scrubbing_backwards_gives_the_same_answer_as_forwards() {
    // The scrub bar must be reversible: dragging left then right must not
    // produce different history. Guaranteed by querying, not by mutating.
    let mut t = Timeline::default();
    t.observed(1_000, 7, State::Mezzed);
    t.observed(5_000, 7, State::Engaged);
    t.observed(9_000, 7, State::Dead);
    let fwd: Vec<_> = (0..12)
        .map(|i| t.state_at(7, i * 1000).map(|x| x.0))
        .collect();
    let back: Vec<_> = (0..12)
        .rev()
        .map(|i| t.state_at(7, i * 1000).map(|x| x.0))
        .collect();
    assert_eq!(fwd, back.into_iter().rev().collect::<Vec<_>>());
}

#[test]
fn out_of_order_arrival_does_not_corrupt_the_order() {
    let mut t = Timeline::default();
    t.observed(9_000, 7, State::Dead);
    t.observed(1_000, 7, State::Mezzed); // late line
    t.observed(5_000, 7, State::Engaged);
    assert_eq!(t.state_at(7, 2_000).unwrap().0, State::Mezzed);
    assert_eq!(t.state_at(7, 6_000).unwrap().0, State::Engaged);
    let ts: Vec<_> = t.transitions_of(7).iter().map(|x| x.ts).collect();
    assert_eq!(ts, [1_000, 5_000, 9_000]);
}

#[test]
fn inferred_transitions_are_marked_as_such() {
    // Memory blur, pacify and lull produce no log line at all. Anything derived
    // from silence must be distinguishable from something the log stated.
    let mut t = Timeline::default();
    t.observed(1_000, 7, State::Engaged);
    t.inferred(20_000, 7, State::Lost);
    assert_eq!(t.state_at(7, 25_000), Some((State::Lost, Cause::Inferred)));
    assert_eq!(
        t.state_at(7, 1_500),
        Some((State::Engaged, Cause::Observed))
    );
}

#[test]
fn snapshot_covers_both_sides_of_a_fight() {
    let mut t = Timeline::default();
    t.observed(1_000, 1, State::Mezzed); // a mob
    t.observed(2_000, 2, State::Dead); // another mob
                                       // entity 3 has no transitions: seen fighting, nothing changed
    let snap = t.snapshot(&[1, 2, 3], 5_000);
    assert_eq!(snap.len(), 3);
    assert_eq!(snap[0].1, State::Mezzed);
    assert_eq!(snap[1].1, State::Dead);
    assert_eq!(snap[2].1, State::Engaged);
    assert_eq!(snap[2].2, Cause::Inferred, "assumed, not observed");
}

#[test]
fn transitions_in_a_window_drive_scrub_bar_markers() {
    let mut t = Timeline::default();
    for i in 0..10 {
        t.observed(i * 1000, i as u32, State::Mezzed);
    }
    assert_eq!(t.between(3_000, 6_000).len(), 4);
    assert_eq!(t.between(0, 0).len(), 1);
    assert_eq!(t.between(50_000, 60_000).len(), 0);
}

#[test]
fn series_emits_empty_buckets_rather_than_skipping_them() {
    // A gap in a fight is information. A series with holes cannot be plotted
    // against a linear time axis.
    let ts = [0i64, 1_000, 9_000];
    let amt = [100u64, 200, 50];
    let s = series(&ts, &amt, 0, 9_000, 1_000);
    assert_eq!(s.len(), 10);
    assert_eq!(s[0].total, 100);
    assert_eq!(s[1].total, 200);
    assert_eq!(s[5].total, 0, "gap must be present, not omitted");
    assert_eq!(s[9].total, 50);
    assert_eq!(s.iter().map(|b| b.total).sum::<u64>(), 350);
}

#[test]
fn series_dps_matches_the_bucket_width() {
    let ts = [0i64, 500];
    let amt = [300u64, 300];
    let s = series(&ts, &amt, 0, 1_000, 1_000);
    assert_eq!(s[0].total, 600);
    assert_eq!(s[0].dps(1_000), 600.0);
    assert_eq!(s[0].events, 2);
}

#[test]
fn series_ignores_events_outside_the_window() {
    let ts = [-5_000i64, 0, 50_000];
    let amt = [999u64, 100, 999];
    let s = series(&ts, &amt, 0, 1_000, 1_000);
    assert_eq!(s.iter().map(|b| b.total).sum::<u64>(), 100);
}

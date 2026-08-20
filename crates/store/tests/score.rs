//! Parse scoring: expected-value baseline vs. an actual selection.

use eqlp_store::{by_ability, score_parse, tag, EventKind, Filter, GearModifiers, Store};

/// Two separate encounters against the same mob with the same ability:
/// one establishes a baseline (mean 300), the other is the "actual" parse
/// being scored against it, deliberately running hot (mean 475).
fn seed() -> (Store, u32, u32) {
    let mut s = Store::default();
    let you = s.sym("You");
    let mob = s.sym("a gnoll");
    let nuke = s.ability_id("Ice Comet", tag::SPELL);

    let base_enc = s.open_encounter(mob, 0, 0, None);
    for (i, amt) in [200u64, 300, 400].into_iter().enumerate() {
        let idx = s.push(
            i as i64 * 1000,
            EventKind::Damage,
            you,
            mob,
            nuke,
            amt,
            0,
            base_enc.0,
            0,
        );
        s.extend_encounter(base_enc, idx);
    }
    s.close_encounter(base_enc, 3000, true, false);

    let actual_enc = s.open_encounter(mob, 10_000, s.len() as u32, None);
    for (i, amt) in [450u64, 500].into_iter().enumerate() {
        let idx = s.push(
            10_000 + i as i64 * 1000,
            EventKind::Damage,
            you,
            mob,
            nuke,
            amt,
            0,
            actual_enc.0,
            0,
        );
        s.extend_encounter(actual_enc, idx);
    }
    s.close_encounter(actual_enc, 12_000, true, false);

    (s, base_enc.0, actual_enc.0)
}

#[test]
fn a_parse_that_runs_hot_scores_above_one() {
    let (s, base_id, actual_id) = seed();
    let baseline = by_ability(
        &s,
        &Filter::encounter(eqlp_store::EncounterId(base_id)).damage(),
    );
    let actual = by_ability(
        &s,
        &Filter::encounter(eqlp_store::EncounterId(actual_id)).damage(),
    );

    let score = score_parse(&baseline, &actual, &GearModifiers::default());
    assert_eq!(score.observed_total, 950);
    assert!(
        (score.expected_total - 600.0).abs() < 1e-9,
        "3-hit baseline mean is 300, x2 hits = 600"
    );
    assert!((score.ratio - 950.0 / 600.0).abs() < 1e-9);
    assert_eq!(score.per_ability.len(), 1);
}

#[test]
fn an_ability_with_no_baseline_is_skipped_not_scored_against_zero() {
    let (mut s, base_id, actual_id) = seed();
    // Backstab only ever appears in the "actual" selection -- no history to
    // compare it against.
    let backstab = s.ability_id("Backstab", tag::MELEE);
    let you = s.sym("You");
    let mob = s.sym("a gnoll");
    let idx = s.push(
        11_000,
        EventKind::Damage,
        you,
        mob,
        backstab,
        999,
        0,
        actual_id,
        0,
    );
    s.extend_encounter(eqlp_store::EncounterId(actual_id), idx);

    let baseline = by_ability(
        &s,
        &Filter::encounter(eqlp_store::EncounterId(base_id)).damage(),
    );
    let actual = by_ability(
        &s,
        &Filter::encounter(eqlp_store::EncounterId(actual_id)).damage(),
    );
    let score = score_parse(&baseline, &actual, &GearModifiers::default());

    // Still only Ice Comet scored -- Backstab's 999 does not appear in
    // observed_total, and does not silently count as infinite overperformance.
    assert_eq!(score.per_ability.len(), 1);
    assert_eq!(score.observed_total, 950);
}

#[test]
fn a_damage_focus_raises_the_expected_baseline() {
    let (s, base_id, actual_id) = seed();
    let baseline = by_ability(
        &s,
        &Filter::encounter(eqlp_store::EncounterId(base_id)).damage(),
    );
    let actual = by_ability(
        &s,
        &Filter::encounter(eqlp_store::EncounterId(actual_id)).damage(),
    );

    let neutral = score_parse(&baseline, &actual, &GearModifiers::default());
    let boosted = score_parse(
        &baseline,
        &actual,
        &GearModifiers {
            damage_focus: 1.5,
            ..GearModifiers::default()
        },
    );

    // Same observed total either way -- only the expectation moves, which
    // is the whole point of the seam: a detected focus should make an
    // otherwise-identical parse look less like overperformance, not change
    // what actually happened.
    assert_eq!(neutral.observed_total, boosted.observed_total);
    assert!(boosted.expected_total > neutral.expected_total);
    assert!(boosted.ratio < neutral.ratio);
}

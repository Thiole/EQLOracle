//! why: adversarial twins for the suite's happy paths -- each test
//! attacks an edge the positive tests skate past: exact boundaries
//! (every `>` vs `>=` is a behavior), same-timestamp ties, out-of-order
//! arrival, conflicting evidence. A failure here is either a real bug
//! or an unpinned semantic; both are findings.

use eqlp_session::{
    Builder, CastOutcome, CastResolver, ClassDetector, GroupTracker, Policy, Rolling, Spans, State,
    Timeline,
};

// ---------------------------------------------------------------- spans

/// why: two enters at the SAME timestamp -- partition_point(<=) places
/// the later arrival after the earlier, so the last write wins for
/// at(). Pinned so a log emitting two zone lines in one second stays
/// deterministic instead of hash-ordered.
#[test]
fn same_timestamp_zone_enters_resolve_to_the_last_arrival() {
    let mut z = Spans::default();
    z.enter(1_000, "A");
    z.enter(1_000, "B");
    assert_eq!(z.at(1_000), Some("B"));
    assert_eq!(z.len(), 2, "both spans exist; B's shadows A's from t=1000");
}

/// why: a LATE line (earlier timestamp arriving after a later one) --
/// the log can carry order variance around zone loads. enter() sorts by
/// timestamp on insert, so lookups must see wall-clock order, not
/// arrival order.
#[test]
fn out_of_order_zone_enter_lands_in_timestamp_order() {
    let mut z = Spans::default();
    z.enter(2_000, "Later");
    z.enter(1_000, "Earlier"); // arrives second, happened first
    assert_eq!(z.at(1_500), Some("Earlier"));
    assert_eq!(z.at(2_500), Some("Later"));
    assert_eq!(z.index_at(1_500), Some(0));
    assert_eq!(z.index_at(2_500), Some(1));
}

// ---------------------------------------------------------------- timeline

/// why: conflicting states at the SAME timestamp (a death line and an
/// action line sharing one log second) -- partition_point(<=) inserts
/// the later arrival after, so the LAST observation wins. Pinned: this
/// is what makes "damage then death at the same second" resolve dead.
#[test]
fn same_timestamp_conflicting_states_resolve_to_the_last_observation() {
    let mut t = Timeline::default();
    t.observed(1_000, 7, State::Engaged);
    t.observed(1_000, 7, State::Dead);
    assert_eq!(t.state_at(7, 1_000).map(|(s, _)| s), Some(State::Dead));

    let mut t2 = Timeline::default();
    t2.observed(1_000, 7, State::Dead);
    t2.observed(1_000, 7, State::Engaged);
    assert_eq!(
        t2.state_at(7, 1_000).map(|(s, _)| s),
        Some(State::Engaged),
        "opposite arrival order must give the opposite answer -- last wins, deterministically"
    );
}

/// why: a late transition older than everything else must not corrupt
/// the ordering the scrub bar depends on
#[test]
fn out_of_order_transition_keeps_state_queries_consistent() {
    let mut t = Timeline::default();
    t.observed(5_000, 1, State::Dead);
    t.observed(1_000, 1, State::Engaged); // late arrival, earlier time
    assert_eq!(t.state_at(1, 2_000).map(|(s, _)| s), Some(State::Engaged));
    assert_eq!(t.state_at(1, 6_000).map(|(s, _)| s), Some(State::Dead));
}

// ---------------------------------------------------------------- rolling

/// why: the window is documented half-open (now-width, now] -- an event
/// at EXACTLY now-width must be evicted, one tick inside must survive.
/// An off-by-one here silently skews every live DPS reading.
#[test]
fn rolling_event_exactly_at_the_window_edge_is_evicted() {
    let mut r = Rolling::new(10_000);
    r.push(0, 100);
    r.push(5_000, 50);
    r.evict(10_000); // cutoff = 0; t=0 satisfies t <= cutoff
    assert_eq!(r.buffered(), 1, "the t=0 event sits ON the edge -- out");
    r.evict(15_000); // cutoff = 5_000; t=5_000 on the edge -- out
    assert_eq!(r.buffered(), 0);
}

// ---------------------------------------------------------------- group

/// why: the session gate is `gap > SESSION_GAP_MS` -- a reinforcement at
/// EXACTLY the gap is the same session, one past it is a new one. The
/// whole MIN_SESSIONS measurement rests on this boundary.
#[test]
fn a_weak_reinforcement_at_exactly_the_session_gap_is_the_same_session() {
    use eqlp_session::group::{GROUP_TTL_MS, MIN_SESSIONS, SESSION_GAP_MS};
    let mut g = GroupTracker::default();
    let mut ts = 0;
    // why: hit exactly-at-gap repeatedly -- must NEVER accumulate sessions
    for _ in 0..MIN_SESSIONS * 2 {
        g.reinforce_weak("Edge", ts);
        ts += SESSION_GAP_MS; // exactly the gap, not past it
    }
    assert!(
        !g.currently_grouped("Edge", ts - SESSION_GAP_MS + GROUP_TTL_MS.min(1)),
        "exactly-at-gap must not count as gap-separated sessions"
    );
}

// ---------------------------------------------------------------- cast

/// why: conflicting resolution -- a terminal line for a DIFFERENT spell
/// than the one open must not close the pending cast (rank collisions
/// aside, a wrong-spell resolve is noise); the real close then comes
/// from expire as Unconfirmed, never as the wrong outcome.
#[test]
fn a_wrong_spell_resolution_leaves_the_cast_open_until_expiry() {
    let mut r = CastResolver::default();
    r.begin(1_000, 1, 100);
    r.resolve(1_500, 1, 999, CastOutcome::Resisted); // different spell key
    assert!(r.resolved().is_empty(), "wrong spell must not close it");
    r.expire(1_000 + 8_000 + 1);
    let out = r.drain_resolved();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].outcome, CastOutcome::Unconfirmed);
}

/// why: two begins at the same timestamp from one caster -- the second
/// force-closes the first as Unconfirmed rather than losing either
#[test]
fn a_same_timestamp_double_begin_force_closes_the_first() {
    let mut r = CastResolver::default();
    r.begin(1_000, 1, 100);
    r.begin(1_000, 1, 200);
    let out = r.drain_resolved();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].spell, 100);
    assert_eq!(out[0].outcome, CastOutcome::Unconfirmed);
}

// ---------------------------------------------------------------- graph

/// why: the death of a name the graph has never seen -- log variance can
/// deliver a stray death line; it must be a no-op, not a panic or a
/// phantom fight
#[test]
fn a_death_for_an_unknown_entity_is_a_quiet_no_op() {
    let mut b = Builder::new(Policy::default());
    b.death(1_000, "never seen");
    assert_eq!(b.live_count(), 0);
    assert!(b.closed.is_empty());
}

/// why: the merge cap is a real conflict point -- two fights whose
/// combined entity count exceeds the cap must NOT merge; the bridging
/// actor joins the target's fight instead, and both fights survive
#[test]
fn a_merge_over_the_entity_cap_attaches_instead_of_merging() {
    let mut b = Builder::new(Policy::default().cap_entities(3));
    b.damage(1_000, "A", "mob one");
    b.damage(1_000, "B", "mob two");
    assert_eq!(b.live_count(), 2);
    // why: bridging edge -- A (fight 1) hits mob two (fight 2); combined
    // entities 4 > cap 3, so no merge
    b.damage(1_100, "A", "mob two");
    assert_eq!(b.live_count(), 2, "cap must prevent the merge");
    assert_eq!(
        b.encounter_of("A"),
        b.encounter_of("mob two"),
        "the actor moves to the target's fight -- target anchors"
    );
}

// ---------------------------------------------------------------- classdetect

/// why: conflicting UNAMBIGUOUS sources -- two different classes each
/// with genuinely unambiguous sightings across enough visits both get
/// proven; the visit they share shows both. That's 2 of the 3 slots
/// gone to a data conflict, and the design accepts it (evidence
/// promotes, never demotes) -- pinned here so it's a documented
/// behavior, not an accident.
#[test]
fn two_conflicting_unambiguous_classes_can_both_confirm() {
    let mut d = ClassDetector::default();
    for v in [0, 1] {
        d.observe_cast(1, Some(v), &["Wizard".to_string()]);
        d.observe_cast(1, Some(v), &["Berserker".to_string()]);
    }
    let cfg = d.configuration_of_visit(1, Some(0));
    assert_eq!(cfg, vec!["Berserker".to_string(), "Wizard".to_string()]);
}

/// why: FOUR proven classes pointing at one visit -- impossible in the
/// game (exactly 3), reachable through data conflicts. The resolved
/// view must not present it as a legitimate full configuration.
#[test]
fn a_visit_with_more_proven_classes_than_the_game_allows_never_reads_as_a_full_config() {
    let mut d = ClassDetector::default();
    for class in ["Wizard", "Enchanter", "Magician", "Necromancer"] {
        for v in [0, 1] {
            d.observe_cast(1, Some(v), &[class.to_string()]);
        }
    }
    let (full, unresolved) = d.visits_by_resolved_configuration(1);
    assert!(
        full.iter().all(|(c, _)| c.len() == 3),
        "no bucket may claim more classes than the game allows, got {full:?}"
    );
    assert!(
        full.iter()
            .all(|(_, vs)| !vs.contains(&Some(0)) && !vs.contains(&Some(1)))
            && unresolved.len() == 2,
        "the 4-class visits must land unresolved, not fabricate a trio"
    );
}

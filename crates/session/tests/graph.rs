//! Encounter graph construction and entity classification.

use eqlp_session::{Builder, Entities, Kind, Policy};

#[test]
fn policy_defaults_to_ten_seconds_and_is_settable() {
    assert_eq!(Policy::default().idle_ms, 10_000);
    assert_eq!(Policy::default().idle_secs(30.0).idle_ms, 30_000);
    assert_eq!(Policy::default().link_secs(2.5).link_ms, 2_500);
    assert_eq!(Policy::default().cap_entities(12).max_entities, Some(12));
    assert!(!Policy::default().no_transitive().transitive);
}

#[test]
fn a_fight_is_one_component_regardless_of_who_swings() {
    let mut b = Builder::default();
    b.damage(0, "You", "a gnoll");
    b.damage(1000, "a gnoll", "You");
    b.damage(2000, "Kaeus", "a gnoll");
    b.damage(3000, "Kaeus pet", "a gnoll");
    assert_eq!(b.live_count(), 1);
    let e = b.live_encounters().next().unwrap();
    assert_eq!(e.entities.len(), 4);
    assert_eq!(e.events, 4);
}

#[test]
fn disjoint_fights_stay_separate() {
    let mut b = Builder::default();
    b.damage(0, "You", "a gnoll");
    b.damage(0, "Stranger", "an orc");
    assert_eq!(b.live_count(), 2);
}

#[test]
fn silence_closes_after_the_configured_idle() {
    let mut b = Builder::new(Policy::default().idle_secs(10.0).idle_unresolved_secs(10.0));
    b.damage(0, "You", "a gnoll");
    b.expire(9_000);
    assert_eq!(b.live_count(), 1, "closed before idle elapsed");
    b.expire(11_000);
    assert_eq!(b.live_count(), 0);
    assert_eq!(b.closed.len(), 1);
    // Duration must not include the silence.
    assert_eq!(b.closed[0].duration_ms(), 0);
}

#[test]
fn a_longer_idle_keeps_a_lull_in_one_encounter() {
    let run = |idle: f64| {
        let mut b = Builder::new(Policy::default().idle_secs(idle).idle_unresolved_secs(idle));
        b.damage(0, "You", "a gnoll");
        b.damage(20_000, "You", "a gnoll");
        b.close_all(30_000);
        b.closed.len()
    };
    assert_eq!(run(10.0), 2, "10s must split a 20s lull");
    assert_eq!(run(30.0), 1, "30s must not");
}

#[test]
fn a_multi_mob_pull_is_one_encounter_and_survives_a_death() {
    let mut b = Builder::default();
    b.damage(0, "You", "gnoll A");
    b.damage(0, "gnoll B", "You");
    assert_eq!(b.live_count(), 1, "both mobs linked through me");
    b.death(1000, "gnoll A");
    assert_eq!(
        b.live_count(),
        1,
        "one death must not end a multi-mob fight"
    );
    b.damage(2000, "You", "gnoll B");
    b.close_all(3000);
    let c = &b.closed[0];
    assert_eq!(c.slain, ["gnoll A"]);
    assert!(c.entities.len() >= 3);
}

#[test]
fn an_interrupted_fight_links_to_its_predecessor() {
    let mut b = Builder::new(
        Policy::default()
            .idle_secs(10.0)
            .idle_unresolved_secs(10.0)
            .link_secs(60.0),
    );
    b.entities.note_player_channel("You");
    b.damage(0, "You", "a gnoll");
    b.expire(11_000); // mob fled; encounter closes, nothing slain
    b.damage(20_000, "You", "a gnoll");
    b.death(21_000, "a gnoll");
    b.close_all(40_000);
    assert_eq!(
        b.closed.len(),
        2,
        "still two encounters -- DPS windows stay separate"
    );
    assert_eq!(
        b.closed[1].links_to,
        Some(b.closed[0].id),
        "but they are one kill"
    );
}

#[test]
fn a_slain_target_does_not_link_forward() {
    let mut b = Builder::default();
    b.entities.note_player_channel("You");
    b.damage(0, "You", "a gnoll");
    b.death(1000, "a gnoll");
    b.expire(20_000);
    b.damage(30_000, "You", "a gnoll"); // a different gnoll, same name
    b.close_all(45_000);
    assert_eq!(b.closed[1].links_to, None, "a corpse cannot be re-engaged");
}

/// The player is in every fight, so linking through one would chain an entire
/// session into a single encounter.
#[test]
fn consecutive_fights_do_not_link_through_the_player() {
    let mut b = Builder::new(
        Policy::default()
            .idle_secs(10.0)
            .idle_unresolved_secs(10.0)
            .link_secs(60.0),
    );
    b.entities.note_player_channel("You");
    b.damage(0, "You", "gnoll A");
    b.death(1000, "gnoll A");
    b.expire(20_000);
    b.damage(25_000, "You", "gnoll B");
    b.death(26_000, "gnoll B");
    b.close_all(40_000);
    assert_eq!(b.closed.len(), 2);
    assert_eq!(
        b.closed[1].links_to, None,
        "two separate kills must not link"
    );
}

#[test]
fn the_entity_cap_stops_runaway_merging() {
    let mut b = Builder::new(Policy::default().cap_entities(3));
    b.damage(0, "A", "m1");
    b.damage(0, "B", "m2");
    b.damage(1000, "A", "m2"); // would merge to 4 entities; cap forbids it
    assert_eq!(b.live_count(), 2);
}

#[test]
fn transitive_merging_can_be_disabled() {
    let mut on = Builder::default();
    on.damage(0, "A", "m1");
    on.damage(0, "B", "m2");
    on.damage(1000, "A", "m2");
    assert_eq!(on.live_count(), 1);

    let mut off = Builder::new(Policy::default().no_transitive());
    off.damage(0, "A", "m1");
    off.damage(0, "B", "m2");
    off.damage(1000, "A", "m2");
    assert_eq!(off.live_count(), 2);
}

#[test]
fn merged_encounters_are_flagged() {
    let mut b = Builder::default();
    b.damage(0, "A", "m1");
    b.damage(0, "B", "m2");
    b.damage(1000, "A", "m2");
    b.close_all(20_000);
    assert!(b.closed.iter().any(|c| c.merged));
}

/// why: the merged-away side must still get a Closed record of its own,
/// not sit orphaned forever -- see graph.rs's `merge` doc.
#[test]
fn the_merged_away_side_gets_its_own_closed_record_too() {
    let mut b = Builder::default();
    b.damage(0, "A", "m1"); // opens id 0
    b.damage(0, "B", "m2"); // opens id 1
    b.damage(1000, "A", "m2"); // merges id 1 into id 0
                               // The merge itself must have already produced a Closed record for the
                               // losing id -- not waiting on close_all/expire, which a merged-away id
                               // (removed from `live`) would never reach.
    assert_eq!(
        b.closed.len(),
        1,
        "merge should close the losing side immediately"
    );
    let orphan = &b.closed[0];
    assert_eq!(orphan.start_ms, 0);
    assert_eq!(
        orphan.end_ms, 0,
        "closes at its own last touch, not merge time"
    );
    assert!(orphan.entities.iter().any(|e| e == "B") && orphan.entities.iter().any(|e| e == "m2"));

    b.close_all(20_000);
    assert_eq!(
        b.closed.len(),
        2,
        "the surviving merged fight closes separately"
    );
}

// ---- entity classification ----

#[test]
fn a_players_pet_names_its_owner_and_damage_credits_them() {
    // A player's pet is possessive: "<Owner>'s pet" (or this log's
    // backtick-as-apostrophe stand-in, "<Owner>`s pet").
    let mut e = Entities::default();
    assert_eq!(e.observe("Kaeus's pet"), Kind::Pet);
    assert_eq!(e.owner_of("Kaeus's pet"), Some("Kaeus"));
    assert_eq!(e.credit("Kaeus's pet"), "Kaeus");

    assert_eq!(e.observe("Manipulator`s pet"), Kind::Pet);
    assert_eq!(e.owner_of("Manipulator`s pet"), Some("Manipulator"));
}

#[test]
fn a_bare_pet_suffix_with_no_possessive_is_a_mobs_own_pet_not_a_players() {
    // "Gynok Moltor pet" is a real boss's own summoned add in the
    // reference log (bare, no possessive) -- not a player's pet. An
    // earlier version treated *any* " pet" suffix as ownership proof and
    // read this as Kind::Pet, putting the boss's own add on the ally side
    // of Allegiance::of.
    let mut e = Entities::default();
    assert_eq!(e.observe("Gynok Moltor pet"), Kind::Unproven);
    assert_eq!(e.owner_of("Gynok Moltor pet"), None);
    assert_eq!(e.credit("Gynok Moltor pet"), "Gynok Moltor pet");
}

#[test]
fn a_charmed_mob_is_not_a_pet_and_credits_nobody() {
    // The log gives charmed mobs no ownership marker of any kind.
    let mut e = Entities::default();
    assert_eq!(e.observe("an abhorrent"), Kind::Unproven);
    assert_eq!(e.owner_of("an abhorrent"), None);
    assert_eq!(e.credit("an abhorrent"), "an abhorrent");
}

#[test]
fn only_a_player_channel_proves_a_player() {
    // Named NPCs are indistinguishable from players by name alone -- 'Ktik'
    // and 'Zobartik' look exactly like character names.
    let mut e = Entities::default();
    e.observe("Ktik");
    e.observe("Dippinsauce");
    assert_eq!(e.kind("Ktik"), Kind::Unproven);
    assert_eq!(e.kind("Dippinsauce"), Kind::Unproven);

    e.note_player_channel("Dippinsauce");
    assert_eq!(e.kind("Dippinsauce"), Kind::Player);
    assert_eq!(e.kind("Ktik"), Kind::Unproven);
    assert_eq!(e.players().collect::<Vec<_>>(), ["Dippinsauce"]);
}

#[test]
fn classification_is_monotonic() {
    let mut e = Entities::default();
    e.note_player_channel("Kaeus");
    e.observe("Kaeus");
    assert_eq!(e.kind("Kaeus"), Kind::Player, "observe must not demote");
}

/// why: the two-tier idle contract (Policy's own docs) -- a fight where
/// something DIED goes quiet because it's over (short window, keeps the
/// next pull separate); a fight with zero kills yet goes quiet because
/// of mezz/fled/medding (long window, a pause is not an end). The
/// real-log measurement behind it: examples/reset_check.rs.
#[test]
fn a_no_kill_lull_outlasts_the_short_idle_but_a_kill_closes_fast() {
    // no kill: still live past the short window, closes after the long one
    let mut b = Builder::new(Policy::default().idle_secs(10.0).idle_unresolved_secs(60.0));
    b.damage(0, "You", "a gnoll");
    b.expire(30_000);
    assert_eq!(
        b.live_count(),
        1,
        "no kill yet -- 30s of mezz quiet must not close it"
    );
    b.expire(61_000);
    assert_eq!(
        b.live_count(),
        0,
        "past the unresolved window it really is over"
    );

    // kill: the short window applies
    let mut b = Builder::new(Policy::default().idle_secs(10.0).idle_unresolved_secs(60.0));
    b.damage(0, "You", "a gnoll");
    b.death(1_000, "a gnoll");
    b.expire(12_000);
    assert_eq!(b.live_count(), 0, "concluded pull closes on the short idle");
}

/// why: "a pet dying isn't a kill" -- a mob's own pet death must not
/// arm the fast post-kill idle window; the fight stays on the patient
/// unresolved clock until something real dies
#[test]
fn a_pet_death_does_not_resolve_the_fight() {
    let mut b = Builder::new(Policy::default().idle_secs(10.0).idle_unresolved_secs(60.0));
    b.damage(0, "You", "a dracoliche");
    b.damage(500, "a dracoliche pet", "You");
    b.death(1_000, "a dracoliche pet");
    b.expire(30_000);
    assert_eq!(
        b.live_count(),
        1,
        "pet death alone -- still the long window"
    );
    b.death(31_000, "a dracoliche");
    b.expire(45_000);
    assert_eq!(b.live_count(), 0, "the real death arms the short close");
}

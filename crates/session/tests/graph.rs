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
    let mut b = Builder::new(Policy::default().idle_secs(10.0));
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
        let mut b = Builder::new(Policy::default().idle_secs(idle));
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
    assert_eq!(b.live_count(), 1, "one death must not end a multi-mob fight");
    b.damage(2000, "You", "gnoll B");
    b.close_all(3000);
    let c = &b.closed[0];
    assert_eq!(c.slain, ["gnoll A"]);
    assert!(c.entities.len() >= 3);
}

#[test]
fn an_interrupted_fight_links_to_its_predecessor() {
    let mut b = Builder::new(Policy::default().idle_secs(10.0).link_secs(60.0));
    b.entities.note_player_channel("You");
    b.damage(0, "You", "a gnoll");
    b.expire(11_000);            // mob fled; encounter closes, nothing slain
    b.damage(20_000, "You", "a gnoll");
    b.death(21_000, "a gnoll");
    b.close_all(40_000);
    assert_eq!(b.closed.len(), 2, "still two encounters -- DPS windows stay separate");
    assert_eq!(b.closed[1].links_to, Some(b.closed[0].id), "but they are one kill");
}

#[test]
fn a_slain_target_does_not_link_forward() {
    let mut b = Builder::default();
    b.entities.note_player_channel("You");
    b.damage(0, "You", "a gnoll");
    b.death(1000, "a gnoll");
    b.expire(20_000);
    b.damage(30_000, "You", "a gnoll");   // a different gnoll, same name
    b.close_all(45_000);
    assert_eq!(b.closed[1].links_to, None, "a corpse cannot be re-engaged");
}

/// The player is in every fight, so linking through one would chain an entire
/// session into a single encounter.
#[test]
fn consecutive_fights_do_not_link_through_the_player() {
    let mut b = Builder::new(Policy::default().idle_secs(10.0).link_secs(60.0));
    b.entities.note_player_channel("You");
    b.damage(0, "You", "gnoll A");
    b.death(1000, "gnoll A");
    b.expire(20_000);
    b.damage(25_000, "You", "gnoll B");
    b.death(26_000, "gnoll B");
    b.close_all(40_000);
    assert_eq!(b.closed.len(), 2);
    assert_eq!(b.closed[1].links_to, None, "two separate kills must not link");
}

#[test]
fn the_entity_cap_stops_runaway_merging() {
    let mut b = Builder::new(Policy::default().cap_entities(3));
    b.damage(0, "A", "m1");
    b.damage(0, "B", "m2");
    b.damage(1000, "A", "m2");   // would merge to 4 entities; cap forbids it
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

// ---- entity classification ----

#[test]
fn pets_name_their_owner_and_damage_credits_them() {
    let mut e = Entities::default();
    assert_eq!(e.observe("Gynok Moltor pet"), Kind::Pet);
    assert_eq!(e.owner_of("Gynok Moltor pet"), Some("Gynok Moltor"));
    assert_eq!(e.credit("Gynok Moltor pet"), "Gynok Moltor");
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

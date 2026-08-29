//! why: overlay's Drop Watch widget -- "hey, you're fighting something
//! that might drop what you're after". Doesn't list everything you're
//! in combat with, only a mob that's (a) a real currently-open enemy
//! encounter and (b) `monsterdata` knows drops *something*. Which of
//! its drops are actually worth surfacing is a player-selected list
//! (`Preferences::tracked_drop_items`) applied client-side, same split
//! as `combat::class_configurations`/`skilltracker` -- this returns
//! every known drop for a matching mob, unfiltered, and the frontend
//! intersects with what's actually tracked (see stores/settings.ts).
//!
//! "Currently fighting" deliberately skips `monsters::counts_as_pull`'s
//! own personal-damage-or-XP bar: mid-fight there's no XP yet, and a
//! group/raid target a teammate opened is still worth a heads-up here --
//! this is a plain notice, not a credit/scoring mechanism.
//!
//! State checked as of *now* (`ing.now_ms()`), not the encounter's own
//! `start_ms` -- `counts_as_pull`'s at-start check is right for scoring
//! a past pull, wrong here: a target charmed or slain mid-fight must
//! drop off this live list immediately, not read as its allegiance back
//! when the fight opened. Real gap found writing this: the store's own
//! `Encounter::is_open()` lags a confirmed kill (it only closes on the
//! session graph's own idle-timeout expiry, see `Entities::death`'s
//! doc) -- so `State::Dead` is checked directly too, same as
//! `target_effects`'s own doc on why `Allegiance::of` alone doesn't
//! special-case a dead Unproven mob.
//!
//! `loot_status` is the other half of the same feature: once a tracked
//! item is actually looted, the frontend prompts to remove it from the
//! list rather than silently dropping it -- see `TrackedLootDto`'s own doc.

use crate::ingest::Ingest;
use eqlp_session::{Allegiance, State};
use eqlp_source::Millis;
use eqlp_store::EventKind;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DropWatchRowDto {
    pub mob: String,
    /// why: this mob's full known drop list, not pre-filtered to what's
    /// tracked -- same "give everything, frontend intersects" split
    /// `get_skill_status` already uses
    pub drops: Vec<String>,
}

/// why: player's own real ask -- once a tracked item is actually looted
/// (or auto-routed to currency/tradeskill storage -- both are the same
/// real `EventKind::Loot` row, `loot.self.direct`'s own trailing clause
/// is discarded past the item/qty, see its own doc) the frontend prompts
/// "remove this from Drop Watch?" with a timer; no answer means no
/// change, still tracked. Only fields needed for that: how many, and
/// when most recently -- `count`/`last_looted_ms` let the frontend tell
/// "you already had some" from "you just got another one" by diffing
/// against its own last-seen baseline (`monsters::item_loot_history`
/// gives the full event list instead, right for a one-time item-page
/// open, wrong for a poll called every tick against several names --
/// its own doc says as much).
#[derive(Debug, Clone, Serialize)]
pub struct TrackedLootDto {
    pub item: String,
    pub count: u64,
    pub last_looted_ms: Millis,
}

/// why: one store pass for every requested name at once, not one call
/// per name -- see TrackedLootDto's own doc for why. Case-insensitive,
/// matching `item_loot_history`'s own comparison; a name with no loot at
/// all is simply absent from the result, not a zeroed row.
pub fn loot_status(ing: &Ingest, items: &[String]) -> Vec<TrackedLootDto> {
    let mut by_key: std::collections::HashMap<String, (u64, Millis)> =
        std::collections::HashMap::new();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Loot {
            continue;
        }
        let name = ing.store.ability_name(ing.store.ability[i]);
        let key = name.to_ascii_lowercase();
        let entry = by_key.entry(key).or_insert((0, 0));
        entry.0 += ing.store.amount[i];
        entry.1 = entry.1.max(ing.store.ts[i]);
    }
    items
        .iter()
        .filter_map(|it| {
            by_key
                .get(&it.to_ascii_lowercase())
                .map(|&(count, last_looted_ms)| TrackedLootDto {
                    item: it.clone(),
                    count,
                    last_looted_ms,
                })
        })
        .collect()
}

/// why: how long a swing that never landed still counts as "actively
/// engaged" -- matches the graph's own 10s idle window (Policy::idle_ms
/// default), so a swings-only engagement goes stale on the same clock a
/// landed-damage fight would
const ENGAGED_MISS_WINDOW_MS: Millis = 10_000;

/// why: one row per living enemy ENTITY actively engaged right now --
/// two sources, both real gaps the player reported in turn:
/// 1. every entity in any currently-live graph fight (not one row per
///    store-encounter anchor -- a mob JOINING an open pull is exactly
///    when the heads-up matters);
/// 2. either side of a recent Miss row -- fight-graph membership only
///    comes from LANDED damage, so a mob trading swings that all miss
///    (or one you keep whiffing at) is genuinely engaged yet in no live
///    fight at all ("it shouldnt have to hit back, just be an active
///    member of the current engagement", the player's own spec).
///
/// Empty drop lists (a real mob the wiki records no drops for) are
/// skipped, nothing to show regardless of tracking.
pub fn drop_watch(ing: &Ingest) -> Vec<DropWatchRowDto> {
    let now = ing.now_ms();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();

    // why: the active zone's own zone-wide drop pool -- player's spec:
    // "a mobs drops should be its individual drops + zone drops"; an
    // item any mob in the zone can drop alerts on every engaged mob
    // while that zone is active. Same label->wiki-zone matcher
    // resolved_wiki_zone uses; 117 zones, one linear find per poll.
    let zone_items: &[String] = ing
        .zone
        .at(now)
        .and_then(|raw| {
            crate::zonedata::zones()
                .iter()
                .find(|z| crate::zone::zone_matches(raw, &z.name))
        })
        .map(|z| z.unique_items.as_slice())
        .unwrap_or(&[]);

    // why: shared filter for both sources -- one place decides what a
    // watchable engaged enemy is; `slain_in_fight` is only meaningful
    // for the graph source (a Miss row has no fight-scoped slain list)
    let mut consider = |name: &str, slain_in_fight: bool| {
        if name.eq_ignore_ascii_case("you") || slain_in_fight {
            return None;
        }
        // why: read-only sym lookup, same shape is_ally uses -- a
        // drop-watch query must never itself intern an identity
        let state = ing
            .store
            .names
            .get(ing.encounters.entities.display_name(name))
            .and_then(|sym| ing.timeline.state_at(sym.0, now))
            .map(|(s, _)| s)
            .unwrap_or(State::Engaged);
        if state == State::Dead || state == State::Charmed {
            return None;
        }
        // why: effective_kind not raw kind -- a group-tracked ally
        // (or charm-flip, caught above) must not read as a mob to watch
        let kind = ing.effective_kind(name, now);
        if !Allegiance::of(kind, state).is_enemy() {
            return None;
        }
        // why: three drop sources unioned per the player's spec --
        // (1) monsters.json drop tables, (2) npcs.json known_loot (a
        // DIFFERENT wiki scrape; measured: 2 of the player's 3 tracked
        // items had their only dropper attribution there), (3) the
        // active zone's own unique_items pool applied to every engaged
        // mob. Deduped case-insensitively, mob-specific first.
        let mut drops: Vec<String> = crate::monsterdata::known_drops(name).to_vec();
        for d in crate::npcdata::known_loot_for(name)
            .iter()
            .chain(zone_items.iter())
        {
            if !drops.iter().any(|x| x.eq_ignore_ascii_case(d)) {
                drops.push(d.clone());
            }
        }
        if drops.is_empty() {
            return None;
        }
        // why: dedupe at PUSH time, not at loop entry -- real bug in
        // the first cut: a slain instance in one fight inserted the
        // name before its own slain-skip, shadowing a genuinely live
        // same-named mob in another fight out of the list entirely
        if seen.insert(name.to_ascii_lowercase()) {
            Some(DropWatchRowDto {
                mob: name.to_string(),
                drops,
            })
        } else {
            None
        }
    };

    for live in ing.encounters.live_encounters() {
        for name in &live.entities {
            let slain = live
                .slain
                .iter()
                .any(|s| s.eq_ignore_ascii_case(name.as_str()));
            if let Some(row) = consider(name, slain) {
                out.push(row);
            }
        }
    }

    // why: source 2 -- recent swings that never landed. Both sides of
    // the Miss row: the mob missing "You" AND the mob "You" keep
    // whiffing at are each engaged. partition_point over the store's
    // time column, same suffix-scan shape overview.rs uses.
    let cutoff = now - ENGAGED_MISS_WINDOW_MS;
    let from = ing.store.ts.partition_point(|&t| t < cutoff);
    for i in from..ing.store.len() {
        if ing.store.kind[i] != EventKind::Miss {
            continue;
        }
        for sym in [ing.store.actor[i], ing.store.target[i]] {
            let name = ing.store.name(sym).to_string();
            if let Some(row) = consider(&name, false) {
                out.push(row);
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{backfill_lines, framed_lines};
    use crate::parser::build_engine;

    fn run(log: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let bytes = log.as_bytes();
        let lines = framed_lines(bytes);
        let mut ing = Ingest::default();
        backfill_lines(&mut ing, &engine, &lines, lines.len());
        ing
    }

    /// why: a real known monster, still-open fight -- must show up with
    /// its real wiki drop list
    #[test]
    fn an_open_fight_against_a_known_monster_lists_its_full_drop_table() {
        let ing =
            run("[Tue Jul 28 15:01:00 2026] You hit Keeper of Souls for 5 points of damage.\n");
        let rows = drop_watch(&ing);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].mob, "Keeper of Souls");
        assert!(
            rows[0].drops.iter().any(|d| d == "Light Woolen Mantle"),
            "real wiki drop, got {:?}",
            rows[0].drops
        );
    }

    /// why: the real reported gap this rewrite fixes -- a known monster
    /// JOINING an already-open fight (never the anchor) must still get
    /// its heads-up row the moment it's in the fight
    #[test]
    fn a_known_monster_joining_an_open_fight_shows_up_without_being_the_anchor() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.\n",
            "[Tue Jul 28 15:01:02 2026] Keeper of Souls hits YOU for 2 points of damage.\n",
        ));
        let rows = drop_watch(&ing);
        assert!(
            rows.iter().any(|r| r.mob == "Keeper of Souls"),
            "joined mid-fight, never the anchor -- must still show, got {:?}",
            rows.iter().map(|r| &r.mob).collect::<Vec<_>>()
        );
    }

    /// why: the real reported bug -- farming the same-named mob: kill
    /// one, engage the next. Timeline state is per-NAME, so the old
    /// kill's Dead used to stick to the new mob until it happened to
    /// act; being the target of live damage must count as proof of life
    #[test]
    fn a_reengaged_same_named_mob_shows_up_before_it_ever_swings_back() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] You hit Keeper of Souls for 50 points of damage.\n",
            "[Tue Jul 28 15:01:05 2026] You have slain Keeper of Souls!\n",
            "[Tue Jul 28 15:02:05 2026] You hit Keeper of Souls for 50 points of damage.\n",
        ));
        let rows = drop_watch(&ing);
        assert!(
            rows.iter().any(|r| r.mob == "Keeper of Souls"),
            "re-engaged farm mob must show without needing to act first, got {:?}",
            rows.iter().map(|r| &r.mob).collect::<Vec<_>>()
        );
    }

    /// why: the player's own spec -- "it shouldnt have to hit back, but
    /// just be an active member in the current engagement". A mob whose
    /// every swing misses (and that "You" never landed on) is in no
    /// graph fight at all, yet it's genuinely engaged
    #[test]
    fn a_mob_only_trading_misses_still_shows_up() {
        let ing =
            run("[Tue Jul 28 15:01:00 2026] Keeper of Souls tries to punch YOU, but YOU dodge!\n");
        let rows = drop_watch(&ing);
        assert!(
            rows.iter().any(|r| r.mob == "Keeper of Souls"),
            "swings-only engagement must still show, got {:?}",
            rows.iter().map(|r| &r.mob).collect::<Vec<_>>()
        );
    }

    /// why: a swing from ages ago isn't a live engagement -- the miss
    /// source goes stale on the same 10s clock a real fight idles out on
    #[test]
    fn a_stale_missed_swing_does_not_keep_a_mob_on_the_list() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] Keeper of Souls tries to punch YOU, but YOU dodge!\n",
            // why: a much later unrelated line advances the log clock well past the window
            "[Tue Jul 28 15:05:00 2026] You hit a rat for 5 points of damage.\n",
        ));
        let rows = drop_watch(&ing);
        assert!(
            !rows.iter().any(|r| r.mob == "Keeper of Souls"),
            "a 4-minute-old whiff is not an active engagement"
        );
    }

    /// why: 2 of the player's 3 real tracked items (Blood Sky Ruby,
    /// Golden Coffer) have their ONLY dropper attribution in npcs.json's
    /// known_loot, not monsters.json -- verified by replaying the
    /// reference log with the old single-source lookup: those two could
    /// never alert at all
    #[test]
    fn an_npc_catalog_only_drop_still_alerts() {
        let ing =
            run("[Tue Jul 28 15:01:00 2026] You hit Eye of Veeshan for 5 points of damage.\n");
        let rows = drop_watch(&ing);
        let eye = rows.iter().find(|r| r.mob == "Eye of Veeshan");
        assert!(
            eye.is_some_and(|r| r.drops.iter().any(|d| d == "Blood Sky Ruby")),
            "npcs.json known_loot must union in, got {:?}",
            rows.iter().map(|r| &r.mob).collect::<Vec<_>>()
        );
    }

    /// why: player's spec -- "a mobs drops should be its individual
    /// drops + zone drops": a zone-unique item alerts on ANY engaged
    /// mob while that zone is active
    #[test]
    fn a_zone_unique_item_attaches_to_every_engaged_mob_in_that_zone() {
        let ing = run(concat!(
            "[Tue Jul 28 15:00:00 2026] You have entered Skyshrine.\n",
            "[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.\n",
        ));
        let rows = drop_watch(&ing);
        let rat = rows.iter().find(|r| r.mob == "a rat");
        assert!(
            rat.is_some_and(|r| r.drops.iter().any(|d| d == "Brightwood Spear")),
            "Skyshrine's own unique_items must attach to any engaged mob there, got {:?}",
            rows
        );
    }

    /// why: a mob already slain inside a still-live fight is old news
    #[test]
    fn a_slain_entity_in_a_still_live_fight_is_excluded() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.\n",
            "[Tue Jul 28 15:01:02 2026] Keeper of Souls hits YOU for 2 points of damage.\n",
            "[Tue Jul 28 15:01:04 2026] Keeper of Souls has been slain by You!\n",
        ));
        let rows = drop_watch(&ing);
        assert!(
            !rows.iter().any(|r| r.mob == "Keeper of Souls"),
            "slain mid-fight -- must drop off the watch list"
        );
    }

    /// why: a closed (already-resolved) fight is old news, not a live "you're fighting this" signal
    #[test]
    fn a_closed_encounter_never_shows_up() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] You hit Keeper of Souls for 5 points of damage.\n",
            "[Tue Jul 28 15:01:05 2026] Keeper of Souls has been slain by You!\n",
        ));
        assert!(drop_watch(&ing).is_empty());
    }

    /// why: a real mob with no recorded wiki drops at all contributes
    /// nothing -- an empty row would be a heads-up about nothing
    #[test]
    fn a_mob_with_no_known_drops_is_skipped() {
        let ing = run("[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.\n");
        assert!(drop_watch(&ing).is_empty());
    }

    /// why: a charmed target is a temporary ally, not an enemy to warn about
    #[test]
    fn a_charmed_target_is_excluded() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] You hit Keeper of Souls for 5 points of damage.\n",
            "[Tue Jul 28 15:01:05 2026] Keeper of Souls has been charmed.\n",
        ));
        assert!(drop_watch(&ing).is_empty());
    }

    /// why: real shape -- a loot line, count and latest timestamp both real
    #[test]
    fn a_looted_tracked_item_reports_its_real_count_and_timestamp() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] --You have looted a Light Woolen Mask from a coyote's corpse.--\n",
            "[Tue Jul 28 15:05:00 2026] --You have looted a Light Woolen Mask from a coyote's corpse.--\n",
        ));
        let rows = loot_status(&ing, &["Light Woolen Mask".to_string()]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[0].last_looted_ms, ing.store.ts[ing.store.len() - 1]);
    }

    /// why: auto-routed to storage is still a real Loot row -- "pickup or
    /// storage" are the same signal, not two things to detect separately
    #[test]
    fn an_auto_stored_item_still_counts_as_looted() {
        let ing = run(
            "[Tue Jul 28 15:01:00 2026] You looted a Mote of Infinitesimal Potential from a dune spiderling's corpse and stored it in your currency\n",
        );
        let rows = loot_status(&ing, &["Mote of Infinitesimal Potential".to_string()]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 1);
    }

    /// why: absent, not a zeroed row -- a name with no loot at all has
    /// nothing to diff a baseline against
    #[test]
    fn an_untracked_or_never_looted_item_is_simply_absent() {
        let ing = run("[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.\n");
        assert!(loot_status(&ing, &["Light Woolen Mask".to_string()]).is_empty());
    }

    /// why: one store pass covers every requested name, not just the first
    #[test]
    fn multiple_tracked_items_are_all_reported_from_one_pass() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] --You have looted a Light Woolen Mask from a coyote's corpse.--\n",
            "[Tue Jul 28 15:02:00 2026] --You have looted an Amulet of Woven Hair from a coyote's corpse.--\n",
        ));
        let rows = loot_status(
            &ing,
            &[
                "Light Woolen Mask".to_string(),
                "Amulet of Woven Hair".to_string(),
            ],
        );
        assert_eq!(rows.len(), 2);
    }
}

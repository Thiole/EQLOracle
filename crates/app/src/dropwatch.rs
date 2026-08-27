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

/// why: one row per currently-open enemy encounter `monsterdata` has any
/// drop data for -- empty rows (a real mob wiki drops never recorded)
/// are skipped, nothing to show for those regardless of tracking
pub fn drop_watch(ing: &Ingest) -> Vec<DropWatchRowDto> {
    let now = ing.now_ms();
    ing.store
        .encounters
        .iter()
        .filter(|e| e.is_open())
        .filter_map(|e| {
            let name = ing.store.name(e.target);
            let state = ing
                .timeline
                .state_at(e.target.0, now)
                .map(|(s, _)| s)
                .unwrap_or(State::Engaged);
            if state == State::Dead || state == State::Charmed {
                return None;
            }
            let kind = ing.encounters.entities.kind(name);
            if !Allegiance::of(kind, state).is_enemy() {
                return None;
            }
            let drops = crate::monsterdata::known_drops(name);
            if drops.is_empty() {
                return None;
            }
            Some(DropWatchRowDto {
                mob: name.to_string(),
                drops: drops.to_vec(),
            })
        })
        .collect()
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
        let ing = run("[Tue Jul 28 15:01:00 2026] You hit Keeper of Souls for 5 points of damage.\n");
        let rows = drop_watch(&ing);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].mob, "Keeper of Souls");
        assert!(
            rows[0].drops.iter().any(|d| d == "Light Woolen Mantle"),
            "real wiki drop, got {:?}",
            rows[0].drops
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

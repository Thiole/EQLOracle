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
//! a past pull, wrong here: a charm-flip must drop off this live list
//! immediately, not read as its allegiance back when the fight opened.
//! Death and fight-end do NOT clear immediately -- player's spec: "it
//! should stay open 15-30 seconds after ENCOUNTER ends, not instantly
//! clear"; the row is a loot reminder and the corpse gets looted right
//! after the kill, so a dead mob holds its row for `CLEAR_GRACE_MS`
//! from the death line, and a fight that ends without a kill (fled,
//! disengaged) holds via the graph's closed list for the same grace
//! past its `end_ms`.
//!
//! `loot_status` is the other half of the same feature: once a tracked
//! item is actually looted, the frontend prompts to remove it from the
//! list rather than silently dropping it -- see `TrackedLootDto`'s own doc.

use crate::ingest::Ingest;
use eqlp_session::State;
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

/// why: one INCREMENTAL pass for every requested name at once --
/// measured inefficiency, full-app walk 2026-08-29: the old version
/// rescanned the entire store (~1.9M rows on the reference log) on
/// every parse-tick whenever anything was tracked. The store is
/// append-only within one Ingest lifetime, so a checkpointed fold
/// (Ingest::loot_scan_next_row/loot_scan_counts) answers identically
/// at O(rows since last call). Case-insensitive, matching
/// `item_loot_history`'s own comparison; a name with no loot at all is
/// simply absent from the result, not a zeroed row.
pub fn loot_status(ing: &mut Ingest, items: &[String]) -> Vec<TrackedLootDto> {
    for i in ing.loot_scan_next_row..ing.store.len() {
        if ing.store.kind[i] != EventKind::Loot {
            continue;
        }
        // why: tier-folded -- a tracked wiki name is untiered, the log
        // loots "+N" instances; both sides fold to the base item
        let key = crate::inventory::strip_tier(ing.store.ability_name(ing.store.ability[i]))
            .0
            .to_ascii_lowercase();
        let amount = ing.store.amount[i];
        let ts = ing.store.ts[i];
        let entry = ing.loot_scan_counts.entry(key).or_insert((0, 0));
        entry.0 += amount;
        entry.1 = entry.1.max(ts);
    }
    ing.loot_scan_next_row = ing.store.len();
    items
        .iter()
        .filter_map(|it| {
            ing.loot_scan_counts
                .get(&crate::inventory::strip_tier(it).0.to_ascii_lowercase())
                .map(|&(count, last_looted_ms)| TrackedLootDto {
                    item: it.clone(),
                    count,
                    last_looted_ms,
                })
        })
        .collect()
}

/// why: player's spec -- "stay open 15-30 seconds after ENCOUNTER
/// ends"; rows are loot reminders, corpses get looted after the kill.
/// Runs from the death line / fight end_ms / last whiffed swing, so
/// every source's row lives ~30s past its fight's last real event
const CLEAR_GRACE_MS: Millis = 30_000;

/// why: one row per enemy ENTITY engaged now or in a just-ended fight --
/// three sources, all real gaps/specs the player reported in turn:
/// 1. every entity in any currently-live graph fight (not one row per
///    store-encounter anchor -- a mob JOINING an open pull is exactly
///    when the heads-up matters);
/// 2. every entity in a fight that ended within CLEAR_GRACE_MS -- the
///    loot-window linger, see the const's own doc;
/// 3. either side of a recent Miss row -- fight-graph membership only
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
    // why: second pool -- union of the zone's own NPCs' known_loot. The
    // zonedata pool reads the zone page's header table, which whole
    // zones lack (Plane of Sky: empty, yet every island boss's loot is
    // attributed per NPC) -- see zone_loot_pool's own doc
    let npc_zone_items: &[String] = ing
        .zone
        .at(now)
        .map(crate::npcdata::zone_loot_pool)
        .unwrap_or(&[]);

    // why: shared filter for all three sources -- one place decides
    // what a watchable enemy is
    let mut consider = |name: &str| {
        if name.eq_ignore_ascii_case("you") {
            return None;
        }
        // why: read-only sym lookup, same shape is_ally uses -- a
        // drop-watch query must never itself intern an identity
        let since = ing
            .store
            .names
            .get(ing.encounters.entities.display_name(name))
            .and_then(|sym| ing.timeline.state_since(sym.0, now));
        if let Some((state, _, since_ts)) = since {
            if state == State::Charmed {
                return None;
            }
            // why: death starts the loot grace, doesn't clear -- the
            // per-name death timestamp also handles a mob slain inside
            // a still-live fight (long AE pull) going stale on its own
            // clock, not the fight's
            if state == State::Dead && now - since_ts > CLEAR_GRACE_MS {
                return None;
            }
        }
        // why: allegiance_at not raw kind -- a group-tracked ally
        // (or charm-flip, caught above) must not read as a mob to watch
        if !ing.allegiance_at(name, now).is_enemy() {
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
            .chain(npc_zone_items.iter())
        {
            if !drops.iter().any(|x| x.eq_ignore_ascii_case(d)) {
                drops.push(d.clone());
            }
        }
        if drops.is_empty() {
            return None;
        }
        // why: dedupe by name across sources -- a mob in a live fight
        // AND a just-closed one (re-engage) is one row, not two
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
            if let Some(row) = consider(name) {
                out.push(row);
            }
        }
    }

    // why: source 2 -- fights ended within the grace. `closed` is in
    // close order and end_ms trails close time by at most the idle
    // window, so the rev take_while bound carries that slack before
    // the exact filter; fled survivors read Lost (drain_closed marks
    // them), which still classifies enemy.
    let slack = CLEAR_GRACE_MS + ing.encounters.policy.idle_ms;
    for closed in ing
        .encounters
        .closed
        .iter()
        .rev()
        .take_while(|e| now - e.end_ms <= slack)
        .filter(|e| now - e.end_ms <= CLEAR_GRACE_MS)
    {
        for name in &closed.entities {
            if let Some(row) = consider(name) {
                out.push(row);
            }
        }
    }

    // why: source 3 -- recent swings that never landed. Both sides of
    // the Miss row: the mob missing "You" AND the mob "You" keep
    // whiffing at are each engaged. The window is the same grace clock
    // the other sources run on, from the last swing. partition_point
    // over the store's time column, same suffix-scan shape overview.rs
    // uses.
    let cutoff = now - CLEAR_GRACE_MS;
    let from = ing.store.ts.partition_point(|&t| t < cutoff);
    for i in from..ing.store.len() {
        if ing.store.kind[i] != EventKind::Miss {
            continue;
        }
        for sym in [ing.store.actor[i], ing.store.target[i]] {
            let name = ing.store.name(sym).to_string();
            if let Some(row) = consider(&name) {
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
    /// source goes stale on the same grace clock the other sources use
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

    /// why: the real reported gap -- "still see no tracked drops nearby,
    /// after killing hand of veeshan". Plane of Sky's zonedata
    /// unique_items is EMPTY (its wiki page keeps loot in body sections
    /// the zone scraper's header-table parse never sees), so the only
    /// zone pool was blank; the NPC-attributed pool must cover it: an
    /// item only attributed to The Spiroc Lord attaches to any engaged
    /// Sky mob
    #[test]
    fn a_zone_with_no_header_table_pool_still_attaches_its_npcs_loot() {
        let ing = run(concat!(
            "[Tue Jul 28 15:00:00 2026] You have entered The Plane of Sky.\n",
            "[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.\n",
        ));
        let rows = drop_watch(&ing);
        let rat = rows.iter().find(|r| r.mob == "a rat");
        assert!(
            rat.is_some_and(|r| r.drops.iter().any(|d| d == "Golden Coffer")),
            "Spiroc Lord's own attribution must reach every engaged Sky mob via the npc pool, got {:?}",
            rows.iter().map(|r| &r.mob).collect::<Vec<_>>()
        );
    }

    /// why: player's spec -- "stay open 15-30 seconds after ENCOUNTER
    /// ends, not instantly clear": the corpse gets looted right after
    /// the kill, the row is the reminder
    #[test]
    fn a_freshly_slain_mob_lingers_through_the_loot_grace() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] You hit Keeper of Souls for 5 points of damage.\n",
            "[Tue Jul 28 15:01:05 2026] Keeper of Souls has been slain by You!\n",
        ));
        let rows = drop_watch(&ing);
        assert!(
            rows.iter().any(|r| r.mob == "Keeper of Souls"),
            "just slain -- must hold through the loot grace, got {:?}",
            rows.iter().map(|r| &r.mob).collect::<Vec<_>>()
        );
    }

    /// why: the grace is a window, not forever -- past it the kill is
    /// old news even inside a fight that never closed. The per-name
    /// death timestamp, not the fight's clock, decides
    #[test]
    fn a_long_dead_mob_clears_once_the_grace_expires() {
        let ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] You hit Keeper of Souls for 5 points of damage.\n",
            "[Tue Jul 28 15:01:05 2026] Keeper of Souls has been slain by You!\n",
            "[Tue Jul 28 15:02:00 2026] You hit a rat for 5 points of damage.\n",
        ));
        let rows = drop_watch(&ing);
        assert!(
            !rows.iter().any(|r| r.mob == "Keeper of Souls"),
            "dead 55s -- grace over, must clear"
        );
    }

    /// why: a fight ending WITHOUT a kill (fled, disengaged) lingers
    /// the same grace via the graph's closed list, then clears --
    /// expire only runs off tick (see monsters.rs's own doc), so the
    /// tick is explicit here
    #[test]
    fn an_idled_out_fight_lingers_then_clears() {
        let mut ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] You hit Keeper of Souls for 5 points of damage.\n",
            "[Tue Jul 28 15:01:20 2026] You hit a rat for 5 points of damage.\n",
        ));
        ing.tick(0);
        let rows = drop_watch(&ing);
        assert!(
            rows.iter().any(|r| r.mob == "Keeper of Souls"),
            "fight ended 20s ago -- within grace, got {:?}",
            rows.iter().map(|r| &r.mob).collect::<Vec<_>>()
        );

        let mut ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] You hit Keeper of Souls for 5 points of damage.\n",
            "[Tue Jul 28 15:02:00 2026] You hit a rat for 5 points of damage.\n",
        ));
        ing.tick(0);
        assert!(
            drop_watch(&ing).is_empty(),
            "fight ended 60s ago -- past grace, must clear"
        );
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
        let mut ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] --You have looted a Light Woolen Mask from a coyote's corpse.--\n",
            "[Tue Jul 28 15:05:00 2026] --You have looted a Light Woolen Mask from a coyote's corpse.--\n",
        ));
        let rows = loot_status(&mut ing, &["Light Woolen Mask".to_string()]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 2);
        assert_eq!(rows[0].last_looted_ms, ing.store.ts[ing.store.len() - 1]);
    }

    /// why: auto-routed to storage is still a real Loot row -- "pickup or
    /// storage" are the same signal, not two things to detect separately
    #[test]
    fn an_auto_stored_item_still_counts_as_looted() {
        let mut ing = run(
            "[Tue Jul 28 15:01:00 2026] You looted a Mote of Infinitesimal Potential from a dune spiderling's corpse and stored it in your currency\n",
        );
        let rows = loot_status(&mut ing, &["Mote of Infinitesimal Potential".to_string()]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].count, 1);
    }

    /// why: absent, not a zeroed row -- a name with no loot at all has
    /// nothing to diff a baseline against
    #[test]
    fn an_untracked_or_never_looted_item_is_simply_absent() {
        let mut ing = run("[Tue Jul 28 15:01:00 2026] You hit a rat for 5 points of damage.\n");
        assert!(loot_status(&mut ing, &["Light Woolen Mask".to_string()]).is_empty());
    }

    /// why: one store pass covers every requested name, not just the first
    #[test]
    fn multiple_tracked_items_are_all_reported_from_one_pass() {
        let mut ing = run(concat!(
            "[Tue Jul 28 15:01:00 2026] --You have looted a Light Woolen Mask from a coyote's corpse.--\n",
            "[Tue Jul 28 15:02:00 2026] --You have looted an Amulet of Woven Hair from a coyote's corpse.--\n",
        ));
        let rows = loot_status(
            &mut ing,
            &[
                "Light Woolen Mask".to_string(),
                "Amulet of Woven Hair".to_string(),
            ],
        );
        assert_eq!(rows.len(), 2);
    }
}

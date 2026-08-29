//! why: Monsters module queries -- mob types fought, kills, loot so far
//!
//! Grouped by mob name, not individual Encounter -- "what does this mob
//! type drop" not "what did this one pull drop". Reads loot's own
//! `target` field, never `Store::enc`, stays correct regardless of
//! per-pull attribution confidence.

use crate::ingest::Ingest;

use eqlp_source::Millis;
use eqlp_store::{by_target_and_ability, total, EventKind, Filter, Sym, NO_ENCOUNTER};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct LootRowDto {
    pub item: String,
    /// why: stack sizes summed in, not a line count; 0 for a not-yet-gotten known drop
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MobDto {
    pub name: String,
    /// why: confirmed death lines only -- a Reset isn't evidence either way
    pub kills: u64,
    /// why: every encounter, kills and resets alike
    pub pulls: u64,
    /// why: whether `monsterdata` recognizes this mob -- whether `loot` is
    /// the complete wiki-known list or just what's actually been looted
    pub known: bool,
    /// why: gotten-first by count, then alphabetically
    pub loot: Vec<LootRowDto>,
    /// why: mean over kills with a matched Xp row; None if none do --
    /// unmatched isn't safe to average in as 0%
    pub avg_xp_pct: Option<f64>,
}

/// why: whether `e` counts as a real pull, shared by `list_mobs`/`mob_stats`.
/// Two checks: (1) allegiance as of this fight's own start, not "is this
/// name ever an ally" -- charm makes allegiance per-fight, not per-name;
/// `Ingest::allegiance_at`, the same composition `is_ally` uses.
/// (2) personal damage OR `xp_credited` -- fixes a real gap: a raid
/// boss's death line only names the killing blow, so a party member's
/// kill with zero personal damage rows used to read as "not your kill"
/// despite real party XP credit for it. `xp_credited` only ever adds
/// encounters, never subtracts what personal damage already caught.
/// Precomputed once via `xp_credited_encounters`, not per-encounter --
/// avoids the O(encounters * store length) cost that caused a real slowdown.
pub(crate) fn counts_as_pull(
    ing: &Ingest,
    e: &eqlp_store::Encounter,
    you: Sym,
    xp_credited: &std::collections::HashSet<u32>,
) -> bool {
    let name = ing.store.name(e.target);
    if !ing.allegiance_at(name, e.start_ms).is_enemy() {
        return false;
    }
    total(&ing.store, &Filter::encounter(e.id).damage().by(you)) != 0
        || xp_credited.contains(&e.id.0)
}

/// why: every EncounterId that earned "You" any XP, one pass, shared
/// across `counts_as_pull` calls; NO_ENCOUNTER (quest turn-ins) excluded
pub(crate) fn xp_credited_encounters(ing: &Ingest) -> std::collections::HashSet<u32> {
    (0..ing.store.len())
        .filter(|&i| ing.store.kind[i] == EventKind::Xp && ing.store.enc[i] != NO_ENCOUNTER)
        .map(|i| ing.store.enc[i])
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct MobStatsDto {
    pub kills: u64,
    pub pulls: u64,
}

/// why: `list_mobs`' counting scoped to one mob -- skips the loot pass
/// entirely; still pays for one full-store pass via `xp_credited_encounters`,
/// but on-demand per NPC page, not a per-tick refresh
pub fn mob_stats(ing: &Ingest, name: &str) -> MobStatsDto {
    let Some(you) = ing.store.names.get("You") else {
        return MobStatsDto { kills: 0, pulls: 0 };
    };
    let xp_credited = xp_credited_encounters(ing);
    let mut kills = 0u64;
    let mut pulls = 0u64;
    for e in &ing.store.encounters {
        if !ing.store.name(e.target).eq_ignore_ascii_case(name)
            || !counts_as_pull(ing, e, you, &xp_credited)
        {
            continue;
        }
        pulls += 1;
        if e.slain {
            kills += 1;
        }
    }
    MobStatsDto { kills, pulls }
}

/// why: single pass grouping Xp rows by mob (same shape as loot's
/// grouping, avoids O(mobs * store length)); builds encounter->target
/// once since `enc` names an encounter, not a mob directly
fn xp_by_mob(ing: &Ingest) -> HashMap<Sym, (u64, u64)> {
    let target_of: HashMap<u32, Sym> = ing
        .store
        .encounters
        .iter()
        .map(|e| (e.id.0, e.target))
        .collect();
    let mut out: HashMap<Sym, (u64, u64)> = HashMap::new();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Xp {
            continue;
        }
        let enc = ing.store.enc[i];
        if enc == NO_ENCOUNTER {
            continue;
        }
        if let Some(&target) = target_of.get(&enc) {
            let e = out.entry(target).or_insert((0, 0));
            e.0 += ing.store.amount[i];
            e.1 += 1;
        }
    }
    out
}

/// why: every mob type fought, by kills descending then name for stable
/// UI order. Passes over the store O(store length), not O(mobs * store
/// length) -- an earlier per-mob-call version was the real, measured
/// cause of the app crawling on real data.
pub fn list_mobs(ing: &Ingest) -> Vec<MobDto> {
    // why: None before anything's been parsed -- empty store yields empty list
    let Some(you) = ing.store.names.get("You") else {
        return Vec::new();
    };
    let xp_credited = xp_credited_encounters(ing);

    let mut counts: HashMap<Sym, (u64, u64)> = HashMap::new(); // (pulls, kills)
    for e in &ing.store.encounters {
        if !counts_as_pull(ing, e, you, &xp_credited) {
            continue;
        }
        let c = counts.entry(e.target).or_insert((0, 0));
        c.0 += 1;
        if e.slain {
            c.1 += 1;
        }
    }

    let mut loot_by_mob = by_target_and_ability(&ing.store, EventKind::Loot);
    let xp_by_mob = xp_by_mob(ing);

    let mut out: Vec<MobDto> = counts
        .into_iter()
        .map(|(sym, (pulls, kills))| {
            let name = ing.store.name(sym).to_string();
            let avg_xp_pct = xp_by_mob
                .get(&sym)
                .filter(|&&(_, count)| count > 0)
                .map(|&(sum_milli, count)| sum_milli as f64 / 1000.0 / count as f64);
            // why: tier-folded to the base item, tiers summed -- so a
            // "+4" loot lands on the wiki's own untiered row instead of
            // spawning a duplicate "+4" row beside a zero-count known one
            let mut gotten: HashMap<String, u64> = HashMap::new();
            for r in loot_by_mob.remove(&sym).unwrap_or_default() {
                let (base, _tier) = crate::inventory::strip_tier(ing.store.ability_name(r.ability));
                *gotten.entry(base.to_string()).or_insert(0) += r.total;
            }

            let known_items = crate::monsterdata::known_drops(&name);
            let known = crate::monsterdata::is_known_monster(&name);
            let mut loot: Vec<LootRowDto> = if known {
                // why: every wiki-known drop plus anything actually looted
                // that the wiki scrape missed, not silently dropped
                let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
                let mut rows: Vec<LootRowDto> = known_items
                    .iter()
                    .map(|item| {
                        seen.insert(item.as_str());
                        LootRowDto {
                            item: item.clone(),
                            count: gotten.get(item).copied().unwrap_or(0),
                        }
                    })
                    .collect();
                for (item, &count) in &gotten {
                    if !seen.contains(item.as_str()) {
                        rows.push(LootRowDto {
                            item: item.clone(),
                            count,
                        });
                    }
                }
                rows
            } else {
                // why: no wiki drop table -- only what's actually been looted
                gotten
                    .into_iter()
                    .map(|(item, count)| LootRowDto { item, count })
                    .collect()
            };
            loot.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.item.cmp(&b.item)));
            MobDto {
                name,
                kills,
                pulls,
                known,
                loot,
                avg_xp_pct,
            }
        })
        .collect();
    out.sort_by(|a, b| b.kills.cmp(&a.kills).then_with(|| a.name.cmp(&b.name)));
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct LootEventDto {
    pub ts_ms: Millis,
    pub mob: String,
    pub qty: u64,
    /// why: zone active at this moment; None for loot before first zone line
    pub zone: Option<String>,
}

/// why: every real loot event, oldest first, for an item page's history --
/// individual events, not `list_mobs`' aggregate. Linear scan, runs once
/// on page open not a 3s poll, so no O(store length) discipline needed.
/// Tier-folded: the log loots "Fine Steel Rapier +4", the wiki page is
/// the untiered base -- compared unnormalized this returned nothing for
/// most real loot (same bug raiding.rs already fixed via strip_tier).
pub fn item_loot_history(ing: &Ingest, item: &str) -> Vec<LootEventDto> {
    let base_query = crate::inventory::strip_tier(item).0;
    let mut out = Vec::new();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Loot {
            continue;
        }
        let (base, _tier) =
            crate::inventory::strip_tier(ing.store.ability_name(ing.store.ability[i]));
        if !base.eq_ignore_ascii_case(base_query) {
            continue;
        }
        let ts = ing.store.ts[i];
        out.push(LootEventDto {
            ts_ms: ts,
            mob: ing.store.name(ing.store.target[i]).to_string(),
            qty: ing.store.amount[i],
            zone: ing.zone.at(ts).map(str::to_string),
        });
    }
    out
}

#[cfg(test)]
mod pull_credit_tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;

    fn run(text: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        // why: `expire`/`drain_closed` only run off `Ingest::tick`'s own
        // wall-clock argument (see `tail_worker.rs`'s post-backfill
        // `ing.mark_live(); ing.tick(now)`), never automatically during
        // `backfill_lines` itself -- a plain replay with no explicit tick
        // leaves every fight open (and `Store::Encounter::slain` false)
        // forever, even with trailing idle time baked into the fixture's
        // own timestamps. Two ticks, not one: `tick` only *advances* the
        // log clock off the *second* wall-clock reading it sees after
        // `mark_live` (the first just sets the baseline -- see `tick`'s
        // own doc on not double-counting elapsed wall time) -- 60s
        // (comfortably past the 10s `idle_ms` threshold) on the second
        // call stands in for "long past every fight in this fixture", the
        // same role a real wall-clock `now` plays live.
        ing.mark_live();
        ing.tick(0);
        ing.tick(60_000);
        ing
    }

    /// why: real reference-log lines -- a raid miniboss died to a
    /// party-mate's hit, zero personal damage rows, but real party XP
    /// credit -- see `counts_as_pull`'s doc
    // why: trailing line 11s after death is load-bearing -- a fight only
    // closes (slain flips true) after idle_ms of silence past it
    const PARTY_CREDITED_KILL_NO_PERSONAL_DAMAGE: &str = "\
[Fri Aug 14 21:34:52 2026] You are not currently assigned to an adventure.
[Fri Aug 14 21:34:52 2026] Thantoas slashes Fright for 118 points of damage. (Critical)
[Fri Aug 14 21:34:53 2026] You begin casting Conflagration X.
[Fri Aug 14 21:34:53 2026] Xenofaul hit Fright for 2788 points of magic damage by Rend. (Critical)
[Fri Aug 14 21:34:53 2026] You gain party experience! (0.542%)
[Fri Aug 14 21:34:53 2026] Fright has been slain by Xenofaul!
[Fri Aug 14 21:35:04 2026] You are not currently assigned to an adventure.
";

    #[test]
    fn a_party_credited_kill_counts_even_with_zero_personal_damage() {
        let ing = run(PARTY_CREDITED_KILL_NO_PERSONAL_DAMAGE);
        let stats = mob_stats(&ing, "Fright");
        assert_eq!(
            stats.kills, 1,
            "party XP credit alone should be enough to count this as your kill"
        );
        assert_eq!(stats.pulls, 1);

        let mobs = list_mobs(&ing);
        let fright = mobs.iter().find(|m| m.name.eq_ignore_ascii_case("Fright"));
        assert!(
            fright.is_some(),
            "list_mobs should surface Fright too, not just mob_stats"
        );
        assert_eq!(fright.unwrap().kills, 1);
    }

    /// why: personal-damage bar still works alone -- not a strictly looser check
    #[test]
    fn a_fight_with_no_personal_damage_and_no_xp_credit_still_does_not_count() {
        let text = "\
[Fri Aug 14 21:34:52 2026] You are not currently assigned to an adventure.
[Fri Aug 14 21:34:52 2026] Thantoas slashes Fright for 118 points of damage. (Critical)
[Fri Aug 14 21:34:53 2026] Fright has been slain by Thantoas!
[Fri Aug 14 21:35:04 2026] You are not currently assigned to an adventure.
";
        let ing = run(text);
        let stats = mob_stats(&ing, "Fright");
        assert_eq!(
            stats.kills, 0,
            "no personal damage and no party XP credit -- not the player's kill"
        );
        assert_eq!(stats.pulls, 0);
    }

    /// why: the log loots tiered instances ("Rusty Mace +2"), the wiki
    /// item page is the untiered base -- history must attribute every
    /// tier to the base item (same strip_tier fix raiding.rs already
    /// carries), and querying by a tiered name folds the same way
    #[test]
    fn tiered_loot_attributes_to_the_base_items_history() {
        let text = "\
[Tue Jul 28 15:02:50 2026] You hit a patrolling gnoll for 5 points of damage.
[Tue Jul 28 15:02:52 2026] You have slain a patrolling gnoll!
[Tue Jul 28 15:02:55 2026] You looted a Rusty Mace +2 from a patrolling gnoll's corpse to create a Rusty Mace +3
[Tue Jul 28 15:03:10 2026] You hit a patrolling gnoll for 5 points of damage.
[Tue Jul 28 15:03:12 2026] You have slain a patrolling gnoll!
[Tue Jul 28 15:03:15 2026] --You have looted a Rusty Mace from a patrolling gnoll's corpse.--";
        let ing = run(text);
        let events = item_loot_history(&ing, "Rusty Mace");
        assert_eq!(events.len(), 2, "both the +2 and the untiered loot");
        assert!(events.iter().all(|e| e.mob == "a patrolling gnoll"));
        // why: a tiered query folds identically -- one item, one history
        assert_eq!(item_loot_history(&ing, "Rusty Mace +2").len(), 2);
        assert!(item_loot_history(&ing, "Fine Steel Rapier").is_empty());
    }
}

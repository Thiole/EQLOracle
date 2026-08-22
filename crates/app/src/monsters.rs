//! Read-side queries over `Ingest` for the Monsters module: which mob types
//! have been fought, how many times each has been confirmed killed, and
//! what's been looted from them so far this session.
//!
//! Grouped by mob *name*, not by individual `Encounter` -- a mob type gets
//! pulled many times across a session, and "what does a fire giant warrior
//! drop" is the question this module answers, not "what did this one pull
//! drop". Every function here reads a loot row's own `target` field (set
//! from the corpse name in the log line itself, independent of whatever
//! encounter it got attributed to -- see `Ingest::record_loot`'s doc),
//! never `Store::enc`, so it stays correct regardless of how confident
//! that per-pull attribution is for any given row.

use crate::ingest::Ingest;
use eqlp_session::{Allegiance, State};
use eqlp_source::Millis;
use eqlp_store::{by_target_and_ability, total, EventKind, Filter, Sym, NO_ENCOUNTER};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct LootRowDto {
    pub item: String,
    /// Total quantity of this item looted from this mob type so far --
    /// stack sizes summed in, not just a count of loot lines (a "You have
    /// looted 2 Bone Chips..." line counts as 2, not 1). `0` for a row that
    /// exists only because `known` mobs list every wiki-known possible
    /// drop, gotten or not -- see `MobDto::known`.
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MobDto {
    pub name: String,
    /// Encounters against this mob type that ended in a confirmed death
    /// line. See `docs/design/timeline.md` / `record_history`'s doc for why
    /// a `Reset` is not evidence the mob lived -- it just isn't evidence it
    /// died, so it's excluded here rather than counted either way.
    pub kills: u64,
    /// Every encounter against this mob type, kills and resets alike.
    pub pulls: u64,
    /// Whether `crate::monsterdata` recognises this mob at all -- i.e.
    /// whether `loot` below is the *complete* wiki-known drop list (gotten
    /// and not-yet-gotten items alike) or just whatever's actually been
    /// looted so far, because the wiki scrape never recorded a drop table
    /// for this mob. See `monsterdata`'s module doc for why absence here
    /// isn't "not a real monster".
    pub known: bool,
    /// Sorted gotten-first (by count descending), then alphabetically --
    /// so a `known` mob's not-yet-looted items naturally fall to the
    /// bottom rather than needing a separate section.
    pub loot: Vec<LootRowDto>,
    /// Average `%` XP per kill, over only the kills that have a matched
    /// `EventKind::Xp` row -- `None` if none do (no kills yet, or every
    /// kill's gain was a group-credit line this log never reports; see
    /// `EventKind::Xp`'s own doc for what "matched" means here). Not
    /// diluted by treating an unmatched kill as a literal 0%: "no XP
    /// happened" and "XP happened but couldn't be linked to this kill"
    /// are different unknowns, and only the first is safe to average in
    /// as zero.
    pub avg_xp_pct: Option<f64>,
}

/// Whether `e` counts as a real pull against its own target, for
/// kill/pull purposes -- shared by `list_mobs` (grouped, every mob at
/// once) and `mob_stats` (scoped to one mob already picked). Two checks,
/// both load-bearing:
///
/// Defence in depth against `Ingest::link` mis-anchoring an encounter on
/// a real player or pet -- see `link`'s doc comment for the fix
/// (retargets an encounter once a later edge proves a better anchor than
/// whatever its first edge guessed) and its known remaining gap.
/// Filtering here too means this stays honest even for encounters opened
/// before that fix took effect, or for the unspoken-ally case it still
/// can't catch at ingest time but a later chat line proves by the time
/// this runs. Checked as of *this fight's own start*, not "is this name
/// ever an ally anywhere" -- identity alone isn't allegiance for a fight
/// that happened while charmed. A raid boss temporarily charming a proven
/// ally, or a player's own charmed mob later reading as a permanent ally
/// everywhere, are the same mistake in opposite directions: allegiance is
/// a per-fight question, not a property of a name. `Ingest::is_ally`
/// makes the identical check at ingest time (`link`'s doc comment); this
/// mirrors it rather than calling it directly, since that one is private
/// to `Ingest` and needs a `Millis` `link` already has in hand for free.
///
/// Second: not a fight "you" were any part of -- some other group's pull
/// sharing the same zone, or a name (proven ally or not) taking damage
/// nowhere near the player. `record_history` already applies this same
/// "did I deal damage here" bar for the same reason (see its doc
/// comment); this module is "monsters/mobs I've fought", not "everything
/// logged nearby".
///
/// That second bar used to be *only* "did You personally land a logged
/// hit" -- real, reported gap: a raid boss's own death line only ever
/// names whoever struck the killing blow ("Fright has been slain by
/// Xenofaul!"), and confirmed against the real log around exactly that
/// line, the player's own attacks simply hadn't landed a matched damage
/// row in that window at all, despite plainly being grouped on the kill
/// (a "You gain party experience!" line, same second, for that same
/// encounter) -- personal damage alone said "not your kill" about a kill
/// the game itself was actively crediting the player for. `xp_credited`
/// (a kill this encounter's own `EncounterId` earned *any* XP for "You",
/// solo/group/party/raid scope alike -- see `Ingest::record_xp`'s own
/// doc: it's always self-directed, You->You, regardless of who actually
/// dealt the log's own damage) is the fix: real party/group/raid credit
/// is exactly the "were you part of this" signal a personal-damage-only
/// check was trying to approximate, not a looser bar than it -- it only
/// ever *adds* encounters that XP-credited "You" and personal damage
/// alone had missed, never subtracts the ones personal damage already
/// caught. Precomputed once by `xp_credited_encounters` and passed in
/// rather than queried per encounter here, for the same reason
/// `by_target_and_ability` groups loot in one pass elsewhere in this
/// module: a linear scan of the full store per encounter would be
/// exactly the O(encounters * store length) cost this module's own doc
/// already flags as the real, measured cause of a past slowdown.
pub(crate) fn counts_as_pull(ing: &Ingest, e: &eqlp_store::Encounter, you: Sym, xp_credited: &std::collections::HashSet<u32>) -> bool {
    let name = ing.store.name(e.target);
    let kind = ing.encounters.entities.kind(name);
    let state = ing
        .timeline
        .state_at(e.target.0, e.start_ms)
        .map(|(s, _)| s)
        .unwrap_or(State::Engaged);
    if !Allegiance::of(kind, state).is_enemy() {
        return false;
    }
    total(&ing.store, &Filter::encounter(e.id).damage().by(you)) != 0 || xp_credited.contains(&e.id.0)
}

/// Every real `EncounterId` (as its raw `u32`) that earned "You" any XP
/// at all -- one pass over `EventKind::Xp` rows, built once per query and
/// shared by every `counts_as_pull` call it feeds, not re-derived per
/// encounter. `NO_ENCOUNTER`-linked rows (quest turn-in XP -- see
/// `Ingest::record_xp`'s own doc) are correctly excluded: there's no kill
/// to credit for those.
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

/// `list_mobs`' own kills/pulls counting, scoped to one mob name instead
/// of grouping every mob type at once -- an NPC page's "your history"
/// section needs exactly this and nothing else, so it skips `list_mobs`'
/// loot-breakdown pass entirely (a full `Store` scan over
/// `EventKind::Loot`, see `by_target_and_ability`) rather than paying for
/// a table it wouldn't use. Still pays for one full-store pass of its own
/// now, via `xp_credited_encounters` (`counts_as_pull`'s own doc) --
/// unavoidable once party/raid credit counts as evidence too, but this is
/// an on-demand, one-NPC-page-at-a-time call, not a per-tick refresh, so
/// it doesn't reintroduce the O(mobs * store length) cost that doc warns
/// against.
pub fn mob_stats(ing: &Ingest, name: &str) -> MobStatsDto {
    let Some(you) = ing.store.names.get("You") else {
        return MobStatsDto { kills: 0, pulls: 0 };
    };
    let xp_credited = xp_credited_encounters(ing);
    let mut kills = 0u64;
    let mut pulls = 0u64;
    for e in &ing.store.encounters {
        if !ing.store.name(e.target).eq_ignore_ascii_case(name) || !counts_as_pull(ing, e, you, &xp_credited) {
            continue;
        }
        pulls += 1;
        if e.slain {
            kills += 1;
        }
    }
    MobStatsDto { kills, pulls }
}

/// Every `EventKind::Xp` row, grouped by which mob type its (best-effort)
/// `enc` attributes it to -- `(sum of milli-percent, count)` per `Sym`,
/// the same single-pass-over-many-mobs shape `by_target_and_ability`
/// already gives loot, and for the identical reason (see `list_mobs`'s
/// own doc): a per-mob scan here would reintroduce the exact O(mobs *
/// store length) cost that loot's own grouping was built to avoid.
///
/// An `Xp` row's `enc` names an *encounter*, not a mob directly, so this
/// builds encounter-id -> target-`Sym` once up front (a few thousand
/// entries at most, `Store::encounters` itself) rather than re-deriving
/// it per row.
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

/// Every mob type fought so far, sorted by kill count descending (ties
/// broken by name, for a stable order the UI doesn't reshuffle on every
/// refresh when two mobs are tied).
///
/// Two passes over the store, not one per mob: `by_target_and_ability`
/// groups every loot row by target in a single scan, so this stays
/// O(store length) regardless of how many distinct mob types have been
/// fought. An earlier version called `by_ability` once per mob (each call
/// itself an O(store length) scan filtered down after the fact), which was
/// O(mobs * store length) -- fine on a short session, but on a real log
/// with hundreds of mob types this refreshes every 3s from the UI and was
/// the actual cause of the app grinding to a crawl on real data. `xp_by_
/// mob` (below) is a third such single pass, for the same reason.
pub fn list_mobs(ing: &Ingest) -> Vec<MobDto> {
    // `None` before "You" has ever dealt or received a line the store had
    // to name -- i.e. nothing has been parsed yet. Every real fight in this
    // list requires the player to have actually landed damage *or* earned
    // real party/group/raid XP credit for it (see `counts_as_pull`'s own
    // doc), so an empty store correctly yields an empty list rather than
    // panicking on a lookup with nothing to find.
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
            let gotten: HashMap<String, u64> = loot_by_mob
                .remove(&sym)
                .unwrap_or_default()
                .into_iter()
                .map(|r| (ing.store.ability_name(r.ability).to_string(), r.total))
                .collect();

            let known_items = crate::monsterdata::known_drops(&name);
            let known = crate::monsterdata::is_known_monster(&name);
            let mut loot: Vec<LootRowDto> = if known {
                // Every wiki-known possible drop, `count: 0` if never
                // gotten -- plus anything actually looted that the wiki
                // scrape didn't list (a real drop it missed, or one added
                // to the game since the scrape ran), shown rather than
                // silently dropped just because it's not in the reference
                // data.
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
                // Not a mob the wiki scrape recorded any drop for -- all
                // that can be shown is what's actually been looted; there
                // is no "known but not yet gotten" list to fill in.
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
    /// The zone active on the log at this moment (`Ingest::zone`'s own
    /// `Spans::at`), if any -- `None` for loot from before the first zone
    /// line ever seen, the same "Unknown" bucket `combat::ZoneVisitDto`'s
    /// own doc covers.
    pub zone: Option<String>,
}

/// Every time `item` has actually been looted, oldest first -- the Game
/// Data module's "your history with this item" section on an item's own
/// page. Individual events, not the aggregate `list_mobs` already answers
/// ("how many total, from which mob") -- each one names a real timestamp
/// and (best-effort) zone, but not a specific encounter: loot rows *do*
/// now carry a best-effort `Store::enc` (`Ingest::record_loot`'s doc), it
/// just isn't surfaced here -- there's no "view in Combat" link on an
/// item page's own blips today, a scope call rather than something this
/// type can't support.
///
/// A full linear scan of the store, not an indexed lookup -- same
/// trade-off `list_mobs`'s own doc weighs for a per-mob scan: this runs
/// once when a page opens, not on a 3s poll, so it doesn't need the same
/// O(store length) discipline that view does. Worth revisiting with a
/// `loot_by_item` index if item pages ever get polled instead of opened.
pub fn item_loot_history(ing: &Ingest, item: &str) -> Vec<LootEventDto> {
    let mut out = Vec::new();
    for i in 0..ing.store.len() {
        if ing.store.kind[i] != EventKind::Loot {
            continue;
        }
        if !ing
            .store
            .ability_name(ing.store.ability[i])
            .eq_ignore_ascii_case(item)
        {
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

    /// Real lines from `eqlog_Manipulator_rivervale.txt` (2026-08-14,
    /// 21:34:52-21:34:53), trimmed to the load-bearing ones -- reported
    /// directly: a raid miniboss ("Fright") died to a party-mate's own
    /// hit, "You" never landed a single matched damage line against it
    /// in this window (only a still-resolving cast), yet still earned
    /// real party XP credit for the kill -- see `counts_as_pull`'s own
    /// doc for why that credit line has to count as much as a personal
    /// hit would.
    // why: the trailing line 11s after the death is load-bearing, not
    // filler -- a fight only closes (and only then does `Store::
    // Encounter::slain` flip true) once `graph::Builder::expire` sees
    // `idle_ms` (10s) of silence past it; a snippet ending right on the
    // death line leaves the encounter open forever and `slain` false, the
    // same real test-harness gap the ingest.rs `KILL_XP` fixture's own
    // doc already calls out.
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
        assert_eq!(stats.kills, 1, "party XP credit alone should be enough to count this as your kill");
        assert_eq!(stats.pulls, 1);

        let mobs = list_mobs(&ing);
        let fright = mobs.iter().find(|m| m.name.eq_ignore_ascii_case("Fright"));
        assert!(fright.is_some(), "list_mobs should surface Fright too, not just mob_stats");
        assert_eq!(fright.unwrap().kills, 1);
    }

    /// The bar this replaces (personal damage alone) still has to keep
    /// working on its own -- this isn't a strictly looser check that
    /// would start counting fights the player was never part of at all.
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
        assert_eq!(stats.kills, 0, "no personal damage and no party XP credit -- not the player's kill");
        assert_eq!(stats.pulls, 0);
    }
}

//! Read-side queries for the Endgame module's Raiding tab.
//!
//! The row/raid grouping below (`CURATED_ROWS`) is hand-curated, given
//! directly by the player, not wiki-derived: the wiki's own "Raid
//! Encounters" category tag (`npcdata::Npc::categories`) turned out to be
//! real but badly incomplete for this purpose -- confirmed against the
//! live scrape, none of Plane of Hate's 10 real miniboss NPCs below
//! (Ashenbone Broodmaster, Avatar of Abhorrence, ...) carry that tag at
//! all, despite every one of them having a genuine `known_loot` table
//! under their own NPC page. Category membership alone would have
//! silently dropped all ten. So every boss/miniboss here is looked up by
//! *exact name* against the full NPC catalog (`find_npc`), never by
//! category -- the curated list is the source of truth for "which zones
//! are raids and who's in them"; the wiki scrape only ever supplies a
//! named target's own stats/loot once that name is already known.
//!
//! Each boss/miniboss carries its own difficulty completion as a 2x5
//! grid, not a single 5-tier row: "Solo" and "Group" are two genuinely
//! different real instance types for the same zone (confirmed against a
//! real reference log -- e.g. "The Plane of Fear" vs "The Plane of Fear -
//! Group", both real, distinct zone-enter labels), each independently
//! carrying the game's usual 0-4 difficulty tier (`zone::zone_tier`'s
//! Base/Awakened/Adaptive/Fused/Refined scale). `Store::tier` already
//! carries the 0-4 tier for every row; the Solo/Group axis is read
//! straight off each encounter's own raw zone label (`Encounter::zone`)
//! for a literal `"- Group"` marker, independent of that tier stamp.

use crate::ingest::Ingest;
use crate::monsters;
use crate::npcdata;
use eqlp_store::{by_target_and_ability, EventKind, Sym};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct RaidDropDto {
    pub item: String,
    pub looted: bool,
    /// Total quantity looted so far, `0` for a wiki-known drop never
    /// gotten -- same convention `monsters::LootRowDto::count` uses.
    pub count: u64,
}

/// One boss or miniboss -- a raid's own `boss` and each of its
/// `minibosses` share this exact same shape, since both are tracked
/// identically (own kills, own difficulty grid, own drop table).
#[derive(Debug, Clone, Serialize)]
pub struct RaidTargetDto {
    pub name: String,
    /// The wiki's own raw level text ("66", "55-56", "?") -- never parsed
    /// into a number, since a range or "?" is itself real, meaningful
    /// data a single int would have to discard. `None` if this name
    /// doesn't resolve against the NPC catalog at all (a curation typo,
    /// or a real name the wiki scrape hasn't picked up yet).
    pub level: Option<String>,
    /// Confirmed kills, allegiance/self-damage-checked the same way
    /// `monsters::list_mobs` counts a real pull -- see `monsters::
    /// counts_as_pull`'s own doc. Sums both Solo and Group kills.
    pub kills: u64,
    /// Which of the 5 difficulty tiers this target has been confirmed
    /// killed at *while the zone was in its Solo form* -- index 0 is the
    /// base/untiered zone, 1-4 are Awakened/Adaptive/Fused/Refined.
    pub solo_tiers_cleared: [bool; 5],
    /// Same 5-tier scale, but confirmed while the zone was in its
    /// "- Group" form -- a genuinely different real instance, not a
    /// duplicate of `solo_tiers_cleared`. See this module's own doc.
    pub group_tiers_cleared: [bool; 5],
    /// Every item the wiki's own NPC page lists this target as dropping,
    /// each tagged with whether (and how many times) this character has
    /// actually looted it -- "drop completion" is this list's own
    /// looted-count out of its length. Empty if this name doesn't
    /// resolve against the NPC catalog, or resolves but the scrape
    /// recorded no drop table for it.
    pub drops: Vec<RaidDropDto>,
}

/// One confirmed best time -- the fastest run's own duration plus when
/// that run happened (`Millis`, same epoch every other timestamp in this
/// app uses), so the frontend can show "achieved <date>" as real
/// evidence behind the number, not a bare duration with nothing backing
/// it.
#[derive(Debug, Clone, Serialize)]
pub struct BestTimeDto {
    pub duration_ms: i64,
    pub achieved_ms: i64,
}

/// "Fastest times" -- a real speedrun timer, not a completion metric,
/// split the same way `RaidTargetDto`'s own difficulty grid is: index =
/// tier (0 = base, 1-4 = Awakened/Adaptive/Fused/Refined), `solo`/`group`
/// the same two real instance types that grid's own doc explains. Each
/// cell is the fastest confirmed real-time span from this character's
/// first real pull of *any* target in a zone visit at that exact tier +
/// mode, to the main boss's own kill in that same visit -- `None` where
/// that specific combination has never been cleared at all. (An earlier
/// version pooled every tier/mode into one bare "any%" number -- asked
/// directly, "is that per difficulty/solo v group or just single time",
/// and pooled turned out to be the wrong call: it hid *which* run a fast
/// time actually came from.)
///
/// "Full clear" (every boss *and* miniboss down, not just the main boss)
/// is a real idea worth having, but there's no agreed definition yet for
/// what that should even measure -- last miniboss kill vs. main boss
/// kill, whichever comes later? does a wipe/reset mid-clear disqualify
/// the run? -- so it stays unbuilt rather than shipping a guessed
/// answer. The frontend shows it as a labeled "coming soon" row, not a
/// fabricated number.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RaidTimesDto {
    pub solo: [Option<BestTimeDto>; 5],
    pub group: [Option<BestTimeDto>; 5],
}

#[derive(Debug, Clone, Serialize)]
pub struct RaidDto {
    pub zone: String,
    pub boss: RaidTargetDto,
    /// Empty for a raid with no separate named minibosses (e.g. Master
    /// Yael, Lady Vox) -- the frontend skips the miniboss section
    /// entirely in that case rather than showing an empty one.
    pub minibosses: Vec<RaidTargetDto>,
    pub times: RaidTimesDto,
}

#[derive(Debug, Clone, Serialize)]
pub struct RaidRowDto {
    pub row: String,
    pub raids: Vec<RaidDto>,
}

/// `(row label, &[(zone, main boss, [minibosses])])`, given directly by
/// the player -- see this module's own doc for why this is curated, not
/// wiki-derived. Ordered exactly as given; `list_raid_rows` preserves
/// this order rather than re-sorting, since row order is itself part of
/// the curation (e.g. "Early Game Raids" first on purpose).
const CURATED_ROWS: &[(&str, &[(&str, &str, &[&str])])] = &[
    (
        "Early Game Raids",
        &[
            ("The Hole", "Master Yael", &[]),
            ("Kedge Keep", "Phinigel Autropos", &[]),
        ],
    ),
    (
        "Dragons",
        &[
            ("Permafrost", "Lady Vox", &[]),
            ("Nagafen's Lair", "Lord Nagafen", &[]),
        ],
    ),
    (
        "Planes",
        &[
            ("Plane of Fear", "Cazic Thule", &["Fright", "Dread", "Terror", "a dracoliche"]),
            (
                "Plane of Hate",
                "Innoruuk",
                &[
                    "Ashenbone Broodmaster",
                    "Avatar of Abhorrence",
                    "Coercer T`vala",
                    "Grandmaster R`Tal",
                    "High Priest M`kari",
                    "Lord of Ire",
                    "Lord of Loathing",
                    "Magi P`Tasa",
                    "Master of Spite",
                    "Mistress of Scorn",
                    "Maestro of Rancor",
                ],
            ),
        ],
    ),
];

/// Exact-name lookup against the full NPC catalog -- see this module's
/// own doc for why category membership alone isn't reliable enough here.
/// First match wins on the rare real duplicate-name page (see
/// `npcdata`'s own doc on "(triggered)" variants) -- both real pages
/// describe the same in-game mob the log itself never distinguishes.
fn find_npc(name: &str) -> Option<&'static npcdata::Npc> {
    npcdata::npcs().iter().find(|n| n.name == name)
}

/// A curated/wiki boss name doesn't always match the exact entity name
/// the combat log itself uses -- confirmed directly against a real log,
/// reported by the player: the wiki page (and `CURATED_ROWS`'s own name,
/// since that's what `find_npc` needs) is "Cazic Thule"/"Innoruuk", but
/// real combat lines read "Cazic-Thule bashes..." and "Innoruuk, the
/// Prince of Hate hits..." respectively -- a hyphen and a full title the
/// wiki page doesn't carry. This is what kills/loot must actually be
/// looked up under; `find_npc` (level, known drop *list*) keeps using the
/// wiki/curated name unchanged, since that's the name the NPC page is
/// keyed by. Absent from this table means the two already agree -- true
/// for every miniboss checked so far (all 15 named minibosses across
/// Fear and Hate matched real kills on their curated name directly).
const LOG_NAME_ALIASES: &[(&str, &str)] = &[("Cazic Thule", "Cazic-Thule"), ("Innoruuk", "Innoruuk, the Prince of Hate")];

fn log_name(curated_name: &str) -> &str {
    LOG_NAME_ALIASES
        .iter()
        .find(|&&(wiki, _)| wiki == curated_name)
        .map(|&(_, log)| log)
        .unwrap_or(curated_name)
}

/// Per-mob-name (lowercase-folded -- the wiki's own casing and whatever
/// casing this character's log first saw a name under don't always
/// agree, same reason `monsters::mob_stats` matches case-insensitively
/// rather than through `Store::names`' exact interner lookup) kill counts
/// and difficulty grids, built in one pass over `ing.store.encounters`
/// (bounded -- a few thousand even on a long-lived character, see
/// `monsters::mob_stats`'s own doc).
#[derive(Default)]
struct KillGrid {
    kills: HashMap<String, u64>,
    solo_tiers: HashMap<String, [bool; 5]>,
    group_tiers: HashMap<String, [bool; 5]>,
}

fn build_kill_grid(ing: &Ingest, you: Sym, xp_credited: &HashSet<u32>) -> KillGrid {
    let mut grid = KillGrid::default();
    for e in &ing.store.encounters {
        if !e.slain || !monsters::counts_as_pull(ing, e, you, xp_credited) {
            continue;
        }
        let key = ing.store.name(e.target).to_ascii_lowercase();
        *grid.kills.entry(key.clone()).or_insert(0) += 1;
        let tier = ing.store.tier.get(e.first as usize).copied().unwrap_or(0) as usize;
        // why: the raw zone label this encounter actually opened under --
        // "- Group" is a real, distinct instance-type marker, separate
        // from the 0-4 tier suffix (see this module's own doc).
        let is_group = e.zone.is_some_and(|z| ing.store.name(z).contains("- Group"));
        let slots = if is_group {
            grid.group_tiers.entry(key).or_insert([false; 5])
        } else {
            grid.solo_tiers.entry(key).or_insert([false; 5])
        };
        if let Some(slot) = slots.get_mut(tier) {
            *slot = true;
        }
    }
    grid
}

/// "any%" -- see `RaidTimesDto`'s own doc for what this measures and why
/// "full clear" isn't attempted yet. Walks every real zone visit (`Ingest::
/// zone`'s own `Spans`) whose label resolves to `zone`, and within each,
/// finds the earliest real pull (`monsters::counts_as_pull`) and the
/// `boss_log_name` target's own kill, both by scanning `ing.store.
/// encounters` bounded to that visit's own `[start, end)` window. Keeps
/// only the fastest (start, duration) pair across every qualifying visit.
/// One pass over `ing.zone` (however many visits this session has, never
/// more than a few thousand) times one pass over `ing.store.encounters`
/// per visit -- bounded the same way `build_kill_grid`'s own single pass
/// is, not a per-query full-store scan.
/// See `RaidTimesDto`'s own doc for what this measures, the tier + Solo/
/// Group split, and why "full clear" isn't attempted yet. Walks every
/// real zone visit (`Ingest::zone`'s own `Spans`) whose label resolves to
/// `zone`; each visit's own label decides which one of the 10 `solo`/
/// `group` x tier cells it can possibly improve (a visit has exactly one
/// zone label for its whole span, so tier/mode are visit-wide facts, not
/// re-derived per encounter -- same `zone::zone_tier`/`"- Group"` reading
/// `build_kill_grid` already applies, just read off the span label
/// directly here instead of a stamped-at-ingest-time copy). Within each
/// visit, finds the earliest real pull (`monsters::counts_as_pull`) and
/// the `boss_log_name` target's own kill, both by scanning `ing.store.
/// encounters` bounded to that visit's own `[start, end)` window. One
/// pass over `ing.zone` times one pass over `ing.store.encounters` per
/// visit -- bounded the same way `build_kill_grid`'s own single pass is,
/// not a per-query full-store scan.
fn build_times(ing: &Ingest, zone: &str, boss_log_name: &str, you: Sym, xp_credited: &HashSet<u32>) -> RaidTimesDto {
    let spans: Vec<(eqlp_source::Millis, &str)> = ing.zone.iter().collect();
    let mut times = RaidTimesDto::default();

    for (i, &(start, label)) in spans.iter().enumerate() {
        if !crate::zone::zone_matches(label, zone) {
            continue;
        }
        let end = spans.get(i + 1).map(|&(s, _)| s).unwrap_or(i64::MAX);
        let tier = crate::zone::zone_tier(label).1 as usize;
        let is_group = label.contains("- Group");

        let mut first_action: Option<i64> = None;
        let mut boss_kill: Option<i64> = None;
        for e in &ing.store.encounters {
            if e.start_ms < start || e.start_ms >= end {
                continue;
            }
            if monsters::counts_as_pull(ing, e, you, xp_credited) {
                first_action = Some(first_action.map_or(e.start_ms, |f| f.min(e.start_ms)));
            }
            if e.slain && ing.store.name(e.target).eq_ignore_ascii_case(boss_log_name) {
                if let Some(end_ms) = e.end_ms {
                    boss_kill = Some(boss_kill.map_or(end_ms, |b| b.min(end_ms)));
                }
            }
        }

        if let (Some(fa), Some(bk)) = (first_action, boss_kill) {
            if bk > fa {
                let dur = bk - fa;
                let slot = if is_group { &mut times.group[tier] } else { &mut times.solo[tier] };
                if slot.as_ref().is_none_or(|best| dur < best.duration_ms) {
                    *slot = Some(BestTimeDto { duration_ms: dur, achieved_ms: fa });
                }
            }
        }
    }

    times
}

/// Every loot row grouped by (lowercase) target name -> item name ->
/// total quantity, same single-pass grouping `monsters::list_mobs`
/// already uses (`by_target_and_ability`), so this stays one scan
/// regardless of how many curated targets ask about it.
fn build_loot_index(ing: &Ingest) -> HashMap<String, HashMap<String, u64>> {
    let mut out: HashMap<String, HashMap<String, u64>> = HashMap::new();
    for (sym, rows) in by_target_and_ability(&ing.store, EventKind::Loot) {
        let key = ing.store.name(sym).to_ascii_lowercase();
        let entry = out.entry(key).or_default();
        for r in rows {
            // why: a real loot line names the *specific instance* looted
            // ("You looted an Engineer's Ring +4 from..."), but the
            // wiki's own drop table lists the untiered base name
            // ("Engineer's Ring") -- reported directly: a real, confirmed
            // drop was showing as "not looted" because those two strings
            // never compare equal. `strip_tier` (`inventory.rs`'s own
            // `/outputfile inventory` parser uses it the same way, for
            // the same reason) normalizes both to the base name; tiers
            // looted at different "+N" still sum into one total here,
            // same as inventory ownership already does.
            let (base_item, _tier) = crate::inventory::strip_tier(ing.store.ability_name(r.ability));
            *entry.entry(base_item.to_string()).or_insert(0) += r.total;
        }
    }
    out
}

fn target_dto(name: &str, grid: &KillGrid, loot: &HashMap<String, HashMap<String, u64>>) -> RaidTargetDto {
    // why: kills/tiers/loot are keyed by whatever the combat log itself
    // calls this entity, which is not always the curated/wiki name --
    // see `LOG_NAME_ALIASES`'s own doc. The drop *list* itself still
    // comes from `find_npc(name)` below, unchanged -- that's a wiki-page
    // lookup, not a log lookup.
    let key = log_name(name).to_ascii_lowercase();
    let npc = find_npc(name);
    let gotten = loot.get(&key);
    let drops = npc
        .map(|n| {
            n.known_loot
                .iter()
                .map(|kl| {
                    let count = gotten.and_then(|g| g.get(&kl.item)).copied().unwrap_or(0);
                    RaidDropDto {
                        item: kl.item.clone(),
                        looted: count > 0,
                        count,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    RaidTargetDto {
        name: name.to_string(),
        level: npc.and_then(|n| n.level.clone()),
        kills: grid.kills.get(&key).copied().unwrap_or(0),
        solo_tiers_cleared: grid.solo_tiers.get(&key).copied().unwrap_or([false; 5]),
        group_tiers_cleared: grid.group_tiers.get(&key).copied().unwrap_or([false; 5]),
        drops,
    }
}

/// The Raiding tab's whole data source -- see this module's own doc for
/// the curated row/raid/boss/miniboss shape and why it isn't wiki-derived.
pub fn list_raid_rows(ing: &Ingest) -> Vec<RaidRowDto> {
    // why: `you`/`xp_credited` computed once here, not once per raid --
    // `xp_credited_encounters` is a real full-store pass (see its own
    // doc), and this module only ever needs one copy of either per query.
    let you = ing.store.names.get("You");
    let xp_credited = you.map(|_| monsters::xp_credited_encounters(ing)).unwrap_or_default();
    let grid = you.map(|y| build_kill_grid(ing, y, &xp_credited)).unwrap_or_default();
    let loot = build_loot_index(ing);

    CURATED_ROWS
        .iter()
        .map(|&(row, raids)| RaidRowDto {
            row: row.to_string(),
            raids: raids
                .iter()
                .map(|&(zone, boss, minibosses)| RaidDto {
                    zone: zone.to_string(),
                    boss: target_dto(boss, &grid, &loot),
                    minibosses: minibosses.iter().map(|m| target_dto(m, &grid, &loot)).collect(),
                    times: you
                        .map(|y| build_times(ing, zone, log_name(boss), y, &xp_credited))
                        .unwrap_or_default(),
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;

    fn run(text: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        // why: `Store::Encounter::slain`/`end_ms` only get set once a
        // fight actually closes, which only ever happens off `Ingest::
        // tick`'s own wall-clock argument -- see `monsters::
        // pull_credit_tests::run`'s own doc for the two-tick pattern and
        // why a bare `backfill_lines` alone never triggers it.
        ing.mark_live();
        ing.tick(0);
        ing.tick(60_000);
        ing
    }

    /// Real lines, reported directly: two genuinely different real loot-
    /// line shapes (the "--...--"-bracketed "You have looted" form, and
    /// the unbracketed "You looted ... to create ..." merge form -- both
    /// already covered by the rule pack's own `loot.self`/`loot.self.
    /// direct` rules) -- neither drop showed as looted, because the item
    /// *this specific instance* was tiered at ("Engineer's Ring +4") was
    /// being compared, unnormalized, against the wiki's own untiered
    /// drop-table entry ("Engineer's Ring"), which can never compare
    /// equal. See `build_loot_index`'s own doc for the fix.
    #[test]
    fn tiered_loot_lines_match_the_wikis_untiered_drop_table_entry() {
        let text = "\
[Tue Aug 11 17:20:08 2026] --You have looted a Ring of Pureblood +4 from Innoruuk, the Prince of Hate's corpse.--
[Tue Aug 18 16:14:21 2026] You looted an Engineer's Ring +4 from Innoruuk, the Prince of Hate's corpse to create an Engineer's Ring +6
";
        let ing = run(text);
        let rows = list_raid_rows(&ing);
        let hate = rows.iter().flat_map(|r| &r.raids).find(|r| r.zone == "Plane of Hate").expect("Plane of Hate");
        let looted: Vec<&str> = hate.boss.drops.iter().filter(|d| d.looted).map(|d| d.item.as_str()).collect();
        assert!(looted.contains(&"Ring of Pureblood"), "looted drops were: {looted:?}");
        assert!(looted.contains(&"Engineer's Ring"), "looted drops were: {looted:?}");
    }

    /// Two full clears of the same raid, same tier + mode (both base/
    /// untiered, both Solo -- no "- Group" or tier suffix on either zone
    /// line), in two separate zone visits (a `Bazaar` line between them
    /// forces a genuinely new `Spans` entry -- re-entering the same label
    /// back-to-back collapses into one visit, see `Spans::enter`'s own
    /// doc), the second one faster. That one shared cell (`solo[0]`)
    /// should report the *faster* run's own duration and *its* start
    /// time, not the first one seen or an average -- every other cell
    /// stays `None`.
    #[test]
    fn fastest_time_reports_the_quickest_confirmed_clear_in_its_own_cell() {
        let text = "\
[Tue Aug 11 17:00:00 2026] You have entered Plane of Hate.
[Tue Aug 11 17:00:05 2026] You hit A very unpleasant hand for 5 points of damage.
[Tue Aug 11 17:10:00 2026] You hit Innoruuk, the Prince of Hate for 100 points of damage.
[Tue Aug 11 17:10:00 2026] You have slain Innoruuk, the Prince of Hate!
[Tue Aug 11 17:20:00 2026] You have entered The Bazaar.
[Tue Aug 11 17:25:00 2026] You have entered Plane of Hate.
[Tue Aug 11 17:25:05 2026] You hit A very unpleasant hand for 5 points of damage.
[Tue Aug 11 17:30:00 2026] You hit Innoruuk, the Prince of Hate for 100 points of damage.
[Tue Aug 11 17:30:00 2026] You have slain Innoruuk, the Prince of Hate!
";
        let ing = run(text);
        let rows = list_raid_rows(&ing);
        let hate = rows.iter().flat_map(|r| &r.raids).find(|r| r.zone == "Plane of Hate").expect("Plane of Hate");
        // First run: 17:00:05 -> 17:10:00 = 595s. Second: 17:25:05 ->
        // 17:30:00 = 295s -- the second, faster run should win.
        assert_eq!(hate.times.solo[0].as_ref().map(|t| t.duration_ms), Some(295_000));
        assert!(hate.times.group.iter().all(Option::is_none), "no Group run happened -- that whole row should stay empty");
        assert!(hate.times.solo[1..].iter().all(Option::is_none), "only the base/D0 tier was actually run");
    }

    /// Asked directly ("is that per difficulty/solo v group or just
    /// single time") -- a Solo/D0 clear and a Group/D4 clear of the same
    /// raid must land in two genuinely separate cells, not compete for
    /// one pooled "fastest ever" number.
    #[test]
    fn solo_and_group_clears_land_in_separate_cells() {
        let text = "\
[Tue Aug 11 17:00:00 2026] You have entered Plane of Hate.
[Tue Aug 11 17:00:05 2026] You hit A very unpleasant hand for 5 points of damage.
[Tue Aug 11 17:10:00 2026] You hit Innoruuk, the Prince of Hate for 100 points of damage.
[Tue Aug 11 17:10:00 2026] You have slain Innoruuk, the Prince of Hate!
[Tue Aug 11 17:20:00 2026] You have entered The Bazaar.
[Tue Aug 11 17:25:00 2026] You have entered Plane of Hate - Group 4 (Refined).
[Tue Aug 11 17:25:05 2026] You hit A very unpleasant hand for 5 points of damage.
[Tue Aug 11 17:36:00 2026] You hit Innoruuk, the Prince of Hate for 100 points of damage.
[Tue Aug 11 17:36:00 2026] You have slain Innoruuk, the Prince of Hate!
";
        let ing = run(text);
        let rows = list_raid_rows(&ing);
        let hate = rows.iter().flat_map(|r| &r.raids).find(|r| r.zone == "Plane of Hate").expect("Plane of Hate");
        assert_eq!(hate.times.solo[0].as_ref().map(|t| t.duration_ms), Some(595_000), "the Solo/D0 run");
        assert_eq!(hate.times.group[4].as_ref().map(|t| t.duration_ms), Some(655_000), "the Group/D4 run");
    }

    /// A raid whose boss has never been killed reports `None` in every
    /// cell, not `0` or a panic -- there is no real run to report a time
    /// for yet.
    #[test]
    fn a_raid_never_cleared_reports_no_fastest_time_anywhere() {
        let ing = Ingest::default();
        let rows = list_raid_rows(&ing);
        for row in &rows {
            for raid in &row.raids {
                assert!(raid.times.solo.iter().all(Option::is_none), "{} solo should report no time yet", raid.zone);
                assert!(raid.times.group.iter().all(Option::is_none), "{} group should report no time yet", raid.zone);
            }
        }
    }

    /// Every curated name has to actually resolve against the real
    /// embedded NPC catalog -- a typo or a wiki-scrape drift here would
    /// otherwise silently show up as an empty drop table / no level, not
    /// a loud failure.
    #[test]
    fn every_curated_boss_and_miniboss_name_resolves_to_a_real_npc() {
        for &(_, raids) in CURATED_ROWS {
            for &(zone, boss, minibosses) in raids {
                assert!(find_npc(boss).is_some(), "{boss} (main boss of {zone}) not found in the real NPC catalog");
                for m in minibosses {
                    assert!(find_npc(m).is_some(), "{m} (miniboss of {zone}) not found in the real NPC catalog");
                }
            }
        }
    }

    /// A regression check for the exact gap that made this module stop
    /// trusting the wiki's own "Raid Encounters" tag: none of Plane of
    /// Hate's 10 real minibosses carry that tag at all, so this only
    /// passes if lookup is genuinely by name.
    #[test]
    fn plane_of_hate_minibosses_carry_real_drop_tables_despite_no_wiki_raid_tag() {
        let ing = Ingest::default();
        let rows = list_raid_rows(&ing);
        let hate = rows
            .iter()
            .flat_map(|r| &r.raids)
            .find(|r| r.zone == "Plane of Hate")
            .expect("Plane of Hate should be a curated raid");
        assert_eq!(hate.minibosses.len(), 11);
        assert!(
            hate.minibosses.iter().all(|m| !m.drops.is_empty()),
            "every real Plane of Hate miniboss should carry a real known drop table"
        );
    }

    /// A brand-new session (no encounters at all) reports every curated
    /// target with zero kills and no tiers cleared in either grid, not a
    /// panic or a missing entry.
    #[test]
    fn a_fresh_session_shows_every_curated_target_uncleared() {
        let ing = Ingest::default();
        let rows = list_raid_rows(&ing);
        assert!(!rows.is_empty());
        for row in &rows {
            for raid in &row.raids {
                for target in std::iter::once(&raid.boss).chain(raid.minibosses.iter()) {
                    assert_eq!(target.kills, 0);
                    assert_eq!(target.solo_tiers_cleared, [false; 5]);
                    assert_eq!(target.group_tiers_cleared, [false; 5]);
                    assert!(target.drops.iter().all(|d| !d.looted && d.count == 0));
                }
            }
        }
    }

    /// Raids the player named as having no separate minibosses (Master
    /// Yael, Phinigel Autropos, Lady Vox, Lord Nagafen) report an empty
    /// miniboss list, not an invented one.
    #[test]
    fn raids_without_named_minibosses_report_an_empty_miniboss_list() {
        let ing = Ingest::default();
        let rows = list_raid_rows(&ing);
        for zone in ["The Hole", "Kedge Keep", "Permafrost", "Nagafen's Lair"] {
            let raid = rows
                .iter()
                .flat_map(|r| &r.raids)
                .find(|r| r.zone == zone)
                .unwrap_or_else(|| panic!("expected a {zone} raid"));
            assert!(raid.minibosses.is_empty(), "{zone} should have no minibosses");
        }
    }

    /// Regression check for the exact bug the player caught: the curated/
    /// wiki names "Cazic Thule" and "Innoruuk" don't match what the
    /// combat log itself calls those entities ("Cazic-Thule", "Innoruuk,
    /// the Prince of Hate") -- without `LOG_NAME_ALIASES`, a real kill or
    /// loot drop for either would silently never match at all.
    #[test]
    fn main_boss_names_that_differ_from_their_log_entity_name_still_resolve() {
        assert_eq!(log_name("Cazic Thule"), "Cazic-Thule");
        assert_eq!(log_name("Innoruuk"), "Innoruuk, the Prince of Hate");
        // Everything else passes through unchanged.
        assert_eq!(log_name("Lord Nagafen"), "Lord Nagafen");
    }

    /// Row order is part of the curation, not incidental -- "Early Game
    /// Raids" first, then "Dragons", then "Planes", exactly as given.
    #[test]
    fn rows_come_back_in_curated_order() {
        let ing = Ingest::default();
        let rows = list_raid_rows(&ing);
        let names: Vec<&str> = rows.iter().map(|r| r.row.as_str()).collect();
        assert_eq!(names, vec!["Early Game Raids", "Dragons", "Planes"]);
    }
}

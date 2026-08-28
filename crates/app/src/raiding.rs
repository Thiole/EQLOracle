//! why: Endgame's Raiding tab, read-side queries
//!
//! `CURATED_ROWS` is hand-curated, not wiki-derived -- the wiki's "Raid
//! Encounters" tag is badly incomplete (none of Plane of Hate's 10 real
//! minibosses carry it, despite real drop tables). Every boss/miniboss
//! looked up by exact name (`find_npc`), never by category.
//!
//! Each target's completion is a 2x5 grid: Solo/Group (real distinct
//! instance types, confirmed in the reference log) x 0-4 difficulty tier.
//! Solo/Group read off the raw zone label's "- Group" marker.

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
    /// why: 0 for a wiki-known drop never gotten, same as `monsters::LootRowDto::count`
    pub count: u64,
}

/// why: boss and miniboss share this shape -- both tracked identically
#[derive(Debug, Clone, Serialize)]
pub struct RaidTargetDto {
    pub name: String,
    /// why: raw wiki level text, never parsed -- a range or "?" is real
    /// data; None if the name doesn't resolve against the NPC catalog
    pub level: Option<String>,
    /// why: allegiance-checked confirmed kills, sums both Solo and Group
    pub kills: u64,
    /// why: tiers cleared while the zone was in Solo form; index 0 = base
    pub solo_tiers_cleared: [bool; 5],
    /// why: same scale, Group form -- a genuinely different real instance
    pub group_tiers_cleared: [bool; 5],
    /// why: every wiki-known drop tagged with looted status; empty if unresolved
    pub drops: Vec<RaidDropDto>,
}

/// why: fastest run's duration + when it happened, for "achieved <date>" evidence
#[derive(Debug, Clone, Serialize)]
pub struct BestTimeDto {
    pub duration_ms: i64,
    pub achieved_ms: i64,
}

/// why: speedrun timer, indexed by tier x solo/group -- each cell is the
/// fastest span from first real pull to the main boss kill in that
/// visit, None if never cleared. Per direct correction: pooling every
/// tier/mode into one number hid which run it came from.
///
/// "Full clear" (boss + every miniboss) has no agreed definition yet
/// (which kill counts as done? does a wipe disqualify?) -- stays unbuilt
/// rather than shipping a guessed answer.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RaidTimesDto {
    pub solo: [Option<BestTimeDto>; 5],
    pub group: [Option<BestTimeDto>; 5],
}

#[derive(Debug, Clone, Serialize)]
pub struct RaidDto {
    pub zone: String,
    pub boss: RaidTargetDto,
    /// why: empty for a raid with no named minibosses -- frontend skips the section
    pub minibosses: Vec<RaidTargetDto>,
    pub times: RaidTimesDto,
}

#[derive(Debug, Clone, Serialize)]
pub struct RaidRowDto {
    pub row: String,
    pub raids: Vec<RaidDto>,
}

/// `(zone, main boss, [minibosses])`.
type CuratedRaid = (&'static str, &'static str, &'static [&'static str]);
/// `(row label, its raids)`.
type CuratedRow = (&'static str, &'static [CuratedRaid]);

/// why: given directly by the player; order is part of the curation, preserved as-is
const CURATED_ROWS: &[CuratedRow] = &[
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
            (
                "Plane of Fear",
                "Cazic Thule",
                &["Fright", "Dread", "Terror", "a dracoliche"],
            ),
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

/// why: exact-name catalog lookup; first match wins on a rare duplicate-name page
fn find_npc(name: &str) -> Option<&'static npcdata::Npc> {
    npcdata::npcs().iter().find(|n| n.name == name)
}

/// why: wiki name doesn't always match the real log entity name --
/// confirmed "Cazic Thule"/"Innoruuk" log as "Cazic-Thule"/"Innoruuk,
/// the Prince of Hate". This is what kills/loot look up under; `find_npc`
/// still uses the wiki name unchanged. Absent means the two already agree.
const LOG_NAME_ALIASES: &[(&str, &str)] = &[
    ("Cazic Thule", "Cazic-Thule"),
    ("Innoruuk", "Innoruuk, the Prince of Hate"),
];

fn log_name(curated_name: &str) -> &str {
    LOG_NAME_ALIASES
        .iter()
        .find(|&&(wiki, _)| wiki == curated_name)
        .map(|&(_, log)| log)
        .unwrap_or(curated_name)
}

/// why: real log names of every curated boss/miniboss, case-insensitive
/// -- lets `ingest::link` retarget an encounter's anchor onto a raid
/// boss that joins mid-pull instead of leaving it stuck on whatever
/// trash mob opened the fight. Confirmed real gap: a live group Lady Vox
/// kill recorded end-to-end under "An icy terror"'s own encounter
/// (she engaged an already-open room pull, never becoming its anchor),
/// invisible to the Raiding tab despite a real, confirmed kill.
pub fn is_curated_raid_target(name: &str) -> bool {
    static NAMES: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
    NAMES
        .get_or_init(|| {
            CURATED_ROWS
                .iter()
                .flat_map(|&(_, raids)| raids.iter())
                .flat_map(|&(_, boss, minibosses)| {
                    std::iter::once(boss).chain(minibosses.iter().copied())
                })
                .map(|n| log_name(n).to_ascii_lowercase())
                .collect()
        })
        .contains(&name.to_ascii_lowercase())
}

/// why: lowercase-folded kill counts and difficulty grids, one pass over
/// encounters -- same case-insensitive stance as `monsters::mob_stats`
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
        // why: "- Group" is a real distinct instance marker, separate from the 0-4 tier
        let is_group = e
            .zone
            .is_some_and(|z| ing.store.name(z).contains("- Group"));
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

/// why: "any%" -- walks every zone visit matching `zone`; each visit's
/// label decides tier/mode (visit-wide, not per-encounter). Within each
/// visit, finds the earliest real pull and the boss's own kill, scanning
/// encounters bounded to `[start, end)`. Keeps the fastest pair per cell.
/// One pass over visits times one pass over encounters per visit --
/// bounded the same way `build_kill_grid`'s single pass is.
fn build_times(
    ing: &Ingest,
    zone: &str,
    boss_log_name: &str,
    you: Sym,
    xp_credited: &HashSet<u32>,
) -> RaidTimesDto {
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
                let slot = if is_group {
                    &mut times.group[tier]
                } else {
                    &mut times.solo[tier]
                };
                if slot.as_ref().is_none_or(|best| dur < best.duration_ms) {
                    *slot = Some(BestTimeDto {
                        duration_ms: dur,
                        achieved_ms: fa,
                    });
                }
            }
        }
    }

    times
}

/// why: per-visit trace for the `raid_timer_debug` example -- same walk
/// `build_times` does, but describing every visit instead of only the
/// fastest. Not used by any real command, kept for empirical debugging.
pub fn debug_visit_trace(ing: &Ingest, zone: &str, boss_log_name: &str) -> Vec<String> {
    let Some(you) = ing.store.names.get("You") else {
        return vec!["no 'You' interned yet".to_string()];
    };
    let xp_credited = monsters::xp_credited_encounters(ing);
    let spans: Vec<(eqlp_source::Millis, &str)> = ing.zone.iter().collect();
    let mut out = Vec::new();
    for (i, &(start, label)) in spans.iter().enumerate() {
        if !crate::zone::zone_matches(label, zone) {
            continue;
        }
        let end = spans.get(i + 1).map(|&(s, _)| s).unwrap_or(i64::MAX);
        let (base, tier) = crate::zone::zone_tier(label);
        let is_group = label.contains("- Group");
        out.push(format!(
            "visit #{i}: label={label:?} base={base:?} tier={tier} is_group={is_group} start={start} end={end}"
        ));

        let mut first_action: Option<(i64, String)> = None;
        let mut boss_kill: Option<i64> = None;
        for e in &ing.store.encounters {
            if e.start_ms < start || e.start_ms >= end {
                continue;
            }
            let name = ing.store.name(e.target);
            let is_pull = monsters::counts_as_pull(ing, e, you, &xp_credited);
            if name.eq_ignore_ascii_case(boss_log_name) {
                out.push(format!(
                    "  boss encounter: start={} slain={} end={:?} counts_as_pull={is_pull}",
                    e.start_ms, e.slain, e.end_ms
                ));
            }
            if is_pull && first_action.as_ref().is_none_or(|(f, _)| e.start_ms < *f) {
                first_action = Some((e.start_ms, name.to_string()));
            }
            if e.slain && name.eq_ignore_ascii_case(boss_log_name) {
                if let Some(end_ms) = e.end_ms {
                    boss_kill = Some(boss_kill.map_or(end_ms, |b| b.min(end_ms)));
                }
            }
        }
        out.push(format!(
            "  first_action={:?} boss_kill={boss_kill:?}",
            first_action.as_ref().map(|(t, n)| format!("{t} ({n})"))
        ));
        match (&first_action, boss_kill) {
            (Some((fa, _)), Some(bk)) if bk > *fa => {
                out.push(format!("  => WOULD RECORD duration_ms={}", bk - fa))
            }
            (Some(_), Some(_)) => out.push("  => boss_kill <= first_action, REJECTED".to_string()),
            (None, Some(_)) => {
                out.push("  => no qualifying pull found at all, REJECTED".to_string())
            }
            (Some(_), None) => {
                out.push("  => boss never confirmed slain in this visit".to_string())
            }
            (None, None) => out.push("  => nothing relevant happened in this visit".to_string()),
        }
    }
    out
}

/// why: one-pass loot grouping, same shape as `monsters::list_mobs` uses
fn build_loot_index(ing: &Ingest) -> HashMap<String, HashMap<String, u64>> {
    let mut out: HashMap<String, HashMap<String, u64>> = HashMap::new();
    for (sym, rows) in by_target_and_ability(&ing.store, EventKind::Loot) {
        let key = ing.store.name(sym).to_ascii_lowercase();
        let entry = out.entry(key).or_default();
        for r in rows {
            // why: a real loot line names the tiered instance ("+4"), but
            // the wiki's drop table lists the untiered base name --
            // `strip_tier` normalizes both, tiers sum into one total
            let (base_item, _tier) =
                crate::inventory::strip_tier(ing.store.ability_name(r.ability));
            *entry.entry(base_item.to_string()).or_insert(0) += r.total;
        }
    }
    out
}

fn target_dto(
    name: &str,
    grid: &KillGrid,
    loot: &HashMap<String, HashMap<String, u64>>,
) -> RaidTargetDto {
    // why: kills/tiers/loot keyed by the log's own entity name, not
    // always the wiki name -- drop list still comes from find_npc(name) unchanged
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

/// why: the Raiding tab's whole data source
pub fn list_raid_rows(ing: &Ingest) -> Vec<RaidRowDto> {
    // why: computed once here, not once per raid -- a real full-store pass
    let you = ing.store.names.get("You");
    let xp_credited = you
        .map(|_| monsters::xp_credited_encounters(ing))
        .unwrap_or_default();
    let grid = you
        .map(|y| build_kill_grid(ing, y, &xp_credited))
        .unwrap_or_default();
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
                    minibosses: minibosses
                        .iter()
                        .map(|m| target_dto(m, &grid, &loot))
                        .collect(),
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
        // why: slain/end_ms only set once tick closes the fight -- see
        // `monsters::pull_credit_tests::run`'s two-tick pattern
        ing.mark_live();
        ing.tick(0);
        ing.tick(60_000);
        ing
    }

    /// why: two real loot-line shapes, tiered instance vs wiki's untiered entry
    #[test]
    fn tiered_loot_lines_match_the_wikis_untiered_drop_table_entry() {
        let text = "\
[Tue Aug 11 17:20:08 2026] --You have looted a Ring of Pureblood +4 from Innoruuk, the Prince of Hate's corpse.--
[Tue Aug 18 16:14:21 2026] You looted an Engineer's Ring +4 from Innoruuk, the Prince of Hate's corpse to create an Engineer's Ring +6
";
        let ing = run(text);
        let rows = list_raid_rows(&ing);
        let hate = rows
            .iter()
            .flat_map(|r| &r.raids)
            .find(|r| r.zone == "Plane of Hate")
            .expect("Plane of Hate");
        let looted: Vec<&str> = hate
            .boss
            .drops
            .iter()
            .filter(|d| d.looted)
            .map(|d| d.item.as_str())
            .collect();
        assert!(
            looted.contains(&"Ring of Pureblood"),
            "looted drops were: {looted:?}"
        );
        assert!(
            looted.contains(&"Engineer's Ring"),
            "looted drops were: {looted:?}"
        );
    }

    /// why: two clears, same cell, second faster -- must report the faster one, not first/average
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
        let hate = rows
            .iter()
            .flat_map(|r| &r.raids)
            .find(|r| r.zone == "Plane of Hate")
            .expect("Plane of Hate");
        // why: first run 595s, second 295s -- the faster run should win
        assert_eq!(
            hate.times.solo[0].as_ref().map(|t| t.duration_ms),
            Some(295_000)
        );
        assert!(
            hate.times.group.iter().all(Option::is_none),
            "no Group run happened -- that whole row should stay empty"
        );
        assert!(
            hate.times.solo[1..].iter().all(Option::is_none),
            "only the base/D0 tier was actually run"
        );
    }

    /// why: Solo/D0 and Group/D4 clears must land in separate cells, not pool
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
        let hate = rows
            .iter()
            .flat_map(|r| &r.raids)
            .find(|r| r.zone == "Plane of Hate")
            .expect("Plane of Hate");
        assert_eq!(
            hate.times.solo[0].as_ref().map(|t| t.duration_ms),
            Some(595_000),
            "the Solo/D0 run"
        );
        assert_eq!(
            hate.times.group[4].as_ref().map(|t| t.duration_ms),
            Some(655_000),
            "the Group/D4 run"
        );
    }

    /// why: real bug -- Lady Vox joined an already-open room-wide trash
    /// pull mid-fight and died to a groupmate's own finishing blow; the
    /// encounter's anchor never moved off the trash mob that opened it
    /// (the existing stale-ally retarget only fires for an ally anchor),
    /// so her kill vanished from the Raiding tab entirely despite a real,
    /// confirmed group clear.
    #[test]
    fn a_boss_that_joins_an_already_open_trash_pull_still_counts_its_own_kill() {
        let text = "\
[Tue Aug 11 17:00:00 2026] You have entered Plane of Hate - Group 4 (Refined).
[Tue Aug 11 17:00:05 2026] You hit A very unpleasant hand for 5 points of damage.
[Tue Aug 11 17:00:10 2026] You hit Innoruuk, the Prince of Hate for 50 points of damage.
[Tue Aug 11 17:00:20 2026] Innoruuk, the Prince of Hate has been slain by Groupmate!
";
        let ing = run(text);
        let rows = list_raid_rows(&ing);
        let hate = rows
            .iter()
            .flat_map(|r| &r.raids)
            .find(|r| r.zone == "Plane of Hate")
            .expect("Plane of Hate");
        assert_eq!(
            hate.boss.kills, 1,
            "Innoruuk's kill must count even though he joined an open trash encounter and a groupmate landed the final blow"
        );
        assert!(
            hate.boss.group_tiers_cleared[4],
            "should register as a Group/D4 clear"
        );
    }

    /// why: never-killed boss reports None everywhere, not 0 or a panic
    #[test]
    fn a_raid_never_cleared_reports_no_fastest_time_anywhere() {
        let ing = Ingest::default();
        let rows = list_raid_rows(&ing);
        for row in &rows {
            for raid in &row.raids {
                assert!(
                    raid.times.solo.iter().all(Option::is_none),
                    "{} solo should report no time yet",
                    raid.zone
                );
                assert!(
                    raid.times.group.iter().all(Option::is_none),
                    "{} group should report no time yet",
                    raid.zone
                );
            }
        }
    }

    /// why: every curated name must resolve, else a typo silently shows as empty
    #[test]
    fn every_curated_boss_and_miniboss_name_resolves_to_a_real_npc() {
        for &(_, raids) in CURATED_ROWS {
            for &(zone, boss, minibosses) in raids {
                assert!(
                    find_npc(boss).is_some(),
                    "{boss} (main boss of {zone}) not found in the real NPC catalog"
                );
                for m in minibosses {
                    assert!(
                        find_npc(m).is_some(),
                        "{m} (miniboss of {zone}) not found in the real NPC catalog"
                    );
                }
            }
        }
    }

    /// why: regression -- none of Hate's minibosses carry the wiki tag, only passes by-name
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

    /// why: fresh session reports every target uncleared, not a panic or missing entry
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

    /// why: raids with no separate minibosses report empty, not invented
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
            assert!(
                raid.minibosses.is_empty(),
                "{zone} should have no minibosses"
            );
        }
    }

    /// why: regression -- wiki names differ from real log entity names
    #[test]
    fn main_boss_names_that_differ_from_their_log_entity_name_still_resolve() {
        assert_eq!(log_name("Cazic Thule"), "Cazic-Thule");
        assert_eq!(log_name("Innoruuk"), "Innoruuk, the Prince of Hate");
        // why: everything else passes through unchanged
        assert_eq!(log_name("Lord Nagafen"), "Lord Nagafen");
    }

    /// why: row order is part of the curation, not incidental
    #[test]
    fn rows_come_back_in_curated_order() {
        let ing = Ingest::default();
        let rows = list_raid_rows(&ing);
        let names: Vec<&str> = rows.iter().map(|r| r.row.as_str()).collect();
        assert_eq!(names, vec!["Early Game Raids", "Dragons", "Planes"]);
    }
}

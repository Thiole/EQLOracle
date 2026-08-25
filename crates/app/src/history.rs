//! why: persisted parse history, one record per closed encounter -- lets
//! "how did this compare to past kills" outlive `Store`'s own eviction
//! within one run.
//!
//! Wiped every launch (`reset`, from `main.rs`'s `setup`) -- carrying
//! records across a restart into different detection logic once caused
//! ~2,900 loadouts claiming impossible 4-10 simultaneous classes.
//!
//! JSON Lines, not one array -- appending a line is O(1), no read-back.
//! `Ingest` builds pure-data `ParseRecord`s; this is the only disk I/O.

use eqlp_session::ClassDetector;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseRecord {
    pub target: String,
    /// why: full zone label at fight start, doubles as tier readout via
    /// `zone::zone_tier`; #[serde(default)] for pre-field records
    #[serde(default)]
    pub zone: String,
    /// why: confirmed classes for this zone visit as of close, alphabetical
    /// (so `by_loadout` can group by key); empty means no evidence yet, not "no class"
    #[serde(default)]
    pub loadout: Vec<String>,
    /// why: zone visit index, lets `refresh_loadouts` re-resolve `loadout`
    /// against the detector's current state
    #[serde(default)]
    pub zone_visit: Option<usize>,
    pub start_ms: i64,
    pub duration_ms: i64,
    /// why: player's own damage/DPS, not the team's combined total
    pub player_damage: u64,
    pub player_dps: f64,
    /// why: confirmed kill vs timeout/reset -- only ~21% get a confirmed
    /// kill; comparisons MUST filter on this or a Reset skews the average
    pub confirmed_kill: bool,
    /// why: score vs this player's own average against this target at
    /// this tier, not an all-mobs/all-tiers blend. None (not 0.0) with no
    /// baseline yet, or when written during backfill (whole-store scan
    /// too costly at that scale, skipped -- see `Ingest::record_history`).
    /// Computed with neutral `GearModifiers::default()`, no gear detection yet.
    /// Known imprecision: baseline includes this encounter's own hits,
    /// self-diluted especially on an ability's very first use.
    pub score_ratio: Option<f64>,
}

const FILE_NAME: &str = "parse_history.jsonl";

fn history_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(FILE_NAME))
}

/// why: wipes persisted history at every launch, before the worker exists.
/// Best-effort/silent -- fresh install has nothing to remove. Scoped to
/// `app_data_dir`, never the game's own `Logs/` folder.
pub fn reset(app: &AppHandle) {
    if let Ok(path) = history_path(app) {
        let _ = std::fs::remove_file(path);
    }
}

/// why: append-only -- a crash mid-write loses at most this one record
pub fn append(app: &AppHandle, record: &ParseRecord) -> Result<(), String> {
    let path = history_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut line = serde_json::to_string(record).map_err(|e| e.to_string())?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    f.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

/// why: re-resolves `loadout` against live detector state by `zone_visit`
/// -- always safe, a confirmed class within a visit is never un-confirmed.
/// Called at read time, not write time -- write still stamps as-of-close
/// so a record is never empty before this can run.
pub fn refresh_loadouts(records: &mut [ParseRecord], classes: &ClassDetector, you: u32) {
    for r in records.iter_mut() {
        r.loadout = classes.configuration_of_visit(you, r.zone_visit);
    }
}

/// why: all records, oldest first; input: app handle; output: raw records
pub fn all(app: &AppHandle) -> Vec<ParseRecord> {
    let path = match history_path(app) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    let Ok(f) = std::fs::File::open(&path) else {
        return Vec::new();
    };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<ParseRecord>(&line).ok())
        .collect()
}

/// why: pure filter, shared by file-backed path and fixture dumping
pub fn filter_for_target(records: Vec<ParseRecord>, target: &str) -> Vec<ParseRecord> {
    records.into_iter().filter(|r| r.target == target).collect()
}

/// why: narrows to confirmed kills only
pub fn only_confirmed_kills(records: Vec<ParseRecord>) -> Vec<ParseRecord> {
    records.into_iter().filter(|r| r.confirmed_kill).collect()
}

/// Every past record for `target`, oldest first, unfiltered by kill status.
pub fn for_target(app: &AppHandle, target: &str) -> Vec<ParseRecord> {
    filter_for_target(all(app), target)
}

/// Only confirmed kills against `target`.
pub fn confirmed_kills_for_target(app: &AppHandle, target: &str) -> Vec<ParseRecord> {
    only_confirmed_kills(for_target(app, target))
}

/// why: get_mob_history's view; output: target-filtered, newest first
pub fn mob_history_view(
    records: Vec<ParseRecord>,
    target: &str,
    confirmed_only: bool,
) -> Vec<ParseRecord> {
    let mut records = filter_for_target(records, target);
    if confirmed_only {
        records = only_confirmed_kills(records);
    }
    records.reverse();
    records
}

/// why: bucket by loadout; empty loadout is its own real, distinct bucket
#[derive(Debug, Clone, Serialize)]
pub struct LoadoutSummary {
    pub loadout: Vec<String>,
    pub fights: usize,
    pub confirmed_kills: usize,
    /// why: mean of each record's own DPS, not pooled damage/duration --
    /// one long grind shouldn't dominate over several short fights
    pub avg_dps: f64,
    /// why: mean of records that have a score at all; None if none do
    pub avg_score_ratio: Option<f64>,
}

/// why: groups by loadout, sorted by fight count descending; caller filters first
pub fn by_loadout(records: &[ParseRecord]) -> Vec<LoadoutSummary> {
    let mut groups: HashMap<Vec<String>, Vec<&ParseRecord>> = HashMap::new();
    for r in records {
        groups.entry(r.loadout.clone()).or_default().push(r);
    }
    let mut out: Vec<LoadoutSummary> = groups
        .into_iter()
        .map(|(loadout, recs)| {
            let fights = recs.len();
            let confirmed_kills = recs.iter().filter(|r| r.confirmed_kill).count();
            let avg_dps = recs.iter().map(|r| r.player_dps).sum::<f64>() / fights as f64;
            let scored: Vec<f64> = recs.iter().filter_map(|r| r.score_ratio).collect();
            let avg_score_ratio = if scored.is_empty() {
                None
            } else {
                Some(scored.iter().sum::<f64>() / scored.len() as f64)
            };
            LoadoutSummary {
                loadout,
                fights,
                confirmed_kills,
                avg_dps,
                avg_score_ratio,
            }
        })
        .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.fights));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn fake_record(loadout: &[&str], zone_visit: Option<usize>) -> ParseRecord {
        ParseRecord {
            target: "Test Mob".to_string(),
            zone: "Befallen".to_string(),
            loadout: strs(loadout),
            zone_visit,
            start_ms: 0,
            duration_ms: 1_000,
            player_damage: 100,
            player_dps: 100.0,
            confirmed_kill: true,
            score_ratio: None,
        }
    }

    /// why: real bug fix -- a fight closed early must pick up later evidence
    /// from the same visit, not stay frozen at close-time state
    #[test]
    fn an_earlier_fights_partial_loadout_is_backfilled_by_later_evidence_in_the_same_visit() {
        // why: elimination narrowing needs 3 distinct visits to
        // corroborate, a stricter bar than an unambiguous cast's own 2
        // (real bug found live -- see classdetect module's own doc on
        // MIN_ELIMINATION_CASTS), so this checks the record stays
        // partial until a 3rd visit corroborates, then gets backfilled
        // retroactively same as before.
        let mut classes = ClassDetector::default();
        for v in [Some(0), Some(1)] {
            classes.observe_cast(1, v, &strs(&["Wizard"]));
            classes.observe_cast(1, v, &strs(&["Enchanter"]));
        }
        // why: fight 1 closes here, frozen at 2/3, matches close-time stamp
        classes.observe_cast(1, Some(2), &strs(&["Wizard"]));
        classes.observe_cast(1, Some(2), &strs(&["Enchanter"]));
        let mut records = vec![fake_record(&["Enchanter", "Wizard"], Some(2))];

        // why: same visit narrows to Cleric, but not proof by itself
        classes.observe_cast(1, Some(2), &strs(&["Beastlord", "Cleric", "Druid"]));
        classes.observe_cast(1, Some(2), &strs(&["Cleric", "Paladin", "Shaman"]));
        refresh_loadouts(&mut records, &classes, 1);
        assert_eq!(
            records[0].loadout,
            strs(&["Enchanter", "Wizard"]),
            "narrowed to Cleric on just this one visit -- not proof by itself"
        );

        // why: a 2nd, distinct visit narrowing to the same class -- still
        // not enough
        classes.observe_cast(1, Some(3), &strs(&["Wizard"]));
        classes.observe_cast(1, Some(3), &strs(&["Enchanter"]));
        classes.observe_cast(1, Some(3), &strs(&["Beastlord", "Cleric", "Druid"]));
        classes.observe_cast(1, Some(3), &strs(&["Cleric", "Paladin", "Shaman"]));
        refresh_loadouts(&mut records, &classes, 1);
        assert_eq!(
            records[0].loadout,
            strs(&["Enchanter", "Wizard"]),
            "narrowed to Cleric on 2 visits now -- still not proof by itself"
        );

        // why: a 3rd, distinct visit finally corroborates it, backfilling
        // visit 2's record retroactively
        classes.observe_cast(1, Some(4), &strs(&["Wizard"]));
        classes.observe_cast(1, Some(4), &strs(&["Enchanter"]));
        classes.observe_cast(1, Some(4), &strs(&["Beastlord", "Cleric", "Druid"]));
        classes.observe_cast(1, Some(4), &strs(&["Cleric", "Paladin", "Shaman"]));
        refresh_loadouts(&mut records, &classes, 1);
        assert_eq!(records[0].loadout, strs(&["Cleric", "Enchanter", "Wizard"]));
    }

    /// why: a real zone change must never backfill from another visit's evidence
    #[test]
    fn a_different_visits_evidence_never_touches_this_visits_record() {
        let mut classes = ClassDetector::default();
        for v in [Some(0), Some(1)] {
            classes.observe_cast(1, v, &strs(&["Wizard"]));
        }
        classes.observe_cast(1, Some(0), &strs(&["Wizard"]));
        let mut records = vec![fake_record(&[], Some(99))]; // an unrelated, unresolved visit
        refresh_loadouts(&mut records, &classes, 1);
        assert!(
            records[0].loadout.is_empty(),
            "visit 99 has no evidence of its own -- must stay empty, not borrow visit 0's"
        );
    }
}

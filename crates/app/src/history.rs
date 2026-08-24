//! Persisted parse history: one record per closed encounter, appended as
//! fights finish. Exists so "how did this compare to past kills of the same
//! mob" outlives `Store`'s own eviction (`Store::evict_before_encounter`) --
//! the store is a bounded live working set so a multi-day tail doesn't grow
//! unbounded in memory; this is the record meant to outlive it *within one
//! run*.
//!
//! Deliberately does **not** survive an app restart: `reset` wipes this file
//! at every launch (called from `main.rs`'s `setup`, before the tail worker
//! can append anything). Every record on disk was written by whatever
//! ingest/class-detection code happened to be running in that write's
//! process -- carrying records across a restart into a build with different
//! logic is exactly how ~2,900 loadouts claiming 4-10 simultaneous classes
//! (impossible; the game caps at `classdetect::CLASS_COUNT`) ended up
//! permanently on disk after `classdetect` moved past the model that wrote
//! them, silently mixing eras of data with no way to tell which record came
//! from which. A clean parse every start costs one session's worth of
//! cross-launch comparison history; keeping it cost a stale, silently
//! self-contradicting file that only grew.
//!
//! JSON Lines, not one JSON array: appending a line is O(1) and never
//! requires reading the whole file back in just to add one record, which a
//! session running for days needs.
//!
//! `Ingest`/`ingest.rs` builds `ParseRecord`s (pure data, no I/O -- see its
//! `pending_history` field) and this module is the only thing that ever
//! touches disk for them, the same split `config.rs` and the rule pack
//! already use elsewhere in this crate.

use eqlp_session::ClassDetector;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseRecord {
    pub target: String,
    /// The full zone label as of this fight's start ("Befallen 4
    /// (Refined)", or "Unknown" before the first zone line) -- doubles as
    /// the difficulty tier readout, since `crate::zone::zone_tier` parses
    /// the tier straight out of this same string. Kept as the raw label
    /// rather than a separate base-name/tier pair so there is exactly one
    /// place this can go stale against the other. `#[serde(default)]` so a
    /// record written before this field existed still parses -- as an
    /// empty string, not a reason to drop the whole line the way a
    /// genuinely malformed one is (see `for_target`'s doc).
    #[serde(default)]
    pub zone: String,
    /// The player's confirmed classes for *this fight's own zone visit*,
    /// as of the moment this fight closed --
    /// `eqlp_session::classdetect::Detector::configuration_of_visit`
    /// queried against that visit, not "now" (see `Ingest::record_history`).
    /// Already alphabetical (the detector groups by a sorted set
    /// internally), so two fights under the same configuration always
    /// produce the same key regardless of which class happened to have
    /// more casts in either fight -- that's what makes `by_loadout` able to
    /// group them. Empty means no unambiguous recognised cast had landed
    /// yet in this visit as of this fight's close, not "no class" -- see
    /// `classdetect`'s module doc. `#[serde(default)]` for records written
    /// before this field existed.
    #[serde(default)]
    pub loadout: Vec<String>,
    /// Which zone visit this fight belongs to (`eqlp_session::context::
    /// Spans::index_at`'s own opaque index, at `start_ms`) -- what
    /// `refresh_loadouts` keys on to re-resolve `loadout` against a live
    /// `ClassDetector`'s *current* state, since `loadout` above is only a
    /// snapshot taken as of this fight's own close. `#[serde(default)]`
    /// for records written before this field existed -- never happens
    /// across a restart (this file is wiped every launch), just matching
    /// this struct's own established convention for schema growth.
    #[serde(default)]
    pub zone_visit: Option<usize>,
    pub start_ms: i64,
    pub duration_ms: i64,
    /// The player's own damage and DPS in this fight -- not the team's
    /// combined total, which would conflate multiple people's rotations
    /// into one number nobody could act on.
    pub player_damage: u64,
    pub player_dps: f64,
    /// Whether this encounter ended in a confirmed kill line, or a
    /// timeout/reset. Only ~21% of encounters get a confirmed kill in the
    /// reference log (`BACKLOG.md`) -- any comparison across records MUST
    /// filter on this, or a truncated Reset gets compared against a full
    /// kill as if they measured the same thing. `for_target` does not
    /// filter for you; `confirmed_kills_for_target` does.
    pub confirmed_kill: bool,
    /// `eqlp_store::score::ParseScore.ratio`, scoped to the player's own
    /// damage and scored against their own per-ability average against this
    /// same `target` **at this same difficulty tier** (`zone`, parsed by
    /// `crate::zone::zone_tier`) -- not an all-mobs, all-tiers average,
    /// which would blend in every other mob's different resists and every
    /// other tier's different damage taken/dealt scaling, making "did I
    /// play this well" answer a question about which mobs and which
    /// difficulty happened to get fought instead. `None`, not 0.0, when
    /// there wasn't yet a baseline to score against (typically: the first
    /// time this ability landed on this target at this tier) -- or when
    /// this record was written during history replay rather than live: the
    /// baseline query is a whole-store scan with no cheap way to bound it
    /// to a shrinking window, so computing it for every encounter a big
    /// backfill closes (thousands, against a store still growing toward
    /// millions of rows) is exactly the quadratic-shaped cost that made a
    /// big log's initial load crawl -- skipped there on purpose (see
    /// `Ingest::record_history`), since nothing could show the number
    /// before backfill finished anyway. Every record written once live
    /// tailing catches up scores normally. See
    /// `eqlp_store::score`'s module doc for the gear-modifier seam this
    /// number inherits -- it is computed with `GearModifiers::default()`
    /// (neutral) because inventory/item detection doesn't exist yet, not
    /// because gear doesn't affect it.
    ///
    /// Known imprecision, stated rather than hidden: the baseline query
    /// includes this same encounter's own hits (there is no cheap way to
    /// exclude just this encounter from an all-time-vs-this-target-tier
    /// aggregate), so it is self-diluted, most noticeably on an ability's
    /// very first-ever use against this target at this tier, where baseline
    /// and actual are identical and the ratio is trivially 1.0. It becomes
    /// a genuinely useful signal once this target-and-tier combination has
    /// enough history behind it that one more fight barely moves the mean
    /// -- which, being scoped to one target at one tier, takes longer to
    /// build up than an all-mobs or even an all-tiers baseline would.
    pub score_ratio: Option<f64>,
}

const FILE_NAME: &str = "parse_history.jsonl";

fn history_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(dir.join(FILE_NAME))
}

/// Wipes all persisted history. Called once, from `main.rs`'s `setup`,
/// before the tail worker exists to append anything -- see this module's
/// doc for why history is reset every launch rather than kept across them.
/// Best-effort and silent: a fresh install with nothing to remove yet is
/// the common case, not an error, and there's no useful recovery from a
/// failure here beyond this run starting with whatever was already on disk
/// instead of a clean slate.
///
/// Scope: `history_path` resolves under Tauri's `app_data_dir`, this app's
/// own storage -- never the player's configured `log_dir` (the game's
/// `Logs/` folder). This purges data eqlp derived from the log, never the
/// log itself; see `eqlp_source::tail`'s module doc for that harder
/// invariant, which this function has no ability to violate by
/// construction.
pub fn reset(app: &AppHandle) {
    if let Ok(path) = history_path(app) {
        let _ = std::fs::remove_file(path);
    }
}

/// Append one record. Never rewrites the file -- only ever adds a line, so
/// a crash mid-write loses at most the record being appended, not the
/// history built up before it.
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

/// Re-resolves every record's `loadout` against a live `ClassDetector`'s
/// *current* state, keyed by `zone_visit` -- supersedes each record's own
/// "as of close" snapshot with whatever's confirmed for that same visit
/// now. Always safe, never a guess: within one zone visit a confirmed
/// class is never un-confirmed, only ever added to (see `classdetect`'s
/// own module doc) -- so an earlier fight in a visit picking up a class
/// that a *later* fight's own evidence went on to confirm is exactly as
/// real as if that evidence had simply arrived a little sooner. The one
/// thing that still legitimately gives two fights different loadouts is a
/// real zone change -- a different visit, a disjoint `VisitState`
/// entirely, untouched by this. Called at read time (`get_mob_history`/
/// `get_loadout_summary`), not at write time -- `record_history` still
/// stamps the as-of-close value so a record is never *empty* to display
/// before this can run, only possibly stale until it does.
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

/// One bucket of past parses against the same target, grouped by which
/// class combination (`ParseRecord::loadout`) was active. An empty
/// `loadout` is its own bucket -- "no class recognised yet as of that
/// fight" is a real, distinct answer, not a reason to fold those records
/// into whatever bucket happens to come first.
#[derive(Debug, Clone, Serialize)]
pub struct LoadoutSummary {
    pub loadout: Vec<String>,
    pub fights: usize,
    pub confirmed_kills: usize,
    /// Mean of each record's own `player_dps`, not total damage over total
    /// duration pooled across the bucket -- same reasoning as
    /// `combat::CombatSummaryDto::dps`: one long grind in the bucket
    /// shouldn't get to dominate the number over several short, sharp
    /// fights that are just as real a sample.
    pub avg_dps: f64,
    /// Mean of `score_ratio` across whichever records in the bucket have
    /// one at all (a backfilled record often won't -- see that field's
    /// doc). `None` if none do, not 0.0.
    pub avg_score_ratio: Option<f64>,
}

/// Groups `records` by `loadout`, sorted by fight count descending -- the
/// combination the player has actually used most against this target
/// first, matching how `monsters::list_mobs` ranks by kills. Callers
/// filter to confirmed-kills-only first if that's what they want compared
/// (`ParseRecord::confirmed_kill`'s doc); this groups whatever it's given.
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

    /// The real bug this exists to fix: a fight that closed early in a
    /// zone visit, before enough evidence had accumulated, must pick up
    /// the visit's fuller confirmed set once later evidence in that *same*
    /// visit resolves it -- not stay frozen forever at whatever was known
    /// at its own close time.
    #[test]
    fn an_earlier_fights_partial_loadout_is_backfilled_by_later_evidence_in_the_same_visit() {
        let mut classes = ClassDetector::default();
        for v in [Some(0), Some(1)] {
            classes.observe_cast(1, v, &strs(&["Wizard"]));
            classes.observe_cast(1, v, &strs(&["Enchanter"]));
        }
        // Fight 1 closes here, frozen at 2/3 -- matches what
        // `Ingest::record_history` actually stamps at close time.
        classes.observe_cast(1, Some(2), &strs(&["Wizard"]));
        classes.observe_cast(1, Some(2), &strs(&["Enchanter"]));
        let mut records = vec![fake_record(&["Enchanter", "Wizard"], Some(2))];

        // Later in the *same* visit, a 3rd class resolves by elimination
        // (a second fight's own evidence, not separately modeled here --
        // only the Detector's resulting state matters for this test).
        classes.observe_cast(1, Some(2), &strs(&["Beastlord", "Cleric", "Druid"]));
        classes.observe_cast(1, Some(2), &strs(&["Cleric", "Paladin", "Shaman"]));

        refresh_loadouts(&mut records, &classes, 1);
        assert_eq!(records[0].loadout, strs(&["Cleric", "Enchanter", "Wizard"]));
    }

    /// A real zone change -- a different visit entirely -- must never
    /// backfill a record from evidence that belongs to some other visit.
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

//! why: the game's `/outputfile` dumps beside Logs -- which exist, the
//! command that writes each, and the spellbook rows nothing read before.

use eqlp_source::Millis;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// why: (kind, filename suffix, command, primary, app reads it).
/// Confirmed against real Outputfile Complete lines: Inventory,
/// Achievements, Spellbook, Factions, and missingspells (which has no
/// fixed suffix at all -- see `file_matches`). The rest are the
/// command's own names and stay unconfirmed until one is written.
pub const KINDS: &[(&str, &str, &str, bool, bool)] = &[
    (
        "inventory",
        "-Inventory.txt",
        "/outputfile inventory",
        true,
        true,
    ),
    (
        "achievements",
        "-Achievements.txt",
        "/outputfile achievements",
        true,
        true,
    ),
    (
        "spellbook",
        "-Spellbook.txt",
        "/outputfile spellbook",
        true,
        true,
    ),
    (
        "factions",
        "-Factions.txt",
        "/outputfile faction",
        true,
        false,
    ),
    ("guild", "-Guild.txt", "/outputfile guild", false, false),
    (
        "guildbank",
        "-GuildBank.txt",
        "/outputfile guildbank",
        false,
        false,
    ),
    (
        "guildhall",
        "-GuildHall.txt",
        "/outputfile guildhall",
        false,
        false,
    ),
    ("raid", "-Raid.txt", "/outputfile raid", false, false),
    (
        "realestate",
        "-RealEstate.txt",
        "/outputfile realestate",
        false,
        false,
    ),
    // why: the game rejects a bare `recipes` -- "Usage: /outputfile
    // recipes [alchemy | baking | ...]", seen in the real log
    (
        "recipes",
        "-Recipes.txt",
        "/outputfile recipes <tradeskill>",
        false,
        false,
    ),
    // why: writes "<Class> Spells_<server>-<date>-<time>.txt", a new file
    // per run with no fixed suffix -- confirmed twice in the real log
    (
        "missingspells",
        " Spells_",
        "/outputfile missingspells",
        false,
        false,
    ),
];

/// why: every kind but one is a plain suffix; missingspells stamps the
/// date into the name, so it needs its own test rather than a fake suffix
fn file_matches(kind: &str, suffix: &str, name: &str) -> bool {
    match kind {
        "missingspells" => name.contains(suffix) && name.ends_with(".txt"),
        _ => name.ends_with(suffix),
    }
}

/// why: built the same way for the on-demand command and the pushed
/// event, so the two can never disagree about the log's own row
pub fn log_row(file: Option<&str>, watching: bool) -> SyncRowDto {
    let (status, detail) = match (file, watching) {
        (Some(f), true) => ("ok", format!("Tailing {f}.")),
        (Some(f), false) => (
            "fix",
            format!("{f} found but not being watched. Check the folder setting."),
        ),
        (None, _) => (
            "missing",
            "No log file found. Turn logging on in game with /log on.".to_string(),
        ),
    };
    SyncRowDto {
        kind: "log".to_string(),
        label: "Combat log".to_string(),
        primary: true,
        status: status.to_string(),
        file: file.map(str::to_string),
        modified_ms: None,
        dumped_at_ms: None,
        detail,
        command: Some("/log on".to_string()),
    }
}

/// why: which kind a dump filename belongs to -- the Outputfile Complete
/// line states only the name, and `Ingest` keys its dump clock by kind
pub fn kind_of(file: &str) -> Option<&'static str> {
    KINDS
        .iter()
        .find(|(kind, suffix, _, _, _)| file_matches(kind, suffix, file))
        .map(|(kind, ..)| *kind)
}

#[derive(Debug, Clone, Serialize)]
pub struct OutputfileDto {
    pub kind: String,
    pub command: String,
    pub file: Option<String>,
    pub modified_ms: Option<Millis>,
}

fn modified_ms(path: &Path) -> Option<Millis> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    Millis::try_from(d.as_millis()).ok()
}

fn newest_of(base_dir: &Path, kind: &str, suffix: &str) -> Option<PathBuf> {
    std::fs::read_dir(base_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| file_matches(kind, suffix, n))
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

/// why: newest dump per kind -- the Import menu's status rows
pub fn list(base_dir: &Path) -> Vec<OutputfileDto> {
    KINDS
        .iter()
        .filter(|(.., reads)| *reads)
        .map(|(kind, suffix, command, _, _)| {
            let path = newest_of(base_dir, kind, suffix);
            OutputfileDto {
                kind: kind.to_string(),
                command: command.to_string(),
                file: path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned()),
                modified_ms: path.as_deref().and_then(modified_ms),
            }
        })
        .collect()
}

/// why: every class file of one character (`<Char>_<server>-<CLASS>-Spellbook.txt`,
/// rows `level<TAB>name`); the file's own mtime stands in for first seen
pub fn spellbook_spells(base_dir: &Path, character: Option<&str>) -> Vec<(String, Millis)> {
    let prefix = character.map(|c| format!("{c}_"));
    let Ok(entries) = std::fs::read_dir(base_dir) else {
        return Vec::new();
    };
    let mut seen: HashMap<String, Millis> = HashMap::new();
    for e in entries.filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().into_owned();
        let mine = prefix.as_ref().is_none_or(|p| name.starts_with(p.as_str()));
        if !name.ends_with("-Spellbook.txt") || !mine {
            continue;
        }
        let ts = modified_ms(&e.path()).unwrap_or(0);
        let Ok(text) = std::fs::read_to_string(e.path()) else {
            continue;
        };
        for line in text.lines() {
            let Some((level, spell)) = line.split_once('\t') else {
                continue;
            };
            let spell = spell.trim();
            if level.trim().parse::<u16>().is_err() || spell.is_empty() {
                continue;
            }
            seen.entry(spell.to_string()).or_insert(ts);
        }
    }
    seen.into_iter().collect()
}

/// why: one health row -- `status` is the colour, `detail` the sentence
#[derive(Debug, Clone, Serialize)]
pub struct SyncRowDto {
    pub kind: String,
    pub label: String,
    pub primary: bool,
    /// "ok" | "fix" | "missing" | "optional" | "inferred"
    pub status: String,
    pub file: Option<String>,
    pub modified_ms: Option<Millis>,
    /// why: log clock of the newest Outputfile Complete for this kind
    pub dumped_at_ms: Option<Millis>,
    pub detail: String,
    pub command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncCheckDto {
    /// "ok" | "fix" | "missing"
    pub overall: String,
    pub rows: Vec<SyncRowDto>,
}

/// why: the command's own one-word names read badly as headings
fn label_for(kind: &str) -> String {
    match kind {
        "guildbank" => "Guild bank",
        "guildhall" => "Guild hall",
        "realestate" => "Real estate",
        "inventory" => "Inventory",
        "achievements" => "Achievements",
        "spellbook" => "Spellbook",
        "factions" => "Factions",
        "guild" => "Guild",
        "raid" => "Raid",
        "recipes" => "Recipes",
        "missingspells" => "Missing spells",
        other => other,
    }
    .to_string()
}

/// why: `LocalTs` is the log's wall clock with no zone attached, so a
/// logged dump time is local-read-as-UTC; a file mtime is true epoch.
/// The game writes both at the same instant, so a matched pair gives the
/// offset without a timezone database. Median over the pairs, so one odd
/// file can't skew it.
fn local_offset_ms(pairs: &[(Millis, Millis)]) -> Option<Millis> {
    let mut d: Vec<Millis> = pairs
        .iter()
        .map(|(logged, mtime)| mtime - logged)
        // why: a real zone offset, never a stale file masquerading as one
        .filter(|d| d.abs() <= 14 * 3_600_000)
        .collect();
    // why: one pair is circular -- the offset would absorb whatever
    // staleness it was meant to reveal, so `unsure` could never fire
    if d.len() < 2 {
        return None;
    }
    d.sort_unstable();
    Some(d[d.len() / 2])
}

/// why: log seconds vs filesystem sub-seconds, and the two writes are
/// not atomic with each other
const DUMP_MATCH_TOLERANCE_MS: Millis = 5_000;

/// why: a dump is usable only while the log still covers the span since
/// it was written. No Outputfile Complete for it in the replayed log
/// means nothing can say what changed, so it needs re-running -- this is
/// an accountability test, not a count of changes.
pub fn sync_check(
    dumps: &HashMap<String, Millis>,
    base_dir: Option<&Path>,
    log_row: SyncRowDto,
) -> SyncCheckDto {
    let mut rows = vec![log_row];
    let found: Vec<(&str, Option<PathBuf>, Option<Millis>)> = KINDS
        .iter()
        .map(|(kind, suffix, ..)| {
            let path = base_dir.and_then(|d| newest_of(d, kind, suffix));
            let mtime = path.as_deref().and_then(modified_ms);
            (*kind, path, mtime)
        })
        .collect();
    let pairs: Vec<(Millis, Millis)> = found
        .iter()
        .filter_map(|(kind, _, mtime)| Some((dumps.get(*kind).copied()?, (*mtime)?)))
        .collect();
    let offset = local_offset_ms(&pairs);
    for ((kind, suffix, command, primary, reads), (_, path, mtime)) in KINDS.iter().zip(&found) {
        let _ = suffix;
        let path = path.clone();
        let dumped = dumps.get(*kind).copied();
        // why: the file put on the log's own clock, so "is this the dump
        // the log saw?" is a like-for-like comparison
        let file_on_log_clock = match (mtime, offset) {
            (Some(m), Some(off)) => Some(m - off),
            _ => None,
        };
        let (status, detail) = match (path.is_some(), dumped) {
            (true, Some(d))
                if file_on_log_clock.is_some_and(|f| f > d + DUMP_MATCH_TOLERANCE_MS) =>
            {
                (
                    "unsure",
                    format!(
                        "A newer dump was written than this log recorded, so what \
                     changed since it can't be pinned down. Run {command} again."
                    ),
                )
            }
            (false, _) if !reads && !primary => (
                "optional",
                "Never written. Nothing here needs it.".to_string(),
            ),
            (false, _) => ("missing", format!("No dump found. Run {command} in game.")),
            (true, Some(_)) => (
                "ok",
                "Dump found, and the log covers everything since it.".to_string(),
            ),
            (true, None) => (
                "fix",
                format!(
                    "Dump found, but this log never recorded it being written, \
                     so changes since it can't be accounted for. Run {command} again."
                ),
            ),
        };
        rows.push(SyncRowDto {
            kind: kind.to_string(),
            label: label_for(kind),
            primary: *primary,
            status: status.to_string(),
            file: path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned()),
            modified_ms: *mtime,
            // why: handed over as true epoch so the UI can render it
            // directly -- the raw log clock would show the zone offset
            dumped_at_ms: match (dumped, offset) {
                (Some(d), Some(off)) => Some(d + off),
                _ => dumped,
            },
            detail,
            command: Some(command.to_string()),
        });
    }
    let worst = |s: &str| match s {
        "missing" => 3,
        "fix" => 2,
        "unsure" => 1,
        _ => 0,
    };
    let overall = rows
        .iter()
        .filter(|r| r.primary)
        .map(|r| r.status.as_str())
        .max_by_key(|s| worst(s))
        .map(|s| match worst(s) {
            3 => "missing",
            2 => "fix",
            1 => "unsure",
            _ => "ok",
        })
        .unwrap_or("ok");
    SyncCheckDto {
        overall: overall.to_string(),
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_row() -> SyncRowDto {
        SyncRowDto {
            kind: "log".into(),
            label: "Combat log".into(),
            primary: true,
            status: "ok".into(),
            file: None,
            modified_ms: None,
            dumped_at_ms: None,
            detail: String::new(),
            command: None,
        }
    }

    /// why: the whole rule -- a dump the log never recorded is one whose
    /// changes since cannot be accounted for, however recent the file is
    #[test]
    fn a_dump_the_log_never_recorded_needs_rerunning_even_when_the_file_is_fresh() {
        let d = std::env::temp_dir().join(format!("eqlp-sync-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Manipulator_rivervale-Inventory.txt"), "x").unwrap();
        std::fs::write(d.join("Manipulator_rivervale-Achievements.txt"), "x").unwrap();

        let mut dumps = HashMap::new();
        dumps.insert("achievements".to_string(), 1_700_000_000_000i64);
        let out = sync_check(&dumps, Some(&d), log_row());
        let by = |k: &str| {
            out.rows
                .iter()
                .find(|r| r.kind == k)
                .unwrap_or_else(|| panic!("{k} row"))
        };
        assert_eq!(by("achievements").status, "ok", "file + a logged dump line");
        assert_eq!(
            by("inventory").status,
            "fix",
            "file on disk but no Outputfile Complete for it in this log"
        );
        assert_eq!(by("spellbook").status, "missing", "no file at all");
        assert_eq!(by("guild").status, "optional", "secondary, never written");
        assert_eq!(
            by("missingspells").status,
            "optional",
            "a real dump kind, just not one anything needs"
        );
        assert_eq!(out.overall, "missing", "worst primary row wins");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// why: the log clock carries no zone, so comparing it to a file
    /// mtime is wrong by the local offset -- measured -4h on the real
    /// install. Derived from matched pairs, and refused when a single
    /// pair would just absorb the staleness it is meant to expose.
    #[test]
    fn the_zone_offset_is_derived_from_pairs_and_refused_when_circular() {
        let h = 3_600_000i64;
        assert_eq!(
            local_offset_ms(&[(1_000, 1_000 - 4 * h)]),
            None,
            "one pair is circular -- it would absorb any staleness"
        );
        assert_eq!(
            local_offset_ms(&[(0, -4 * h), (500, 500 - 4 * h)]),
            Some(-4 * h),
            "two agreeing pairs give the real offset"
        );
        assert_eq!(
            local_offset_ms(&[(0, -4 * h), (0, -4 * h), (0, 99 * h)]),
            Some(-4 * h),
            "median ignores an outlier, and beyond 14h is rejected outright"
        );
        assert_eq!(local_offset_ms(&[]), None, "nothing to calibrate from");
    }

    #[test]
    fn lists_the_newest_per_kind_and_reads_one_characters_spellbook_rows() {
        let d = std::env::temp_dir().join(format!("eqlp-outputfiles-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("Manipulator_rivervale-Inventory.txt"),
            "Location\tName\n",
        )
        .unwrap();
        std::fs::write(
            d.join("Manipulator_rivervale-WIZ-Spellbook.txt"),
            "1\tBlast of Cold\r\n4\tFrost Bolt\r\nno tab here\r\n",
        )
        .unwrap();
        std::fs::write(
            d.join("Manipulator_rivervale-ENC-Spellbook.txt"),
            "1\tBlast of Cold\r\n2\tMesmerize\r\n",
        )
        .unwrap();
        std::fs::write(d.join("Alt_rivervale-BRD-Spellbook.txt"), "1\tChant\r\n").unwrap();

        let listed = list(&d);
        assert_eq!(listed.len(), 3);
        assert_eq!(
            listed[0].file.as_deref(),
            Some("Manipulator_rivervale-Inventory.txt")
        );
        assert!(listed[0].modified_ms.is_some());
        assert_eq!(listed[1].file, None, "no achievements dump written");
        assert_eq!(listed[1].command, "/outputfile achievements");

        let mut spells: Vec<String> = spellbook_spells(&d, Some("Manipulator"))
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        spells.sort();
        assert_eq!(
            spells,
            ["Blast of Cold", "Frost Bolt", "Mesmerize"],
            "both class files, deduped, the alt's file left out"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}

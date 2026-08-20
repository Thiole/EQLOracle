//! Read-side diagnostics for the Debug module -- not something a normal
//! user needs day to day, but a direct window into what `Ingest` actually
//! recorded, for verifying something like zone tagging against real data
//! instead of trusting it blind. Named `debugview`, not `debug`, so it
//! doesn't read as this crate's own debug-logging machinery (it has
//! none) at a glance.

use crate::ingest::Ingest;
use eqlp_source::Millis;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DebugEncounterDto {
    pub id: u32,
    pub target: String,
    pub start_ms: Millis,
    pub duration_ms: Millis,
    /// The exact `zone.enter` label stamped onto this encounter at open
    /// time (`Store::Encounter::zone`). `None` only for a fight that
    /// opened before this session's log history has any `zone.enter`
    /// line at all (the "Unknown" bucket elsewhere in this app).
    pub raw_zone: Option<String>,
    /// What `raw_zone` resolved to against `zonedata::zones()`
    /// (`Ingest::cached_wiki_zone`) -- a `zonedata::Zone::id`, not a
    /// display name (see that method's doc for why an id: an exact `==`
    /// at every call site, and directly eyeball-comparable against a
    /// zone page's own `Zone::id`, which is exactly what this column is
    /// for). `None` here while `raw_zone` is `Some` means the match
    /// genuinely failed -- `zone_key` and `ZONE_ALIASES` both missed it --
    /// not that resolution hasn't run yet; `current_zone` always resolves
    /// before an encounter can exist to be queried at all. That
    /// distinction (genuine miss vs. no zone known yet) is exactly what
    /// this table exists to make visible.
    pub resolved_zone_id: Option<String>,
    pub tier: u8,
    /// What `classdetect` currently believes the player was playing during
    /// this fight's own zone visit -- same query `Ingest::record_history`
    /// makes for the History pane's loadout column, exposed per-fight here
    /// so a real session can be eyeballed fight by fight. Empty means
    /// nothing's confirmed for this visit yet, not "no classes" -- see
    /// `classdetect`'s own module doc for what counts as evidence.
    pub player_classes: Vec<String>,
}

/// The most recent `limit` encounters, newest first, with exactly what
/// they're tagged with -- the Debug module's one table. Same reverse-
/// scan-and-stop-early shape as `combat::list_zone_encounters`, for the
/// same reason: bounded work regardless of session length, not a scan of
/// every fight ever recorded just to show the most recent handful.
pub fn list_debug_encounters(ing: &Ingest, limit: usize) -> Vec<DebugEncounterDto> {
    let now = ing.now_ms();
    let you = ing.store.names.get("You");
    ing.store
        .encounters
        .iter()
        .rev()
        .take(limit)
        .map(|e| DebugEncounterDto {
            id: e.id.0,
            target: ing.store.name(e.target).to_string(),
            start_ms: e.start_ms,
            duration_ms: e.duration_ms(now).max(0),
            raw_zone: e.zone.map(|z| ing.store.name(z).to_string()),
            resolved_zone_id: e
                .zone
                .and_then(|z| ing.cached_wiki_zone(z))
                .map(str::to_string),
            tier: ing.store.tier.get(e.first as usize).copied().unwrap_or(0),
            player_classes: you
                .map(|y| ing.classes.configuration_of_visit(y.0, ing.zone.index_at(e.start_ms)))
                .unwrap_or_default(),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct UnmatchedShapeDto {
    /// The collapsed template (`eqlp_core::shape`, aggressive mode) --
    /// variable text (names, numbers) replaced with a placeholder, so
    /// every real line that differs only in who/how-much collapses to
    /// one row here instead of one per line.
    pub shape: String,
    pub count: u64,
    /// One real, unmodified line this shape matched -- the first one
    /// seen, kept exactly as the log wrote it, for writing a new rule
    /// pattern against.
    pub example: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnmatchedCoverageDto {
    /// Highest count first -- the biggest remaining coverage gaps sort to
    /// the top, same as `eqlp coverage --top N`'s own output.
    pub shapes: Vec<UnmatchedShapeDto>,
    pub distinct_shapes: usize,
    /// Real unmatched *lines*, not distinct shapes, dropped once the
    /// shape cap was already full -- see `ingest::Ingest::unmatched_
    /// shapes_overflow`'s own doc.
    pub shapes_overflow: u64,
    pub unmatched_total: u64,
    pub total_lines: u64,
}

/// The Debug module's "Unparsed" tab: every unmatched-line shape seen
/// this session (live tail and backfilled history both -- see `ingest::
/// Ingest::unmatched_shapes`'s own doc), ranked by how often it fired.
/// This is the same clustering the `eqlp coverage`/`eqlp shapes` CLI
/// commands do offline against a log file, just kept live in the running
/// app so closing a coverage gap doesn't require pulling the log out and
/// running a separate tool first.
pub fn unmatched_coverage(ing: &Ingest, top: usize) -> UnmatchedCoverageDto {
    let shapes = ing
        .unmatched_shapes_top(top)
        .into_iter()
        .map(|(shape, stat)| UnmatchedShapeDto {
            shape: String::from_utf8_lossy(shape).into_owned(),
            count: stat.count,
            example: String::from_utf8_lossy(&stat.example).into_owned(),
        })
        .collect();
    UnmatchedCoverageDto {
        shapes,
        distinct_shapes: ing.unmatched_shapes_distinct(),
        shapes_overflow: ing.unmatched_shapes_overflow(),
        unmatched_total: ing.counts.unmatched,
        total_lines: ing.counts.total,
    }
}

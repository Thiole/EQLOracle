//! why: read-side diagnostics for the Debug module -- a window into what
//! `Ingest` actually recorded, to verify zone tagging against real data

use crate::ingest::Ingest;
use eqlp_source::Millis;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DebugEncounterDto {
    pub id: u32,
    pub target: String,
    pub start_ms: Millis,
    pub duration_ms: Millis,
    /// why: raw `zone.enter` label at open time; None means no line yet
    pub raw_zone: Option<String>,
    /// why: `raw_zone` resolved to a `zonedata::Zone::id`; None while
    /// `raw_zone` is Some means the alias match genuinely failed
    pub resolved_zone_id: Option<String>,
    pub tier: u8,
    /// why: classdetect's belief for this zone visit; empty means unconfirmed
    pub player_classes: Vec<String>,
}

/// why: newest-first, bounded scan like `combat::list_zone_encounters`
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
                .map(|y| {
                    ing.classes
                        .configuration_of_visit(y.0, ing.zone.index_at(e.start_ms))
                })
                .unwrap_or_default(),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct UnmatchedShapeDto {
    /// why: collapsed template, variable text placeholdered so lines merge
    pub shape: String,
    pub count: u64,
    /// why: first real unmodified line seen, for writing a new rule against
    pub example: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnmatchedCoverageDto {
    /// why: highest count first, same as `eqlp coverage --top N`
    pub shapes: Vec<UnmatchedShapeDto>,
    pub distinct_shapes: usize,
    /// why: unmatched lines dropped once the shape cap was full
    pub shapes_overflow: u64,
    pub unmatched_total: u64,
    pub total_lines: u64,
}

/// why: "Unparsed" tab -- same clustering as `eqlp coverage`/`shapes`
/// CLI, kept live so closing a gap needs no separate tool run
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

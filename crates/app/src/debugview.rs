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
    /// why: false marks someone else's fight -- parsed for clean data,
    /// filtered out of Combat/overlay; Debug is where it stays visible
    pub involves_you: bool,
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
            involves_you: e.involves_you,
            player_classes: you
                .map(|y| {
                    ing.classes
                        .configuration_of_visit(y.0, ing.unit_at(e.start_ms))
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

#[derive(Debug, Clone, Serialize)]
pub struct PartyMemberDto {
    pub name: String,
    /// why: "you" (the log owner) | "joined" (an explicit roster line --
    /// join/leave lines, group chat, an accepted invite) | "strong"
    /// (Quick Buff corroborated) | "weak" (shared-target damage,
    /// session-gated) -- see eqlp_session::group's own doc
    pub via: &'static str,
    /// why: only meaningful for "weak" -- how many real, gap-separated
    /// occasions of shared-target evidence this crossed
    pub sessions: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameStateDto {
    pub party: Vec<PartyMemberDto>,
    /// why: everyone ever proven a real player (chat channels, roster
    /// lines) across the whole log -- a permanent identity fact, NOT
    /// party membership. Kept as a count: the old party view listed all
    /// of these as members, which is how a 245MB log "grouped" the log
    /// owner with 3,800 strangers.
    pub known_players: usize,
    /// why: "You"'s own current class configuration, as of right now --
    /// same call `list_debug_encounters` makes per-encounter, just at "now"
    pub your_classes: Vec<String>,
    /// why: directly observed from real level.up lines, not a per-class estimate
    pub your_level: Option<u8>,
}

/// why: "Game State" debug tab -- a compact, live dump of what the
/// backend currently believes, not a polished feature. Deliberately a
/// scratchpad: whatever in-progress backend state (GroupTracker today,
/// more later) is worth eyeballing without a dedicated UI for it yet.
/// The party list is GroupTracker's CURRENT roster only -- "ever proven
/// a player" is identity, not membership, and stays a count.
pub fn game_state(ing: &Ingest) -> GameStateDto {
    let now = ing.now_ms();
    let mut party = vec![PartyMemberDto {
        name: "You".to_string(),
        via: "you",
        sessions: 0,
    }];

    // why: GroupTracker's roster, resolved through display_name for real
    // casing -- keys are fold_key'd. Sorted by channel certainty then
    // name so the dump reads stably between polls.
    let mut members = ing.groups.current_members(now);
    members.sort_by(|a, b| (a.2 as u8, &a.0).cmp(&(b.2 as u8, &b.0)));
    for (key, sessions, via, _last_ms) in members {
        let display = ing.encounters.entities.display_name(&key).to_string();
        if display.eq_ignore_ascii_case("you") {
            continue;
        }
        party.push(PartyMemberDto {
            name: display,
            via: via.name(),
            sessions,
        });
    }

    let your_classes = ing
        .store
        .names
        .get("You")
        .map(|y| ing.classes.configuration_of_visit(y.0, ing.unit_at(now)))
        .unwrap_or_default();

    GameStateDto {
        party,
        known_players: ing.encounters.entities.players().count(),
        your_classes,
        your_level: ing.levels.latest(),
    }
}

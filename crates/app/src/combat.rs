//! Read-side queries over `Ingest` for the Combat module: which zone visits
//! exist, which encounters are in one, and the ability breakdown for a
//! selection -- one encounter, every encounter in a zone visit, or
//! everything parsed so far.
//!
//! No parsing happens here. Every query runs against `Store`, which already
//! holds everything `ingest::Ingest::route` has classified -- nothing is
//! reparsed to answer these.

use crate::ingest::Ingest;
use eqlp_session::{series as bucket_series, Cause, Kind};
use eqlp_source::Millis;
use eqlp_store::{by_ability, by_actor, dps_window, tag, total, AbilityRow, EncounterId, EventKind, Filter, Sym};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct ZoneVisitDto {
    /// `Spans` index for this visit, or `None` for the "unknown" bucket --
    /// encounters seen before the first zone line (attaching mid-session).
    /// See `docs/design/context.md`, "Unknown is a bucket, not an error".
    pub index: Option<usize>,
    pub label: String,
    pub fight_count: usize,
    /// The most recent visit with no successor -- the zone the player is
    /// presumably still in.
    pub current: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EncounterDto {
    pub id: u32,
    pub target: String,
    /// Every entity seen in this fight, not just the anchor `target` label
    /// -- a multi-mob pull holds several. See `Ingest::entities_by_enc`.
    pub entities: Vec<String>,
    pub start_ms: Millis,
    pub end_ms: Option<Millis>,
    pub duration_ms: Millis,
    pub total_damage: u64,
    pub dps: f64,
    pub slain: bool,
    pub open: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AbilityRowDto {
    pub ability: String,
    pub tags: Vec<&'static str>,
    pub total: u64,
    pub hits: u64,
    pub min: u64,
    pub max: u64,
    pub crits: u64,
    pub dps: f64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CombatSummaryDto {
    pub fight_count: usize,
    pub total_damage: u64,
    pub duration_ms: Millis,
    pub dps: f64,
    pub abilities: Vec<AbilityRowDto>,
}

fn zone_visit_of(ing: &Ingest, start_ms: Millis) -> Option<usize> {
    ing.zone.index_at(start_ms)
}

/// `zone_visit` as it crosses IPC: `None` means "no filter, everything";
/// otherwise a visit index, with `-1` standing in for the "Unknown" bucket
/// (`None` on the `Option<usize>` side). A plain `Option<usize>` can't
/// distinguish "no filter" from "filter to the one bucket with no zone" --
/// both would be `None` -- so this is `Option<i64>` instead, and `-1` is the
/// one value `usize` never produces, making it a safe sentinel rather than
/// a colliding one.
fn matches_visit(ing: &Ingest, start_ms: Millis, want: Option<i64>) -> bool {
    match want {
        None => true,
        Some(-1) => zone_visit_of(ing, start_ms).is_none(),
        Some(n) => zone_visit_of(ing, start_ms) == usize::try_from(n).ok(),
    }
}

pub fn list_zone_visits(ing: &Ingest) -> Vec<ZoneVisitDto> {
    let mut counts: HashMap<Option<usize>, usize> = HashMap::new();
    for e in &ing.store.encounters {
        *counts.entry(zone_visit_of(ing, e.start_ms)).or_insert(0) += 1;
    }
    let last_zone_index = if ing.zone.is_empty() { None } else { Some(ing.zone.len() - 1) };

    let mut out: Vec<ZoneVisitDto> = counts
        .into_iter()
        .map(|(zi, fight_count)| {
            let label = match zi {
                Some(i) => ing.zone.iter().nth(i).map(|(_, l)| l.to_string()).unwrap_or_else(|| "?".to_string()),
                None => "Unknown".to_string(),
            };
            ZoneVisitDto { index: zi, label, fight_count, current: zi.is_some() && zi == last_zone_index }
        })
        .collect();

    // Newest visit first; the pre-first-zone-line "Unknown" bucket sorts last.
    out.sort_by(|a, b| match (a.index, b.index) {
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    out
}

pub fn list_encounters(ing: &Ingest, zone_visit: Option<i64>) -> Vec<EncounterDto> {
    let now = ing.now_ms();
    let mut out: Vec<EncounterDto> = ing
        .store
        .encounters
        .iter()
        .filter(|e| matches_visit(ing, e.start_ms, zone_visit))
        .map(|e| {
            let dur = e.duration_ms(now).max(0);
            let dmg = total(&ing.store, &Filter::encounter(e.id).damage());
            EncounterDto {
                id: e.id.0,
                target: ing.store.name(e.target).to_string(),
                entities: ing.entities_by_enc.get(&e.id).cloned().unwrap_or_default(),
                start_ms: e.start_ms,
                end_ms: e.end_ms,
                duration_ms: dur,
                total_damage: dmg,
                dps: if dur > 0 { dmg as f64 / (dur as f64 / 1000.0) } else { 0.0 },
                slain: e.slain,
                open: e.is_open(),
            }
        })
        .collect();
    out.sort_by(|a, b| b.start_ms.cmp(&a.start_ms));
    out
}

/// Every encounter id in a selection: one specific fight if `encounter_id`
/// is given, otherwise every fight in `zone_visit`, otherwise every fight
/// parsed so far. Shared by `summarize` and `list_allies` -- both aggregate
/// over "the current selection", just grouped by a different key.
fn resolve_ids(ing: &Ingest, zone_visit: Option<i64>, encounter_id: Option<u32>) -> Vec<EncounterId> {
    if let Some(eid) = encounter_id {
        vec![EncounterId(eid)]
    } else {
        ing.store.encounters.iter().filter(|e| matches_visit(ing, e.start_ms, zone_visit)).map(|e| e.id).collect()
    }
}

/// Aggregates one encounter, every encounter in a zone visit, or every
/// encounter parsed so far. `encounter_id` wins if given; otherwise
/// `zone_visit`; otherwise everything. `actor`, if given, narrows to one
/// ally's own abilities -- the drill-down from `list_allies`.
///
/// Merging several encounters' ability breakdowns client-side (one
/// `by_ability` call per encounter, summed) rather than teaching
/// `eqlp-store`'s `Filter` to accept a set of encounter ids: a zone visit is
/// a handful of fights, so this is a handful of cheap calls
/// (`docs/design/store.md` measures a single-encounter `by_ability` at
/// 39µs), and it leaves the store's query contract exactly as documented
/// rather than extending it for one caller.
pub fn summarize(ing: &Ingest, zone_visit: Option<i64>, encounter_id: Option<u32>, actor: Option<&str>) -> CombatSummaryDto {
    let now = ing.now_ms();
    let ids = resolve_ids(ing, zone_visit, encounter_id);
    if ids.is_empty() {
        return CombatSummaryDto::default();
    }
    let actor_sym = actor.and_then(|n| ing.store.names.get(n));

    let mut total_damage = 0u64;
    let mut duration_ms: Millis = 0;
    let mut merged: HashMap<eqlp_store::AbilityId, AbilityRow> = HashMap::new();

    for &id in &ids {
        let mut f = Filter::encounter(id).damage();
        if let Some(s) = actor_sym {
            f = f.by(s);
        }
        total_damage += total(&ing.store, &f);
        if let Some(e) = ing.store.encounter(id) {
            duration_ms += e.duration_ms(now).max(0);
        }
        for row in by_ability(&ing.store, &f) {
            let acc = merged.entry(row.ability).or_insert_with(|| AbilityRow {
                ability: row.ability,
                tags: row.tags,
                total: 0,
                hits: 0,
                min: u64::MAX,
                max: 0,
                full_power: 0,
                crits: 0,
                flags: 0,
            });
            acc.total += row.total;
            acc.hits += row.hits;
            acc.min = acc.min.min(row.min);
            acc.max = acc.max.max(row.max);
            acc.full_power += row.full_power;
            acc.crits += row.crits;
            acc.flags |= row.flags;
        }
    }

    let mut rows: Vec<AbilityRow> = merged.into_values().collect();
    for r in &mut rows {
        if r.min == u64::MAX {
            r.min = 0;
        }
    }
    rows.sort_by(|a, b| b.total.cmp(&a.total));

    // A floor under the divisor, not under the reported DPS: an aggregate
    // with a handful of ms of combat should read as a huge, honest number,
    // not a silently suppressed one.
    let dur_secs = (duration_ms.max(0) as f64 / 1000.0).max(0.001);
    let abilities = rows
        .into_iter()
        .map(|r| AbilityRowDto {
            ability: ing.store.ability_name(r.ability).to_string(),
            tags: tag::names(r.tags),
            total: r.total,
            hits: r.hits,
            min: r.min,
            max: r.max,
            crits: r.crits,
            dps: r.total as f64 / dur_secs,
            pct: if total_damage > 0 { 100.0 * r.total as f64 / total_damage as f64 } else { 0.0 },
        })
        .collect();

    CombatSummaryDto { fight_count: ids.len(), total_damage, duration_ms, dps: total_damage as f64 / dur_secs, abilities }
}

// ---------------------------------------------------------------- allies

#[derive(Debug, Clone, Serialize)]
pub struct AllyDto {
    pub name: String,
    pub is_player: bool,
    pub is_pet: bool,
    pub total: u64,
    pub hits: u64,
    pub dps: f64,
    pub pct: f64,
}

/// Damage dealers in the current selection, sorted by total descending --
/// the Combat module's primary view. Click one (`summarize` with `actor`
/// set) to drill into their own ability breakdown.
///
/// Excludes whichever entity anchors each encounter's `target` label. The
/// log gives no clean enemy/ally flag -- `Kind` only distinguishes
/// `Player`/`Pet` from `Unproven`, and an unspoken NPC and an unspoken
/// player look identical -- but the fight's own target is reliably the
/// thing being fought, in both a kill and a timeout. A multi-mob pull's
/// *other* mobs are a known gap this doesn't cover; see
/// `Ingest::entities_by_enc`'s doc comment.
pub fn list_allies(ing: &Ingest, zone_visit: Option<i64>, encounter_id: Option<u32>) -> Vec<AllyDto> {
    let ids = resolve_ids(ing, zone_visit, encounter_id);
    if ids.is_empty() {
        return Vec::new();
    }

    let mut exclude: HashSet<Sym> = HashSet::new();
    let mut total_damage = 0u64;
    let mut acc: HashMap<Sym, (u64, u64)> = HashMap::new();

    for &id in &ids {
        if let Some(e) = ing.store.encounter(id) {
            exclude.insert(e.target);
        }
        let f = Filter::encounter(id).damage();
        total_damage += total(&ing.store, &f);
        for (sym, dmg, hits) in by_actor(&ing.store, &f) {
            let e = acc.entry(sym).or_insert((0, 0));
            e.0 += dmg;
            e.1 += hits;
        }
    }

    let now = ing.now_ms();
    let duration_ms: Millis = ids.iter().filter_map(|&id| ing.store.encounter(id)).map(|e| e.duration_ms(now).max(0)).sum();
    let dur_secs = (duration_ms.max(0) as f64 / 1000.0).max(0.001);

    let mut out: Vec<AllyDto> = acc
        .into_iter()
        .filter(|(sym, _)| !exclude.contains(sym))
        .map(|(sym, (dmg, hits))| {
            let name = ing.store.name(sym).to_string();
            let kind = ing.encounters.entities.kind(&name);
            AllyDto {
                is_player: kind == Kind::Player,
                is_pet: kind == Kind::Pet,
                total: dmg,
                hits,
                dps: dmg as f64 / dur_secs,
                pct: if total_damage > 0 { 100.0 * dmg as f64 / total_damage as f64 } else { 0.0 },
                name,
            }
        })
        .collect();
    out.sort_by(|a, b| b.total.cmp(&a.total));
    out
}

// ---------------------------------------------------------------- timeline

/// Aim for around this many buckets across a fight regardless of its
/// length, so a 10-second skirmish and a 10-minute grind both render as a
/// readable strip of bars rather than one bucket or ten thousand.
const TARGET_BUCKETS: Millis = 60;
const MIN_BUCKET_MS: Millis = 1000;

/// A believable "current DPS" window for the click-to-inspect readout --
/// long enough not to be one lucky hit, short enough to feel like "right
/// now" rather than the whole fight's average.
const INSPECT_WINDOW_MS: Millis = 6000;

#[derive(Debug, Clone, Serialize)]
pub struct EntitySeriesDto {
    pub name: String,
    pub is_player: bool,
    pub total: u64,
    /// One damage total per bucket in `FightTimelineDto::buckets`, same
    /// length and same order.
    pub values: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct FightTimelineDto {
    pub start_ms: Millis,
    pub duration_ms: Millis,
    pub bucket_ms: Millis,
    /// Start time of each bucket, log-time ms -- same basis as every other
    /// timestamp in this app, so the frontend can line this up against
    /// `start_ms`/`end_ms` without a conversion.
    pub buckets: Vec<Millis>,
    /// Damage-dealing entities only, sorted by total descending. Healers
    /// and pure targets (an entity that only ever received damage) are not
    /// bars on this chart -- the ask was "dps over time per person".
    pub series: Vec<EntitySeriesDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntityStateDto {
    pub name: String,
    pub is_player: bool,
    pub state: &'static str,
    /// Whether `state` came from a log line (mesmerized/charmed/slain) or
    /// was inferred from silence (`Lost`, or the default `Engaged` before
    /// any transition at all). See `docs/design/timeline.md`.
    pub observed: bool,
    /// Damage over the `INSPECT_WINDOW_MS` trailing up to the clicked
    /// instant -- a snapshot reading, not a running total.
    pub dps: f64,
}

/// Per-entity damage-over-time for one fight, for the scrub bar. `None` if
/// the encounter id doesn't exist (evicted, or never was one).
pub fn fight_timeline(ing: &Ingest, encounter_id: u32) -> Option<FightTimelineDto> {
    let id = EncounterId(encounter_id);
    let e = ing.store.encounter(id)?;
    let now = ing.now_ms();
    let start = e.start_ms;
    let end = e.end_ms.unwrap_or(now).max(start);
    let duration = (end - start).max(1);
    let bucket_ms = (duration / TARGET_BUCKETS).max(MIN_BUCKET_MS);

    let entities = ing.entities_by_enc.get(&id).cloned().unwrap_or_default();
    let range = e.range();

    let mut series: Vec<EntitySeriesDto> = Vec::new();
    let mut buckets_len = 0usize;
    for name in &entities {
        let sym = match ing.store.names.get(name) {
            Some(s) => s,
            None => continue,
        };
        let mut ts = Vec::new();
        let mut amt = Vec::new();
        for i in range.clone() {
            if ing.store.kind[i] == EventKind::Damage && ing.store.actor[i] == sym {
                ts.push(ing.store.ts[i]);
                amt.push(ing.store.amount[i]);
            }
        }
        if ts.is_empty() {
            continue; // healer or pure target -- nothing to plot as a dps bar
        }
        let buckets = bucket_series(&ts, &amt, start, end, bucket_ms);
        buckets_len = buckets_len.max(buckets.len());
        let total: u64 = amt.iter().sum();
        let is_player = ing.encounters.entities.kind(name) == Kind::Player;
        series.push(EntitySeriesDto { name: name.clone(), is_player, total, values: buckets.iter().map(|b| b.total).collect() });
    }
    series.sort_by(|a, b| b.total.cmp(&a.total));

    let buckets: Vec<Millis> = (0..buckets_len as Millis).map(|i| start + i * bucket_ms).collect();
    Some(FightTimelineDto { start_ms: start, duration_ms: duration, bucket_ms, buckets, series })
}

/// Every entity in the fight, their state, and a snapshot DPS reading, all
/// as of `ts_ms` -- what clicking a point on the timeline shows.
pub fn fight_state_at(ing: &Ingest, encounter_id: u32, ts_ms: Millis) -> Vec<EntityStateDto> {
    let id = EncounterId(encounter_id);
    let entities = ing.entities_by_enc.get(&id).cloned().unwrap_or_default();

    let mut out: Vec<EntityStateDto> = entities
        .into_iter()
        .map(|name| {
            let is_player = ing.encounters.entities.kind(&name) == Kind::Player;
            let sym = ing.store.names.get(&name);
            let (state, observed) = sym
                .and_then(|s| ing.timeline.state_at(s.0, ts_ms))
                .map(|(s, c)| (s, matches!(c, Cause::Observed)))
                .unwrap_or((eqlp_session::State::Engaged, false));
            let dps = sym
                .map(|s| dps_window(&ing.store, &Filter::encounter(id).damage().by(s), ts_ms, INSPECT_WINDOW_MS))
                .unwrap_or(0.0);
            EntityStateDto { name, is_player, state: state.name(), observed, dps }
        })
        .collect();
    out.sort_by(|a, b| b.dps.partial_cmp(&a.dps).unwrap_or(std::cmp::Ordering::Equal));
    out
}

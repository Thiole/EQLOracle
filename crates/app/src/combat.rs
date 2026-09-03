//! why: Combat module read-side queries -- zone visits, encounters, and
//! ability breakdowns for a selection. No parsing here, every query runs
//! against the already-classified `Store`.

use crate::ingest::Ingest;
use eqlp_session::{series as bucket_series, Cause, Kind, State};
use eqlp_source::Millis;
use eqlp_store::{
    by_ability, by_actor, dps_window, flag, tag, total, AbilityId, AbilityRow, Encounter,
    EncounterId, EventKind, Filter, Sym, NO_ENCOUNTER,
};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct ZoneVisitDto {
    /// why: None for the "unknown" bucket -- encounters before the first zone line
    pub index: Option<usize>,
    pub label: String,
    pub fight_count: usize,
    /// why: most recent visit with no successor, presumably where the player still is
    pub current: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EncounterDto {
    pub id: u32,
    pub target: String,
    /// why: every entity in this fight, not just the anchor target -- a multi-mob pull holds several
    pub entities: Vec<String>,
    pub start_ms: Millis,
    pub end_ms: Option<Millis>,
    pub duration_ms: Millis,
    /// why: team's own output, excludes what the target dealt back -- see enemy_damage/dps
    pub total_damage: u64,
    pub dps: f64,
    /// why: damage the target dealt to the team
    pub enemy_damage: u64,
    pub enemy_dps: f64,
    pub slain: bool,
    pub wiped: bool,
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
    /// why: avg of non-crit hits, matches real EQ parsers -- a per-ability
    /// dps figure used to live here but says more about fight length than the ability
    pub avg_hit: f64,
    pub avg_crit: f64,
    pub pct: f64,
    /// why: fully-avoided swings of this attack type, broken out by how
    pub missed: u64,
    pub blocked: u64,
    pub dodged: u64,
    pub parried: u64,
}

/// why: cast attempts vs landed damage are different questions -- Cast
/// rows track attempts, Damage rows track landed hits (a DoT can be
/// several per cast); blending them into one `hits` would conflate the two.
#[derive(Debug, Clone, Serialize)]
pub struct CastRowDto {
    pub spell: String,
    pub attempts: u32,
    pub landed: u32,
    pub resisted: u32,
    pub interrupted: u32,
    pub fizzled: u32,
    /// why: expired with no confirming line at all
    pub unconfirmed: u32,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CombatSummaryDto {
    pub fight_count: usize,
    /// why: team's own output, excludes what each target dealt back
    pub total_damage: u64,
    pub duration_ms: Millis,
    /// why: mean of each fight's own dps, not pooled -- pooling would let
    /// long grinds dominate over several short, sharp fights
    pub dps: f64,
    /// why: damage the enemy dealt back, grouped not mixed into total_damage
    pub enemy_damage: u64,
    /// why: same per-fight averaging as dps, for incoming damage
    pub enemy_dps: f64,
    pub abilities: Vec<AbilityRowDto>,
    /// why: separate from abilities -- see CastRowDto
    pub casts: Vec<CastRowDto>,
    /// why: healing landed on the target during the selection -- offsets
    /// team damage, worth surfacing separately rather than folding into
    /// total_damage where it'd misread as team output
    pub enemy_heal: u64,
}

fn zone_visit_of(ing: &Ingest, start_ms: Millis) -> Option<usize> {
    ing.zone.index_at(start_ms)
}

/// why: -1 stands in for the "Unknown" bucket -- Option<usize> alone
/// can't distinguish "no filter" from "filter to the no-zone bucket"
fn matches_visit(ing: &Ingest, start_ms: Millis, want: Option<i64>) -> bool {
    match want {
        None => true,
        Some(-1) => zone_visit_of(ing, start_ms).is_none(),
        Some(n) => zone_visit_of(ing, start_ms) == usize::try_from(n).ok(),
    }
}

/// why: shared by two callers so both agree on "fight count" from one scan
fn zone_visit_dtos(ing: &Ingest) -> Vec<ZoneVisitDto> {
    let mut counts: HashMap<Option<usize>, usize> = HashMap::new();
    for e in &ing.store.encounters {
        *counts.entry(zone_visit_of(ing, e.start_ms)).or_insert(0) += 1;
    }
    let last_zone_index = if ing.zone.is_empty() {
        None
    } else {
        Some(ing.zone.len() - 1)
    };

    counts
        .into_iter()
        .map(|(zi, fight_count)| {
            let label = match zi {
                Some(i) => ing
                    .zone
                    .iter()
                    .nth(i)
                    .map(|(_, l)| l.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                None => "Unknown".to_string(),
            };
            ZoneVisitDto {
                index: zi,
                label,
                fight_count,
                current: zi.is_some() && zi == last_zone_index,
            }
        })
        .collect()
}

/// why: newest visit first, "Unknown" bucket sorts last
fn sort_zone_visits(visits: &mut [ZoneVisitDto]) {
    visits.sort_by(|a, b| match (a.index, b.index) {
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
}

pub fn list_zone_visits(ing: &Ingest) -> Vec<ZoneVisitDto> {
    let mut out = zone_visit_dtos(ing);
    sort_zone_visits(&mut out);
    out
}

/// why: computed fresh every call, shared by two callers so both build numbers identically
fn encounter_dto(ing: &Ingest, e: &Encounter, now: Millis) -> EncounterDto {
    let dur = e.duration_ms(now).max(0);
    let dur_secs = (dur as f64 / 1000.0).max(0.001);
    let all = total(&ing.store, &Filter::encounter(e.id).damage());
    let enemy = total(&ing.store, &Filter::encounter(e.id).damage().by(e.target));
    let dmg = all.saturating_sub(enemy);
    EncounterDto {
        id: e.id.0,
        target: ing.store.name(e.target).to_string(),
        entities: ing.entities_by_enc.get(&e.id).cloned().unwrap_or_default(),
        start_ms: e.start_ms,
        end_ms: e.end_ms,
        duration_ms: dur,
        total_damage: dmg,
        dps: dmg as f64 / dur_secs,
        enemy_damage: enemy,
        enemy_dps: enemy as f64 / dur_secs,
        slain: e.slain,
        wiped: e.wiped,
        open: e.is_open(),
    }
}

/// why: windowed for real -- sort cheap refs first, slice, then only
/// compute DTOs for the slice; O(limit) not O(total fights ever)
pub fn list_encounters(
    ing: &Ingest,
    zone_visit: Option<i64>,
    offset: usize,
    limit: usize,
) -> Vec<EncounterDto> {
    let now = ing.now_ms();
    let mut matched: Vec<&Encounter> = ing
        .store
        .encounters
        .iter()
        // why: the Combat tab lists the player's own fights; someone
        // else's stays parsed in the backend (Debug shows it, flagged)
        .filter(|e| e.involves_you && !e.absorbed && matches_visit(ing, e.start_ms, zone_visit))
        .collect();
    matched.sort_by_key(|b| std::cmp::Reverse(b.start_ms));
    matched
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|e| encounter_dto(ing, e, now))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct EncounterDropDto {
    pub item: String,
    pub qty: u64,
    /// why: lower is rarer; None sorts after every ranked drop, not guessed
    pub chance: Option<f64>,
}

/// why: the cheap half of an encounter, no `total()` aggregation needed --
/// computing totals for every visible row was a real measured cost;
/// `encounter_detail` fetches those once a card actually expands
#[derive(Debug, Clone, Serialize)]
pub struct EncounterPreviewDto {
    pub id: u32,
    pub target: String,
    pub start_ms: Millis,
    pub end_ms: Option<Millis>,
    pub duration_ms: Millis,
    pub slain: bool,
    pub wiped: bool,
    pub open: bool,
}

fn encounter_preview_dto(ing: &Ingest, e: &Encounter, now: Millis) -> EncounterPreviewDto {
    EncounterPreviewDto {
        id: e.id.0,
        target: ing.store.name(e.target).to_string(),
        start_ms: e.start_ms,
        end_ms: e.end_ms,
        duration_ms: e.duration_ms(now).max(0),
        slain: e.slain,
        wiped: e.wiped,
        open: e.is_open(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ZoneEncounterDto {
    // why: deliberately NOT #[serde(flatten)] -- this nests as a real
    // "encounter": {...} object the frontend expects (ze.encounter.id).
    // flatten silently broke this once already (a real "stuck on loading"
    // bug traced back here, no error surfaced, no crash).
    pub encounter: EncounterPreviewDto,
    /// why: difficulty tier read off the first row, stamped at ingest time not recomputed
    pub tier: u8,
    /// why: this fight's visit index, what the zone-select dropdown is keyed by
    pub zone_visit: Option<i64>,
    /// why: best-effort display zone -- resolved wiki id, else raw label, else None
    pub zone: Option<String>,
}

fn display_zone(ing: &Ingest, e: &Encounter) -> Option<String> {
    let z = e.zone?;
    Some(
        ing.cached_wiki_zone(z)
            .map(str::to_string)
            .unwrap_or_else(|| ing.store.name(z).to_string()),
    )
}

/// why: 90s (the original value) assumed looting always follows death
/// quickly -- wrong for interactive loot windows that can sit for
/// minutes mid-raid, silently missing real drops. 30min trades a small
/// false-attribution risk (blunted by claim tracking preferring the
/// oldest unclaimed kill) for closing that larger false-miss gap.
/// `pub(crate)`: `Ingest::record_loot` uses this exact window too.
pub(crate) const LOOT_GRACE_MS: Millis = 30 * 60_000;

/// why: prefers chance_per_kill (more consistently populated) over
/// chance_per_drop; None covers three distinct real gaps, never "common" by default
fn drop_chance(mob: &str, item: &str) -> Option<f64> {
    let npc = crate::npcdata::npcs()
        .iter()
        .find(|n| crate::mobalias::mob_matches(mob, &n.name))?;
    let entry = npc
        .known_loot
        .iter()
        .find(|l| l.item.eq_ignore_ascii_case(item))?;
    entry.chance_per_kill.or(entry.chance_per_drop)
}

fn sort_drops_rarer_first(drops: &mut [EncounterDropDto]) {
    drops.sort_by(|a, b| match (a.chance, b.chance) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.item.cmp(&b.item),
    });
}

/// why: recent encounters keyed by exact `zone_id`, not a display-name
/// compare -- folded across visits since this only needs "did this fight
/// happen in this zone", not which specific visit. Walked in reverse,
/// stops at `limit` matches, O(1) zone lookup via `cached_wiki_zone`.
/// Deliberately no damage totals/drops here -- computing those eagerly
/// for every row was the real measured cost; see `encounter_detail`.
pub fn list_zone_encounters(ing: &Ingest, zone_id: &str, limit: usize) -> Vec<ZoneEncounterDto> {
    let now = ing.now_ms();

    let mut matched: Vec<&Encounter> = Vec::with_capacity(limit.min(256));
    for e in ing.store.encounters.iter().rev() {
        // why: involves_you -- your own recent fights here, not a
        // stranger group's; list_mob_encounters (Monsters, a data view)
        // deliberately keeps every observed fight instead
        let is_match = e.involves_you
            && !e.absorbed
            && e.zone.and_then(|z| ing.cached_wiki_zone(z)) == Some(zone_id);
        if is_match {
            matched.push(e);
            if matched.len() >= limit {
                break;
            }
        }
    }
    // why: already newest-first from reverse iteration, no separate sort needed

    matched
        .into_iter()
        .map(|e| {
            let dto = encounter_preview_dto(ing, e, now);
            let tier = ing.store.tier.get(e.first as usize).copied().unwrap_or(0);
            let zone_visit = zone_visit_of(ing, e.start_ms).map(|i| i as i64);
            let zone = display_zone(ing, e);
            ZoneEncounterDto {
                encounter: dto,
                tier,
                zone_visit,
                zone,
            }
        })
        .collect()
}

/// why: `list_zone_encounters`' twin, matching by mob name; simpler --
/// log and Game Data names agree, no alias table needed, just eq_ignore_ascii_case
pub fn list_mob_encounters(ing: &Ingest, mob_name: &str, limit: usize) -> Vec<ZoneEncounterDto> {
    let now = ing.now_ms();

    let mut matched: Vec<&Encounter> = Vec::with_capacity(limit.min(256));
    for e in ing.store.encounters.iter().rev() {
        if ing.store.name(e.target).eq_ignore_ascii_case(mob_name) {
            matched.push(e);
            if matched.len() >= limit {
                break;
            }
        }
    }

    matched
        .into_iter()
        .map(|e| {
            let dto = encounter_preview_dto(ing, e, now);
            let tier = ing.store.tier.get(e.first as usize).copied().unwrap_or(0);
            let zone_visit = zone_visit_of(ing, e.start_ms).map(|i| i as i64);
            let zone = display_zone(ing, e);
            ZoneEncounterDto {
                encounter: dto,
                tier,
                zone_visit,
                zone,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct EncounterDetailDto {
    pub total_damage: u64,
    pub dps: f64,
    pub enemy_damage: u64,
    pub enemy_dps: f64,
    /// why: best-effort, resolved once at ingest time; ambiguity (two
    /// same-named mobs pulled at once) doesn't go away, just when it's
    /// resolved. Rarer first.
    pub drops: Vec<EncounterDropDto>,
}

/// why: expensive half of one encounter, computed on demand once
/// expanded -- damage totals + windowed loot join. Rows are appended in
/// strict chronological order, so `ing.store.ts` is already sorted;
/// binary-searches to the fight's window instead of scanning from
/// session start. Falls back to the encounter's own last row, not "now",
/// for a never-cleanly-closed fight -- "now" would silently widen the
/// window to the entire rest of the store. None for unknown id, not a
/// zeroed DTO -- "doesn't exist" and "zero damage" are different facts.
pub fn encounter_detail(ing: &Ingest, encounter_id: u32) -> Option<EncounterDetailDto> {
    let now = ing.now_ms();
    let e = ing.store.encounter(EncounterId(encounter_id))?;

    let dur = e.duration_ms(now).max(0);
    let dur_secs = (dur as f64 / 1000.0).max(0.001);
    let all = total(&ing.store, &Filter::encounter(e.id).damage());
    let enemy = total(&ing.store, &Filter::encounter(e.id).damage().by(e.target));
    let dmg = all.saturating_sub(enemy);

    // why: loot rows fall outside e.range() -- looting happens after the
    // last swing/cast, needs its own time-bounded scan. Match is
    // enc[i] == e.id, an exact read of Ingest::record_loot's decision, not re-derived
    let last_activity = e.end_ms.unwrap_or_else(|| {
        ing.store
            .ts
            .get(e.last as usize)
            .copied()
            .unwrap_or(e.start_ms)
    });
    let window_end = last_activity + LOOT_GRACE_MS;
    let lo = ing.store.ts.partition_point(|&t| t < e.start_ms);
    let hi = ing.store.ts.partition_point(|&t| t <= window_end);
    let target_name = ing.store.name(e.target).to_string();
    // why: tier-folded to the base item, same-fight tiers merged -- the
    // wiki drop table (drop_chance's own lookup) only knows untiered
    // names, and "Rusty Mace +2"/"+3" looted in one fight is one drop row
    let mut by_item: HashMap<String, u64> = HashMap::new();
    for i in
        (lo..hi).filter(|&i| ing.store.kind[i] == EventKind::Loot && ing.store.enc[i] == e.id.0)
    {
        let (base, _tier) =
            crate::inventory::strip_tier(ing.store.ability_name(ing.store.ability[i]));
        *by_item.entry(base.to_string()).or_insert(0) += ing.store.amount[i];
    }
    let mut drops: Vec<EncounterDropDto> = by_item
        .into_iter()
        .map(|(item, qty)| {
            let chance = drop_chance(&target_name, &item);
            EncounterDropDto { item, qty, chance }
        })
        .collect();
    sort_drops_rarer_first(&mut drops);

    Some(EncounterDetailDto {
        total_damage: dmg,
        dps: dmg as f64 / dur_secs,
        enemy_damage: enemy,
        enemy_dps: enemy as f64 / dur_secs,
        drops,
    })
}

/// why: shared by `summarize`/`list_allies`, both aggregate the current selection differently
fn resolve_ids(
    ing: &Ingest,
    zone_visit: Option<i64>,
    encounter_id: Option<u32>,
    confirmed_only: bool,
) -> Vec<EncounterId> {
    if let Some(eid) = encounter_id {
        // why: an explicit id is honored involved-or-not -- the caller
        // (debug view, a direct link) asked for that specific fight
        vec![EncounterId(eid)]
    } else {
        // why: aggregates scope to the player's own fights -- a stranger
        // group's fight in the same visit is backend data, not "your combat".
        // confirmed_only additionally drops closed "reset" fights (neither
        // slain nor wiped) -- the copy-report ask: an abandoned/fled fight
        // fragment shouldn't dilute a shared aggregate's numbers. Open
        // fights and wipes stay: both are real combat.
        ing.store
            .encounters
            .iter()
            .filter(|e| e.involves_you && !e.absorbed && matches_visit(ing, e.start_ms, zone_visit))
            .filter(|e| !confirmed_only || e.is_open() || e.slain || e.wiped)
            .map(|e| e.id)
            .collect()
    }
}

/// Merges one `by_ability` call's rows into a running accumulator.
fn merge_ability_rows(dst: &mut HashMap<eqlp_store::AbilityId, AbilityRow>, rows: Vec<AbilityRow>) {
    for row in rows {
        let acc = dst.entry(row.ability).or_insert_with(|| AbilityRow {
            ability: row.ability,
            tags: row.tags,
            total: 0,
            hits: 0,
            min: u64::MAX,
            max: 0,
            full_power: 0,
            crits: 0,
            crit_total: 0,
            missed: 0,
            blocked: 0,
            dodged: 0,
            parried: 0,
            flags: 0,
        });
        acc.total += row.total;
        acc.hits += row.hits;
        acc.min = acc.min.min(row.min);
        acc.max = acc.max.max(row.max);
        acc.full_power += row.full_power;
        acc.crits += row.crits;
        acc.crit_total += row.crit_total;
        acc.missed += row.missed;
        acc.blocked += row.blocked;
        acc.dodged += row.dodged;
        acc.parried += row.parried;
        acc.flags |= row.flags;
    }
}

/// why: subtracts and drops zeroed entries so an enemy-only ability
/// vanishes from a team breakdown, not lingers as a zero row. min/max
/// aren't subtractable, left as all-actors extremes -- a minor known imprecision
fn subtract_ability_rows(
    dst: &mut HashMap<eqlp_store::AbilityId, AbilityRow>,
    rows: Vec<AbilityRow>,
) {
    for row in rows {
        if let Some(acc) = dst.get_mut(&row.ability) {
            acc.total = acc.total.saturating_sub(row.total);
            acc.hits = acc.hits.saturating_sub(row.hits);
            acc.crits = acc.crits.saturating_sub(row.crits);
            acc.crit_total = acc.crit_total.saturating_sub(row.crit_total);
            acc.full_power = acc.full_power.saturating_sub(row.full_power);
            acc.missed = acc.missed.saturating_sub(row.missed);
            acc.blocked = acc.blocked.saturating_sub(row.blocked);
            acc.dodged = acc.dodged.saturating_sub(row.dodged);
            acc.parried = acc.parried.saturating_sub(row.parried);
        }
    }
    dst.retain(|_, r| r.total > 0 || r.attempts() > 0);
}

/// why: every cast, grouped by ability and outcome. Walks raw store
/// columns not `by_ability` -- that only tracks one OR'd flags bitmask,
/// can't recover landed/resisted/interrupted counts.
fn cast_rows(ing: &Ingest, ids: &[EncounterId], actor_sym: Option<Sym>) -> Vec<CastRowDto> {
    let mut acc: HashMap<AbilityId, CastRowDto> = HashMap::new();
    for &id in ids {
        let Some(enc) = ing.store.encounter(id) else {
            continue;
        };
        for i in enc.range() {
            if ing.store.kind[i] != EventKind::Cast || ing.store.enc[i] != id.0 {
                continue;
            }
            let row_actor = ing.store.actor[i];
            match actor_sym {
                Some(sym) if row_actor != sym => continue,
                None if row_actor == enc.target => continue, // the fight's own mob, not "the team"
                _ => {}
            }
            let ab = ing.store.ability[i];
            let row = acc.entry(ab).or_insert_with(|| CastRowDto {
                spell: ing.store.ability_name(ab).to_string(),
                attempts: 0,
                landed: 0,
                resisted: 0,
                interrupted: 0,
                fizzled: 0,
                unconfirmed: 0,
            });
            row.attempts += 1;
            let fl = ing.store.flags[i];
            if fl & flag::CAST_LANDED != 0 {
                row.landed += 1;
            } else if fl & flag::CAST_RESISTED != 0 {
                row.resisted += 1;
            } else if fl & flag::CAST_INTERRUPTED != 0 {
                row.interrupted += 1;
            } else if fl & flag::CAST_FIZZLED != 0 {
                row.fizzled += 1;
            } else if fl & flag::CAST_UNCONFIRMED != 0 {
                row.unconfirmed += 1;
            }
        }
    }
    let mut v: Vec<CastRowDto> = acc.into_values().collect();
    v.sort_by(|a, b| {
        b.attempts
            .cmp(&a.attempts)
            .then_with(|| a.spell.cmp(&b.spell))
    });
    v
}

/// why: aggregates the selection, `encounter_id` wins over `zone_visit`
/// over everything; `actor` narrows to one ally's abilities. Team totals
/// exclude the target (subtracted below), reported separately as
/// enemy_damage/dps. One or two calls per encounter (measured 39µs each)
/// rather than teaching Filter to accept a set of ids -- cheap enough as is.
pub fn summarize(
    ing: &Ingest,
    zone_visit: Option<i64>,
    encounter_id: Option<u32>,
    actor: Option<&str>,
    confirmed_only: bool,
) -> CombatSummaryDto {
    let now = ing.now_ms();
    let ids = resolve_ids(ing, zone_visit, encounter_id, confirmed_only);
    if ids.is_empty() {
        return CombatSummaryDto::default();
    }
    let actor_sym = actor.and_then(|n| ing.store.names.get(n));

    let mut duration_ms: Millis = 0;
    let mut enemy_damage = 0u64;
    let mut enemy_heal = 0u64;
    let mut merged: HashMap<eqlp_store::AbilityId, AbilityRow> = HashMap::new();
    // why: one dps reading per fight, averaged not pooled -- see CombatSummaryDto::dps
    let mut per_fight_dps: Vec<f64> = Vec::new();
    let mut per_fight_enemy_dps: Vec<f64> = Vec::new();

    for &id in &ids {
        let Some(enc) = ing.store.encounter(id) else {
            continue;
        };
        let dur = enc.duration_ms(now).max(0);
        let fight_secs = (dur as f64 / 1000.0).max(0.001);
        duration_ms += dur;

        let enemy_rows = by_ability(&ing.store, &Filter::encounter(id).damage().by(enc.target));
        let fight_enemy: u64 = enemy_rows.iter().map(|r| r.total).sum();
        enemy_damage += fight_enemy;
        per_fight_enemy_dps.push(fight_enemy as f64 / fight_secs);
        // why: heals landing ON the target, whoever cast them -- self-heal
        // or an ally healing it, either way it's damage the team didn't
        // actually make stick
        enemy_heal += total(
            &ing.store,
            &Filter::encounter(id)
                .kind(EventKind::Heal)
                .target(enc.target),
        );

        if let Some(sym) = actor_sym {
            // why: one specific ally, own rows only -- never their own target
            let rows = by_ability(&ing.store, &Filter::encounter(id).damage().by(sym));
            let fight_total: u64 = rows.iter().map(|r| r.total).sum();
            per_fight_dps.push(fight_total as f64 / fight_secs);
            merge_ability_rows(&mut merged, rows);
            // why: separate query -- Filter narrows to one kind, avoided
            // swings are Miss not Damage; merges onto the same row below
            let avoided = by_ability(
                &ing.store,
                &Filter::encounter(id).kind(EventKind::Miss).by(sym),
            );
            merge_ability_rows(&mut merged, avoided);
        } else {
            // why: everyone minus the target's own contribution; reuses enemy_rows above
            let all = by_ability(&ing.store, &Filter::encounter(id).damage());
            let all_total: u64 = all.iter().map(|r| r.total).sum();
            per_fight_dps.push(all_total.saturating_sub(fight_enemy) as f64 / fight_secs);
            merge_ability_rows(&mut merged, all);
            subtract_ability_rows(&mut merged, enemy_rows);
            let all_avoided = by_ability(&ing.store, &Filter::encounter(id).kind(EventKind::Miss));
            let enemy_avoided = by_ability(
                &ing.store,
                &Filter::encounter(id).kind(EventKind::Miss).by(enc.target),
            );
            merge_ability_rows(&mut merged, all_avoided);
            subtract_ability_rows(&mut merged, enemy_avoided);
        }
    }

    let mut rows: Vec<AbilityRow> = merged.into_values().collect();
    for r in &mut rows {
        if r.min == u64::MAX {
            r.min = 0;
        }
    }
    rows.sort_by_key(|b| std::cmp::Reverse(b.total));

    let total_damage: u64 = rows.iter().map(|r| r.total).sum();

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
            avg_hit: r.avg_normal(),
            avg_crit: r.avg_crit(),
            pct: if total_damage > 0 {
                100.0 * r.total as f64 / total_damage as f64
            } else {
                0.0
            },
            missed: r.missed,
            blocked: r.blocked,
            dodged: r.dodged,
            parried: r.parried,
        })
        .collect();

    let mean = |v: &[f64]| {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };

    CombatSummaryDto {
        fight_count: ids.len(),
        total_damage,
        duration_ms,
        dps: mean(&per_fight_dps),
        enemy_damage,
        enemy_dps: mean(&per_fight_enemy_dps),
        abilities,
        casts: cast_rows(ing, &ids, actor_sym),
        enemy_heal,
    }
}

// ---------------------------------------------------------------- allies

#[derive(Debug, Clone, Serialize)]
pub struct AllyDto {
    pub name: String,
    pub is_player: bool,
    pub is_pet: bool,
    pub total: u64,
    pub hits: u64,
    pub crits: u64,
    pub crit_pct: f64,
    pub dps: f64,
    pub pct: f64,
    /// why: None (not 0%) when this ally never threw a melee-avoidable
    /// swing -- a pure caster has nothing to report here, distinct from
    /// "landed everything"
    pub hit_pct: Option<f64>,
    /// why: None when this ally never cast a resistable spell, same
    /// reasoning as hit_pct
    pub resist_pct: Option<f64>,
    /// why: the class trio -- a /who row from THIS presence when one
    /// printed (`class_confirmed`, with `level`), else inferred through
    /// combat from what this ally cast and swung; empty with no evidence
    pub classes: Vec<String>,
    pub class_confirmed: bool,
    /// why: how many votes (landed or begun spells, class-only swings) back
    /// an inference; 0 when confirmed
    pub class_evidence: u32,
    /// why: from the /who row only
    pub level: Option<u8>,
    /// why: how much of `total` arrived via this ally's own pet(s),
    /// folded in by possessive-name ownership ("X's pet" -> X) -- 0 for
    /// an ally with no attributed pet damage. Summon-matched pets were
    /// already merged at ingest (Ingest::sym's pet_owner map); this
    /// closes the possessive-name half that Entities::credit knew about
    /// but nothing ever applied.
    pub pet_total: u64,
    /// why: this row is a SUGGESTED ally, not a proven one -- included
    /// only because it's currently charm-flipped or group-tracked
    /// (effective_kind), with no permanent Player/Pet proof behind it.
    /// Charm pets land here by design (player's own spec: "pets should
    /// be attributed, charm pets should be suggestions") -- the UI
    /// renders these visibly tentative instead of silently equal.
    pub suggested: bool,
}

/// why: Combat module's primary view, damage dealers sorted descending.
/// Reads directly from the store, not `entities_by_enc` -- a pet-owner
/// who never personally swings could be missing from that list, the
/// store finds them regardless. Excludes everything currently `Enemy`
/// per `Allegiance::of` -- a multi-mob pull's other mobs excluded too, a
/// live-charmed mob counts as ally while the charm holds.
pub fn list_allies(
    ing: &Ingest,
    zone_visit: Option<i64>,
    encounter_id: Option<u32>,
    confirmed_only: bool,
) -> Vec<AllyDto> {
    let ids = resolve_ids(ing, zone_visit, encounter_id, confirmed_only);
    // why: class evidence is per zone visit -- the visit of the fight
    // being looked at (a selected encounter, else the selected visit,
    // else now), so an old fight shows the classes in play back then
    let class_visit: Option<usize> = match (encounter_id, zone_visit) {
        (Some(eid), _) => ing
            .store
            .encounter(EncounterId(eid))
            .and_then(|e| ing.zone.index_at(e.start_ms)),
        (None, Some(zv)) => usize::try_from(zv).ok(),
        (None, None) => ing.zone.index_at(ing.now_ms()),
    };
    if ids.is_empty() {
        return Vec::new();
    }

    let now = ing.now_ms();
    let mut acc: HashMap<Sym, (u64, u64, u64)> = HashMap::new();

    for &id in &ids {
        if ing.store.encounter(id).is_none() {
            continue;
        }
        for (sym, dmg, hits, crits) in by_actor(&ing.store, &Filter::encounter(id).damage()) {
            let name = ing.store.name(sym);
            // why: one composition of kind/charm/group belief -- see
            // Ingest::allegiance_at for why this isn't effective_kind+of
            if ing.allegiance_at(name, now).is_enemy() {
                continue;
            }
            let e = acc.entry(sym).or_insert((0, 0, 0));
            e.0 += dmg;
            e.1 += hits;
            e.2 += crits;
        }
    }

    // why: melee-avoided swings (Miss kind) and cast outcomes (Cast kind)
    // are separate event kinds from landed Damage, see EventKind's own
    // doc -- only rolled up for syms already known to be allies above, no
    // point tracking a mob's own casts/misses here
    let mut avoided: HashMap<Sym, u64> = HashMap::new();
    let mut casts: HashMap<Sym, (u64, u64)> = HashMap::new(); // (attempts, resisted)
    for &id in &ids {
        let Some(enc) = ing.store.encounter(id) else {
            continue;
        };
        for (sym, _amt, n, _crits) in
            by_actor(&ing.store, &Filter::encounter(id).kind(EventKind::Miss))
        {
            if acc.contains_key(&sym) {
                *avoided.entry(sym).or_insert(0) += n;
            }
        }
        for i in enc.range() {
            if ing.store.enc[i] != id.0 || ing.store.kind[i] != EventKind::Cast {
                continue;
            }
            let actor = ing.store.actor[i];
            if actor == enc.target || !acc.contains_key(&actor) {
                continue; // the mob's own casts, or a non-ally, not "the team"
            }
            let e = casts.entry(actor).or_insert((0, 0));
            e.0 += 1;
            if ing.store.flags[i] & flag::CAST_RESISTED != 0 {
                e.1 += 1;
            }
        }
    }

    let duration_ms: Millis = ids
        .iter()
        .filter_map(|&id| ing.store.encounter(id))
        .map(|e| e.duration_ms(now).max(0))
        .sum();
    let dur_secs = (duration_ms.max(0) as f64 / 1000.0).max(0.001);
    let total_damage: u64 = acc.values().map(|(dmg, _, _)| dmg).sum();

    // why: fold possessive-named pets ("X's pet") into their owner's own
    // row before building DTOs -- Entities::owner_of has known this
    // mapping all along, nothing ever applied it, so a pet-heavy ally
    // read as two unrelated rows. Summon-matched pets never reach here
    // as themselves (merged at ingest, Ingest::sym's own pet_owner map).
    // Charm pets have no owner mapping and stay their own row, marked
    // `suggested` below instead of silently equal.
    #[derive(Default)]
    struct Merged {
        total: u64,
        hits: u64,
        crits: u64,
        avoided: u64,
        cast_attempts: u64,
        cast_resisted: u64,
        pet_total: u64,
        /// why: true only while EVERY contributor was a pet row -- an
        /// owner who also swings personally clears it
        pet_only: bool,
    }
    let mut by_name: HashMap<String, Merged> = HashMap::new();
    for (sym, (dmg, hits, crits)) in acc {
        let name = ing.store.name(sym).to_string();
        let owner = ing
            .encounters
            .entities
            .owner_of(&name)
            .map(|o| o.to_string());
        let is_pet_row = owner.is_some();
        let credited = owner.unwrap_or_else(|| name.clone());
        let first = !by_name.contains_key(&credited);
        let e = by_name.entry(credited).or_default();
        if first {
            e.pet_only = is_pet_row;
        } else {
            e.pet_only = e.pet_only && is_pet_row;
        }
        e.total += dmg;
        e.hits += hits;
        e.crits += crits;
        if is_pet_row {
            e.pet_total += dmg;
        }
        e.avoided += avoided.get(&sym).copied().unwrap_or(0);
        if let Some(&(attempts, resisted)) = casts.get(&sym) {
            e.cast_attempts += attempts;
            e.cast_resisted += resisted;
        }
    }

    let mut out: Vec<AllyDto> = by_name
        .into_iter()
        .map(|(name, m)| {
            let kind = ing.effective_kind(&name, now);
            // why: proven means a permanent Kind (player channel, or a
            // real pet-name/summon match); Unproven-but-included means
            // only the charm flip or GroupTracker let it in -- that's a
            // suggestion, not a fact. "You" needs no proving.
            let suggested = !name.eq_ignore_ascii_case("you")
                && ing.encounters.entities.kind(&name) == Kind::Unproven;
            let (classes, class_confirmed, class_evidence, level) =
                match ing.ally_who(&name, class_visit) {
                    Some((lvl, trio)) => (trio.to_vec(), true, 0, Some(lvl)),
                    None => {
                        let (c, n) = ing.ally_classes(&name, class_visit);
                        (c, false, n, None)
                    }
                };
            AllyDto {
                classes,
                class_confirmed,
                class_evidence,
                level,
                is_player: kind == Kind::Player,
                is_pet: kind == Kind::Pet || m.pet_only,
                total: m.total,
                hits: m.hits,
                crits: m.crits,
                crit_pct: if m.hits > 0 {
                    100.0 * m.crits as f64 / m.hits as f64
                } else {
                    0.0
                },
                dps: m.total as f64 / dur_secs,
                pct: if total_damage > 0 {
                    100.0 * m.total as f64 / total_damage as f64
                } else {
                    0.0
                },
                hit_pct: {
                    let attempts = m.hits + m.avoided;
                    (attempts > 0).then(|| 100.0 * m.hits as f64 / attempts as f64)
                },
                resist_pct: (m.cast_attempts > 0)
                    .then(|| 100.0 * m.cast_resisted as f64 / m.cast_attempts as f64),
                pet_total: m.pet_total,
                suggested,
                name,
            }
        })
        .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.total));
    out
}

// ---------------------------------------------------------------- timeline

/// why: target bucket count so short skirmishes and long grinds both render readable
const TARGET_BUCKETS: Millis = 60;
const MIN_BUCKET_MS: Millis = 1000;

/// why: long enough not to be one lucky hit, short enough to feel like "right now"
const INSPECT_WINDOW_MS: Millis = 6000;

/// why: wider than INSPECT_WINDOW_MS -- a buff stays plausibly relevant
/// longer than a DPS snapshot; no wear-off line, so this is "landed
/// recently" never "still active"
const EFFECT_RECENCY_MS: Millis = 60_000;

#[derive(Debug, Clone, Serialize)]
pub struct EntitySeriesDto {
    pub name: String,
    pub is_player: bool,
    pub is_pet: bool,
    /// why: allegiance as of the fight's end, covers multi-mob pulls, charmed mob reads as ally
    pub is_enemy: bool,
    pub total: u64,
    /// why: one total per bucket, same length and order
    pub values: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct FightTimelineDto {
    pub start_ms: Millis,
    pub duration_ms: Millis,
    pub bucket_ms: Millis,
    /// why: log-time ms, same basis as every other timestamp, no conversion needed
    pub buckets: Vec<Millis>,
    /// why: damage-dealing entities only, sorted descending -- the ask was "dps over time"
    pub series: Vec<EntitySeriesDto>,
}

/// why: real, best-effort attribution -- see `Ingest::attribute_effect`'s
/// own doc for exactly how `source`/`skill` get filled in. Either can be
/// `None` on its own (skill known, source ambiguous/out of view -- or,
/// rarer, the reverse); `text` is always real, straight off the log line.
#[derive(Debug, Clone, Serialize)]
pub struct RecentEffectDto {
    pub source: Option<String>,
    pub skill: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntityStateDto {
    pub name: String,
    pub is_player: bool,
    pub is_pet: bool,
    /// why: allegiance at ts_ms, always agrees with `state` since both derive from it
    pub is_enemy: bool,
    pub state: &'static str,
    /// why: whether state came from a log line or was inferred from silence
    pub observed: bool,
    /// why: snapshot damage over the trailing inspect window, not a running total
    pub dps: f64,
    /// why: recognized buff/state text within EFFECT_RECENCY_MS, each
    /// with best-effort source/skill attribution
    pub recent_effects: Vec<RecentEffectDto>,
}

/// why: per-entity damage-over-time for the scrub bar; None for an unknown encounter id
pub fn fight_timeline(ing: &Ingest, encounter_id: u32) -> Option<FightTimelineDto> {
    let id = EncounterId(encounter_id);
    let e = ing.store.encounter(id)?;
    let now = ing.now_ms();
    let start = e.start_ms;
    let end = e.end_ms.unwrap_or(now).max(start);
    let duration = (end - start).max(1);
    let bucket_ms = (duration / TARGET_BUCKETS).max(MIN_BUCKET_MS);

    // why: resolved through inferred pet ownership, de-duplicated --
    // graph doesn't know about pet merging, could name the same entity twice
    let mut entities: Vec<String> = ing
        .entities_by_enc
        .get(&id)
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|n| ing.effective_name(n))
        .collect();
    entities.sort();
    entities.dedup();
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
            continue; // why: healer or pure target, nothing to plot
        }
        let buckets = bucket_series(&ts, &amt, start, end, bucket_ms);
        buckets_len = buckets_len.max(buckets.len());
        let total: u64 = amt.iter().sum();
        let kind = ing.effective_kind(name, end);
        // why: as of the fight's end -- a query, not a stored flag; a still-charmed mob reads as ally
        series.push(EntitySeriesDto {
            name: name.clone(),
            is_player: kind == Kind::Player,
            is_pet: kind == Kind::Pet,
            is_enemy: ing.allegiance_at(name, end).is_enemy(),
            total,
            values: buckets.iter().map(|b| b.total).collect(),
        });
    }
    series.sort_by_key(|b| std::cmp::Reverse(b.total));

    let buckets: Vec<Millis> = (0..buckets_len as Millis)
        .map(|i| start + i * bucket_ms)
        .collect();
    Some(FightTimelineDto {
        start_ms: start,
        duration_ms: duration,
        bucket_ms,
        buckets,
        series,
    })
}

/// why: what clicking a timeline point shows -- every entity, state, and a snapshot DPS
pub fn fight_state_at(ing: &Ingest, encounter_id: u32, ts_ms: Millis) -> Vec<EntityStateDto> {
    fight_state_at_windowed(ing, encounter_id, ts_ms, INSPECT_WINDOW_MS)
}

/// why: fight_state_at's own real body, window size pulled out -- the
/// overlay's own live_meter needs a wider rolling window than the
/// Combat tab's timeline-scrub feature does (see live_meter's own doc),
/// and needs to point that same window at the fight's own end once it's
/// closed rather than "now". Same dps calc either way, just parameterized.
fn fight_state_at_windowed(
    ing: &Ingest,
    encounter_id: u32,
    ts_ms: Millis,
    window_ms: Millis,
) -> Vec<EntityStateDto> {
    let id = EncounterId(encounter_id);
    // why: same as fight_timeline -- resolved through inferred pet ownership, de-duplicated
    let mut entities: Vec<String> = ing
        .entities_by_enc
        .get(&id)
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|n| ing.effective_name(n))
        .collect();
    entities.sort();
    entities.dedup();

    let mut out: Vec<EntityStateDto> = entities
        .into_iter()
        .map(|name| {
            let kind = ing.effective_kind(&name, ts_ms);
            let sym = ing.store.names.get(&name);
            let (state, observed) = sym
                .and_then(|s| ing.timeline.state_at(s.0, ts_ms))
                .map(|(s, c)| (s, matches!(c, Cause::Observed)))
                .unwrap_or((State::Engaged, false));
            let dps = sym
                .map(|s| {
                    dps_window(
                        &ing.store,
                        &Filter::encounter(id).damage().by(s),
                        ts_ms,
                        window_ms,
                    )
                })
                .unwrap_or(0.0);
            let recent_effects = sym
                .map(|s| {
                    ing.effects
                        .recent(s.0, ts_ms, EFFECT_RECENCY_MS)
                        .into_iter()
                        .map(|p| RecentEffectDto {
                            source: p.source.as_deref().map(str::to_string),
                            skill: p.skill.as_deref().map(str::to_string),
                            text: p.text.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            EntityStateDto {
                is_player: kind == Kind::Player,
                is_pet: kind == Kind::Pet,
                is_enemy: ing.allegiance_at(&name, ts_ms).is_enemy(),
                state: state.name(),
                observed,
                dps,
                recent_effects,
                name,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.dps
            .partial_cmp(&a.dps)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// why: one meter row, the spec: name, time in encounter, damage, DPS,
/// % of that team's damage. Totals accumulate over the WHOLE encounter
/// -- they never reset when a target dies. `active_ms` is this entity's
/// time in the encounter (its first action to the live edge) and DPS
/// runs over it; the encounter's own timer is LiveMeterDto::duration_ms.
#[derive(Debug, Clone, Serialize)]
pub struct LiveMeterRowDto {
    pub name: String,
    pub pct: f64,
    pub total: u64,
    pub dps: f64,
    pub active_ms: Millis,
    pub is_player: bool,
    pub is_pet: bool,
}

/// why: the overlay DPS meter's whole data source -- same split the
/// Combat tab's own summary card shows (team output vs. what's coming
/// back), not just a flat roster
#[derive(Debug, Clone, Serialize)]
pub struct LiveMeterDto {
    /// why: the anchor label ("a zol ghoul knight +2") -- kept for the
    /// Combat tab's fight list; the widget header is team v team now
    pub target: String,
    pub open: bool,
    /// why: "team v team": distinct allies who dealt damage, distinct
    /// enemies hit or hitting, over the whole encounter
    pub ally_count: usize,
    pub enemy_count: usize,
    /// why: the enemy you most recently exchanged damage with -- shown
    /// UNDER the encounter line, never as its name ("if you want to do
    /// that show current target: but under the encounter name")
    pub current_target: Option<String>,
    /// why: the encounter clock -- from the player's own first
    /// involvement (a hit dealt or taken) to the live edge or the end;
    /// every row's DPS is total over this
    pub start_ms: Millis,
    pub duration_ms: Millis,
    /// why: ally-side damage INTO enemies, ranked by total
    pub outgoing: Vec<LiveMeterRowDto>,
    /// why: enemy-side damage INTO allies -- same calc, other side
    pub incoming: Vec<LiveMeterRowDto>,
}

/// why: shared by live_meter and the Skill Tracker's target-effects
/// section -- "most recently ACTIVE real encounter," not just most
/// recently opened. Real bug, caught live -- Encounter::start_ms is when
/// that encounter object was *opened*, not evidence anything real is
/// still happening in it. A stray miss/attempt against an unrelated
/// target (real case: a lone 0/0 encounter against "Consetta") can open
/// a brand new encounter with a later start_ms than a real fight that's
/// still actively landing damage, winning a max_by_key(start_ms) pick
/// outright and showing an empty meter while the real fight goes on.
/// The store is append order (chronological) -- walking backward for
/// the last row that actually belongs to *any* encounter finds
/// whichever one most recently had real activity, not just whichever
/// was opened most recently. Short-circuits near-instantly in the
/// common case (the most recent row almost always belongs to whatever's
/// actively being fought). None before any real encounter exists yet
/// this session.
/// Only fights the player is actually part of (`involves_you`) --
/// someone else's fight nearby is parsed as an encounter in the backend
/// but never becomes "the current encounter" the overlay shows.
pub fn current_encounter(ing: &Ingest) -> Option<&Encounter> {
    // why: an OPEN fight you're in always wins -- a loot or XP row that
    // lands on an already-closed fight (a corpse looted after its fight
    // ended, a late kill credit) must not drag the meter back to that
    // dead fight for a poll ("deaths are still closing it out ... the
    // dps graph is resetting"; live_meter_trace). Only with nothing
    // open does the newest row's fight give the after-fight summary.
    let open_id = (0..ing.store.len())
        .rev()
        .map(|i| ing.store.enc[i])
        .find(|&e| {
            e != NO_ENCOUNTER
                && ing
                    .store
                    .encounter(EncounterId(e))
                    .is_some_and(|enc| enc.involves_you && enc.is_open())
        });
    // why: nothing open -- the fight that ENDED most recently, not the
    // one that most recently got a row: a corpse looted late writes a
    // row onto an old fight and would swap the after-fight summary
    let latest_id = match open_id {
        Some(id) => id,
        None => {
            ing.store
                .encounters
                .iter()
                .filter(|e| e.involves_you && !e.absorbed)
                .max_by_key(|e| (e.end_ms.unwrap_or(e.start_ms), e.id.0))?
                .id
                .0
        }
    };
    let enc = ing.store.encounter(EncounterId(latest_id))?;
    // why: a fight that ended at (or before) the last zone line was left
    // behind -- an evac cast to reset an encounter, a zone-out. Reported
    // directly: "doesn't clear out if an evac is cast". A fight that
    // ended on its own (kill/wipe) keeps its summary as before.
    let left_behind = ing
        .last_zone_enter_ms
        .is_some_and(|z| enc.end_ms.is_some_and(|e| e <= z));
    (!left_behind).then_some(enc)
}

/// why: Skill Tracker's target-effects section -- current_encounter's
/// own "most recently ACTIVE, whole store" resolution is exactly right
/// for a DPS meter, but wrong for a feature scoped to one specific
/// entity: real bug, caught live, group content -- whichever mob a
/// party member (not necessarily "You") is actively hitting keeps
/// winning current_encounter's own backward scan, starving out the
/// mob "You" are actually casting debuffs on. This scans the same way
/// but filtered to rows that actually name `target_sym` (either side),
/// so it finds that entity's own most recent encounter regardless of
/// what the rest of the group is doing.
pub fn encounter_for(ing: &Ingest, target_sym: Sym) -> Option<&Encounter> {
    let id = (0..ing.store.len())
        .rev()
        .find(|&i| {
            ing.store.enc[i] != NO_ENCOUNTER
                && (ing.store.actor[i] == target_sym || ing.store.target[i] == target_sym)
        })
        .map(|i| ing.store.enc[i])?;
    ing.store.encounter(EncounterId(id))
}

/// why: overlay's live poll. Open engagement -> rolling window at "now"
/// (self-corrects in a lull). Closed -> window frozen at the end, the
/// whole engagement, so the summary doesn't decay to 0 after the fight.
/// None before any encounter exists yet this session.
pub fn live_meter(ing: &Ingest) -> Option<LiveMeterDto> {
    let now = ing.now_ms();
    let primary = current_encounter(ing)?;
    // why: the engagement is EVERY fight of yours that overlapped the
    // current one in time, open or closed -- the graph can hold one
    // fight per mob of a pull, and a union of only OPEN fights dropped a
    // mob's damage the moment its fight closed after its death ("you are
    // culling ... the dps should be shown as a compendium of the
    // encounter"). Grown to a fixpoint so a chain of overlaps folds in.
    let last_of = |e: &Encounter| -> Millis {
        e.end_ms.unwrap_or_else(|| {
            ing.store
                .ts
                .get(e.last as usize)
                .copied()
                .unwrap_or(e.start_ms)
                .max(now)
        })
    };
    let mut encs: Vec<&Encounter> = vec![primary];
    let (mut span_start, mut span_end) = (primary.start_ms, last_of(primary));
    loop {
        let mut grew = false;
        for e in &ing.store.encounters {
            if !e.involves_you || e.absorbed || encs.iter().any(|x| x.id == e.id) {
                continue;
            }
            let (s, l) = (e.start_ms, last_of(e));
            if s <= span_end && l >= span_start {
                encs.push(e);
                span_start = span_start.min(s);
                span_end = span_end.max(l);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let open = encs.iter().any(|e| e.is_open());
    let end = if open {
        now
    } else {
        encs.iter()
            .filter_map(|e| e.end_ms)
            .max()
            .unwrap_or(now)
            .max(primary.start_ms)
    };

    // why: per-entity accumulation straight off every engagement
    // encounter's damage rows, sided per row by allegiance AT THAT
    // ROW'S ts (charm flips mid-fight side correctly). Outgoing = ally
    // actor hitting an enemy target; incoming = enemy actor hitting an
    // ally. ONE clock for the whole engagement: from the player's own
    // first involvement -- a hit You dealt, or one that landed on You
    // -- to the live edge; the encounter's open if You never acted.
    struct Acc {
        total: u64,
        first_ts: Millis,
        is_player: bool,
        is_pet: bool,
    }
    let mut out_acc: HashMap<String, Acc> = HashMap::new();
    let mut in_acc: HashMap<String, Acc> = HashMap::new();
    let mut you_first: Option<Millis> = None;
    let mut enemies: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut current_target: Option<(Millis, String)> = None;
    for enc in &encs {
        for i in enc.range() {
            if ing.store.enc[i] != enc.id.0 || ing.store.kind[i] != EventKind::Damage {
                continue;
            }
            let ts = ing.store.ts[i];
            let actor_name = ing.effective_name(ing.store.name(ing.store.actor[i]));
            let target_name = ing.store.name(ing.store.target[i]).to_string();
            if actor_name.eq_ignore_ascii_case("You") || target_name.eq_ignore_ascii_case("You") {
                you_first = Some(you_first.map_or(ts, |f: Millis| f.min(ts)));
            }
            let actor_enemy = ing.allegiance_at(&actor_name, ts).is_enemy();
            let target_enemy = ing.allegiance_at(&target_name, ts).is_enemy();
            let acc = if !actor_enemy && target_enemy {
                enemies.insert(target_name.to_lowercase());
                if actor_name.eq_ignore_ascii_case("You")
                    && current_target.as_ref().is_none_or(|(t, _)| ts >= *t)
                {
                    current_target = Some((ts, target_name.clone()));
                }
                &mut out_acc
            } else if actor_enemy && !target_enemy {
                enemies.insert(actor_name.to_lowercase());
                if target_name.eq_ignore_ascii_case("You")
                    && current_target.as_ref().is_none_or(|(t, _)| ts >= *t)
                {
                    current_target = Some((ts, actor_name.clone()));
                }
                &mut in_acc
            } else {
                // ally-on-ally or enemy-on-enemy -- not meter damage
                continue;
            };
            let kind = ing.effective_kind(&actor_name, ts);
            let e = acc.entry(actor_name).or_insert(Acc {
                total: 0,
                first_ts: ts,
                is_player: kind == Kind::Player,
                is_pet: kind == Kind::Pet,
            });
            e.total += ing.store.amount[i];
            e.first_ts = e.first_ts.min(ts);
        }
    }
    let start_ms = you_first.unwrap_or(primary.start_ms).min(end);
    let duration_ms = (end - start_ms).max(1);

    let build = |acc: HashMap<String, Acc>| -> Vec<LiveMeterRowDto> {
        let team_total: u64 = acc.values().map(|a| a.total).sum();
        let mut rows: Vec<LiveMeterRowDto> = acc
            .into_iter()
            .map(|(name, a)| {
                // why: time in encounter -- this entity's first action to
                // the live edge; DPS runs over it
                let active_ms = (end - a.first_ts).max(1);
                LiveMeterRowDto {
                    pct: if team_total > 0 {
                        100.0 * a.total as f64 / team_total as f64
                    } else {
                        0.0
                    },
                    dps: a.total as f64 / (active_ms as f64 / 1000.0),
                    total: a.total,
                    active_ms,
                    is_player: a.is_player,
                    is_pet: a.is_pet,
                    name,
                }
            })
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.total));
        rows
    };

    // why: the label names the primary anchor; "+N" says other mobs'
    // encounters are folded into this same engagement
    let extra = encs.len() - 1;
    let target = if extra > 0 {
        format!("{} +{extra}", ing.store.name(primary.target))
    } else {
        ing.store.name(primary.target).to_string()
    };

    let ally_count = out_acc.len();
    let enemy_count = enemies.len();
    Some(LiveMeterDto {
        target,
        open,
        ally_count,
        enemy_count,
        current_target: current_target.map(|(_, n)| n),
        start_ms,
        duration_ms,
        outgoing: build(out_acc),
        incoming: build(in_acc),
    })
}

// ---------------------------------------------------------------- class detection

#[derive(Debug, Clone, Serialize)]
pub struct ClassConfigurationDto {
    /// why: alphabetical, always exactly CLASS_COUNT long -- no smaller real configuration above level 10
    pub classes: Vec<String>,
    /// why: plain count of visits resolving to this configuration, not a confidence score
    pub zone_visits: usize,
    /// why: (lowest, highest) across zone visits from real level.up lines
    /// inside them; None if no ding landed. A range since a class swap
    /// drops effective level with no line marking it
    pub level_range: Option<(u8, u8)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassConfigurationsDto {
    pub configurations: Vec<ClassConfigurationDto>,
    /// why: visits with incomplete evidence subset-elimination couldn't
    /// fold in; a plain count, no smaller real configuration exists to show
    pub unresolved_visits: usize,
}

/// why: real bug, caught live -- every visit sharing the same 3-class
/// set used to fold into one bucket regardless of how far apart in
/// real time they were, so the frontend's own max-per-class level
/// display (character.ts's applyEstimatedLevels) attributed a much
/// later, much higher-level revisit's own level_range to every class in
/// an early, brief try of the same trio. Confirmed live on a real
/// class-tourism character: gaps within one real play session (3.3h,
/// 6.5h) vs. a later revisit of the same trio (25h/74h/95h -- always a
/// different calendar day) split cleanly with real data, not a guess --
/// see the split it actually produces in this file's own tests.
const SESSION_GAP_MS: Millis = 24 * 60 * 60 * 1000;

/// why: splits one configuration's visits into real, time-contiguous
/// sessions -- a visit with no real timestamp (`None`, before the
/// first zone.enter) has nothing to compare against, so it's just
/// attached to the earliest real session (or its own, if there isn't
/// one); it never affects level_range_for's own output either way.
fn split_into_sessions(
    ing: &Ingest,
    visits: &[eqlp_session::classdetect::ZoneVisit],
    gap_ms: Millis,
) -> Vec<Vec<eqlp_session::classdetect::ZoneVisit>> {
    type ZoneVisit = eqlp_session::classdetect::ZoneVisit;
    let has_untimed = visits.contains(&None);
    let mut timed: Vec<(Millis, Millis, ZoneVisit)> = visits
        .iter()
        .filter_map(|&v| {
            let i = v?;
            let (start, next) = ing.zone.bounds(i)?;
            Some((start, next.unwrap_or(start), v))
        })
        .collect();
    timed.sort_by_key(|&(start, ..)| start);

    let mut sessions: Vec<Vec<ZoneVisit>> = Vec::new();
    let mut last_end: Option<Millis> = None;
    for (start, end, v) in timed {
        let starts_new_session = match last_end {
            Some(prev_end) => start - prev_end > gap_ms,
            None => true,
        };
        if starts_new_session {
            sessions.push(Vec::new());
        }
        sessions.last_mut().expect("just pushed if new").push(v);
        last_end = Some(end);
    }
    if has_untimed {
        match sessions.first_mut() {
            Some(first) => first.insert(0, None),
            None => sessions.push(vec![None]),
        }
    }
    sessions
}

/// why: a list of configurations, not one rolling combination -- a
/// rarely-used loadout is just as real as the most-played one; empty
/// only if never landed a single unambiguous recognized cast. Each
/// real, time-separate session of the same 3-class set is its own row
/// -- see SESSION_GAP_MS's own doc.
pub fn class_configurations(ing: &Ingest, name: &str) -> ClassConfigurationsDto {
    let Some(sym) = ing.store.names.get(name) else {
        return ClassConfigurationsDto {
            configurations: Vec::new(),
            unresolved_visits: 0,
        };
    };
    let (resolved, unresolved) = ing.classes.visits_by_resolved_configuration(sym.0);
    let mut configurations: Vec<ClassConfigurationDto> = Vec::new();
    for (classes, visits) in resolved {
        for session_visits in split_into_sessions(ing, &visits, SESSION_GAP_MS) {
            let level_range = level_range_for(ing, &session_visits);
            configurations.push(ClassConfigurationDto {
                classes: classes.clone(),
                zone_visits: session_visits.len(),
                level_range,
            });
        }
    }
    // why: same "most-played first" ordering visits_by_resolved_configuration
    // itself used before splitting -- otherwise a class-set's several
    // session-rows would land wherever they happened to be pushed
    configurations.sort_by(|a, b| {
        b.zone_visits
            .cmp(&a.zone_visits)
            .then_with(|| a.classes.cmp(&b.classes))
    });
    ClassConfigurationsDto {
        configurations,
        unresolved_visits: unresolved.len(),
    }
}

/// why: real level.up lines fired inside the visit only, never a
/// boundary snapshot -- an earlier version sampled Levels::at the visit
/// boundary, confirmed wrong: a config swap drops effective level
/// silently, so the start sample could belong to the previous
/// configuration entirely. A visit with no ding contributes nothing.
fn level_range_for(
    ing: &Ingest,
    visits: &[eqlp_session::classdetect::ZoneVisit],
) -> Option<(u8, u8)> {
    let levels: Vec<u8> = visits
        .iter()
        .filter_map(|&v| v)
        .filter_map(|i| ing.zone.bounds(i))
        .flat_map(|(start, next_start)| ing.levels.between(start, next_start))
        .collect();
    let min = levels.iter().copied().min()?;
    let max = levels.iter().copied().max()?;
    Some((min, max))
}

/// why: drills from one configuration row down to its specific visits; empty if no match
/// why: `level_range` disambiguates which session-row -- since
/// SESSION_GAP_MS's own split, more than one row can share the same
/// `classes` (separate real sessions of the same trio), so `classes`
/// alone no longer picks a unique row the way it used to.
pub fn zone_visits_for_configuration(
    ing: &Ingest,
    name: &str,
    classes: &[String],
    level_range: Option<(u8, u8)>,
) -> Vec<ZoneVisitDto> {
    let Some(sym) = ing.store.names.get(name) else {
        return Vec::new();
    };
    let (resolved, _) = ing.classes.visits_by_resolved_configuration(sym.0);
    let Some((_, visits)) = resolved.into_iter().find(|(c, _)| c.as_slice() == classes) else {
        return Vec::new();
    };
    let wanted = split_into_sessions(ing, &visits, SESSION_GAP_MS)
        .into_iter()
        .find(|session| level_range_for(ing, session) == level_range)
        .unwrap_or_default();
    let mut out: Vec<ZoneVisitDto> = zone_visit_dtos(ing)
        .into_iter()
        .filter(|dto| wanted.contains(&dto.index))
        .collect();
    sort_zone_visits(&mut out);
    out
}

#[cfg(test)]
mod live_meter_tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;

    fn run(text: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing
    }

    #[test]
    fn no_encounters_yet_is_none_not_a_panic() {
        let ing = run("");
        assert!(live_meter(&ing).is_none());
    }

    #[test]
    fn a_real_fight_splits_outgoing_from_incoming() {
        // why: "You" isn't Kind::Player until proven (a player-only chat
        // channel, or damaging a mob "You" also damaged) -- solo melee
        // alone never proves it, so a real chat line comes first
        let ing = run("[Tue Jul 28 15:00:00 2026] You tell your party, 'ready'\n\
             [Tue Jul 28 15:01:00 2026] You hit Refugee Splitpaw for 10 points of damage.\n\
             [Tue Jul 28 15:01:01 2026] Refugee Splitpaw hits You for 4 points of damage.\n");
        let m = live_meter(&ing).expect("a real encounter should exist");
        assert_eq!(m.target, "Refugee Splitpaw");
        assert!(m.open, "no death/reset line yet -- still open");
        assert!(
            m.outgoing.iter().any(|r| r.name == "You" && r.is_player),
            "{:?}",
            m.outgoing
        );
        // why: sides are structural now -- outgoing IS the ally side by
        // construction, so the old is_enemy flag has nothing to say
        assert!(
            m.incoming
                .iter()
                .any(|r| r.name.eq_ignore_ascii_case("Refugee Splitpaw") && r.dps > 0.0),
            "{:?}",
            m.incoming
        );
    }

    /// why: real bug, caught live -- a stray hit against an unrelated
    /// target ("Consetta") opened its own encounter with a later
    /// start_ms than a real fight that was still actively landing
    /// damage, and the old max_by_key(start_ms) pick chose the newer-
    /// but-dead encounter over the real ongoing one. The real fight's
    /// own most recent row is later than the stray encounter's only
    /// row, which is what should decide it, not which encounter object
    /// was opened more recently.
    #[test]
    fn the_encounter_with_more_recent_real_activity_wins_even_with_an_earlier_start() {
        let ing = run("[Tue Jul 28 15:00:00 2026] You tell your party, 'ready'\n\
             [Tue Jul 28 15:01:00 2026] You hit Innoruuk for 10 points of damage.\n\
             [Tue Jul 28 15:02:00 2026] You hit Consetta for 3 points of damage.\n\
             [Tue Jul 28 15:03:00 2026] You hit Innoruuk for 12 points of damage.\n");
        let m = live_meter(&ing).expect("a real encounter should exist");
        assert_eq!(
            m.target, "Innoruuk",
            "the still-active fight should win, not the more-recently-opened stray encounter"
        );
    }
}

#[cfg(test)]
mod session_split_tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;
    use eqlp_session::classdetect::ZoneVisit;

    fn run(text: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing
    }

    /// why: visits close together in real time stay one session
    #[test]
    fn visits_within_the_gap_stay_one_session() {
        let text = "\
[Tue Jul 28 15:00:00 2026] You have entered The Estate of Unrest.
[Tue Jul 28 15:10:00 2026] You have entered North Qeynos.
[Tue Jul 28 15:20:00 2026] You have entered West Karana.
";
        let ing = run(text);
        let visits: Vec<ZoneVisit> = vec![Some(0), Some(1), Some(2)];
        let gap_ms = 15 * 60 * 1000; // 15 min -- wider than any real gap here
        let sessions = split_into_sessions(&ing, &visits, gap_ms);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].len(), 3);
    }

    /// why: real bug fix's whole point -- a gap bigger than the threshold splits
    /// one class-set's visits into separate rows instead of one continuous arc.
    /// why the Freeport visit: a visit's own "end" (bounds().1) is defined as
    /// the *next raw zone.enter's* start, whatever it is -- so this subset's
    /// own visit 1 needs some other, unlisted zone right after it to close
    /// off its real bound before the big gap; without it, visit 1's end would
    /// be defined as visit 3's own start, making the gap disappear by
    /// construction rather than ever being compared against the threshold.
    #[test]
    fn a_gap_bigger_than_the_threshold_starts_a_new_session() {
        let text = "\
[Tue Jul 28 15:00:00 2026] You have entered The Estate of Unrest.
[Tue Jul 28 15:10:00 2026] You have entered North Qeynos.
[Tue Jul 28 15:20:00 2026] You have entered Freeport.
[Wed Jul 29 20:00:00 2026] You have entered West Karana.
";
        let ing = run(text);
        let visits: Vec<ZoneVisit> = vec![Some(0), Some(1), Some(3)]; // visit 2 (Freeport) not part of this configuration
        let gap_ms = 15 * 60 * 1000; // 15 min
        let sessions = split_into_sessions(&ing, &visits, gap_ms);
        assert_eq!(sessions.len(), 2);
        assert_eq!(
            sessions[0],
            vec![Some(0), Some(1)],
            "first two visits, 10 min apart, stay together"
        );
        assert_eq!(
            sessions[1],
            vec![Some(3)],
            "the next-day visit starts its own session"
        );
    }

    /// why: sessions come out ordered by real time, not by input order
    #[test]
    fn sessions_are_ordered_by_real_time_not_input_order() {
        let text = "\
[Tue Jul 28 15:00:00 2026] You have entered The Estate of Unrest.
[Tue Jul 28 15:10:00 2026] You have entered North Qeynos.
";
        let ing = run(text);
        let visits: Vec<ZoneVisit> = vec![Some(1), Some(0)]; // reversed on purpose
        let gap_ms = 15 * 60 * 1000;
        let sessions = split_into_sessions(&ing, &visits, gap_ms);
        assert_eq!(sessions, vec![vec![Some(0), Some(1)]]);
    }

    /// why: a visit with no real timestamp of its own (e.g. the log starts
    /// mid-visit) has nothing to compare against -- it attaches to the
    /// earliest real session rather than getting lost or forming its own
    #[test]
    fn an_untimed_visit_attaches_to_the_earliest_session() {
        let text = "\
[Tue Jul 28 15:00:00 2026] You have entered The Estate of Unrest.
[Tue Jul 28 15:05:00 2026] You have entered Freeport.
[Wed Jul 29 20:00:00 2026] You have entered North Qeynos.
";
        let ing = run(text);
        let visits: Vec<ZoneVisit> = vec![Some(0), Some(2), None]; // visit 1 (Freeport) not part of this configuration
        let gap_ms = 15 * 60 * 1000;
        let sessions = split_into_sessions(&ing, &visits, gap_ms);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0], vec![None, Some(0)]);
        assert_eq!(sessions[1], vec![Some(2)]);
    }

    /// why: an untimed-only visit list is still one real session, not zero
    #[test]
    fn an_untimed_only_visit_list_is_still_one_session() {
        let ing = Ingest::default();
        let visits: Vec<ZoneVisit> = vec![None];
        let sessions = split_into_sessions(&ing, &visits, 1000);
        assert_eq!(sessions, vec![vec![None]]);
    }
}

#[cfg(test)]
mod level_range_tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;
    use eqlp_session::classdetect::ZoneVisit;

    fn run(text: &str, seed_level: u8) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        ing.levels.observe(0, seed_level);
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing
    }

    /// why: a ding inside the visit that fired it is real evidence, regardless of what's later
    #[test]
    fn a_mid_visit_ding_is_captured() {
        let text = "\
[Tue Jul 28 15:00:00 2026] You have entered The Estate of Unrest.
[Tue Jul 28 15:30:00 2026] You have gained a level! Welcome to level 46!
[Tue Jul 28 16:00:00 2026] You have entered North Qeynos.
";
        let ing = run(text, 45);
        let visits: Vec<ZoneVisit> = vec![Some(0)];
        let range = level_range_for(&ing, &visits).expect("the 15:30 ding is real evidence");
        assert_eq!(range, (46, 46));
    }

    /// why: real bug fix -- a level seeded before the visit started must not leak in
    #[test]
    fn a_level_from_before_the_visit_started_does_not_leak_in() {
        let text = "\
[Tue Jul 28 15:00:00 2026] You have entered The Estate of Unrest.
[Tue Jul 28 15:30:00 2026] You have gained a level! Welcome to level 46!
";
        let ing = run(text, 45);
        let visits: Vec<ZoneVisit> = vec![Some(0)];
        let range = level_range_for(&ing, &visits).expect("the 15:30 ding is real evidence");
        assert_eq!(
            range,
            (46, 46),
            "45 was never observed *during* this visit -- only 46 was"
        );
    }

    /// why: a visit with no ding contributes nothing, not an inherited level
    #[test]
    fn a_visit_with_no_ding_of_its_own_contributes_no_evidence() {
        let text = "[Tue Jul 28 15:00:00 2026] You have entered The Estate of Unrest.\n";
        let ing = run(text, 45);
        let visits: Vec<ZoneVisit> = vec![Some(0)];
        assert_eq!(level_range_for(&ing, &visits), None);
    }

    /// why: real bug reproduced -- a later visit's own dings must never inherit an earlier one's
    #[test]
    fn a_ding_from_an_earlier_unrelated_visit_never_leaks_into_a_later_one() {
        let text = "\
[Tue Aug 18 17:10:20 2026] You have gained a level! Welcome to level 29!
[Tue Aug 18 17:53:45 2026] You have gained a level! Welcome to level 30!
[Tue Aug 18 20:18:19 2026] You have entered Befallen.
[Tue Aug 18 20:25:06 2026] You have gained a level! Welcome to level 11!
[Tue Aug 18 20:33:02 2026] You have gained a level! Welcome to level 12!
[Tue Aug 18 21:33:53 2026] You have gained a level! Welcome to level 18!
[Tue Aug 18 21:34:52 2026] You have entered West Karana.
";
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);

        // why: Befallen (index 0) is the only zone visit here
        let visits: Vec<ZoneVisit> = vec![Some(0)];
        let range =
            level_range_for(&ing, &visits).expect("3 real dings happened inside this visit");
        assert_eq!(
            range,
            (11, 18),
            "30 belongs to whatever was active before this visit opened, not this one"
        );
    }
}

#[cfg(test)]
mod list_encounters_paging_tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;

    /// why: 6 real-shaped kills, one minute apart, distinct targets for paging by name
    fn six_kills() -> Ingest {
        let mut text = String::new();
        for i in 1..=6 {
            text.push_str(&format!(
                "[Tue Jul 28 15:0{i}:00 2026] You hit a target {i} for 5 points of fire damage by Burst of Flame.\n\
                 [Tue Jul 28 15:0{i}:01 2026] You have slain a target {i}!\n",
            ));
        }
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing
    }

    #[test]
    fn a_page_is_newest_first_and_the_right_size() {
        let ing = six_kills();
        let page = list_encounters(&ing, None, 0, 3);
        assert_eq!(page.len(), 3);
        // why: target 6 died last, newest first means it's page one
        assert_eq!(page[0].target, "a target 6");
        assert_eq!(page[1].target, "a target 5");
        assert_eq!(page[2].target, "a target 4");
    }

    #[test]
    fn the_next_page_picks_up_where_the_first_left_off() {
        let ing = six_kills();
        let page2 = list_encounters(&ing, None, 3, 3);
        assert_eq!(page2.len(), 3);
        assert_eq!(page2[0].target, "a target 3");
        assert_eq!(page2[1].target, "a target 2");
        assert_eq!(page2[2].target, "a target 1");
    }

    #[test]
    fn offset_past_the_end_is_empty_not_an_error() {
        let ing = six_kills();
        assert!(list_encounters(&ing, None, 100, 10).is_empty());
    }

    #[test]
    fn zero_limit_is_empty() {
        let ing = six_kills();
        assert!(list_encounters(&ing, None, 0, 0).is_empty());
    }

    #[test]
    fn a_huge_limit_still_returns_only_what_actually_exists() {
        let ing = six_kills();
        assert_eq!(list_encounters(&ing, None, 0, usize::MAX).len(), 6);
    }

    /// why: two adjacent pages concatenated must equal one big page, no fight lost or duplicated
    #[test]
    fn two_pages_back_to_back_cover_the_same_ground_as_one_big_page() {
        let ing = six_kills();
        let whole = list_encounters(&ing, None, 0, 6);
        let mut paged = list_encounters(&ing, None, 0, 3);
        paged.extend(list_encounters(&ing, None, 3, 3));
        let whole_ids: Vec<u32> = whole.iter().map(|e| e.id).collect();
        let paged_ids: Vec<u32> = paged.iter().map(|e| e.id).collect();
        assert_eq!(whole_ids, paged_ids);
    }
}

#[cfg(test)]
mod outcome_tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;

    /// why: a fight stays open without a later line to force idle-close;
    /// every case gets one trailing filler line past the 10s idle timeout
    fn ingest_from(text: &str) -> Ingest {
        let mut text = text.to_string();
        text.push_str("[Tue Jul 28 15:01:30 2026] You hit a filler target for 1 points of fire damage by Burst of Flame.\n");
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing.tick(ing.now_ms());
        ing
    }

    /// why: the filler line is its own encounter -- find by name not position
    fn find<'a>(list: &'a [EncounterDto], target: &str) -> &'a EncounterDto {
        list.iter()
            .find(|e| e.target == target)
            .expect("target should have its own encounter")
    }

    /// why: real evac shape -- cast, LOADING, then a zone line (even for
    /// the same zone). The fight it closes must not linger as "current".
    #[test]
    fn an_evac_leaves_the_fight_behind() {
        let text = "[Tue Jul 28 15:01:00 2026] You hit a target 1 for 5 points of fire damage by Burst of Flame.\n\
             [Tue Jul 28 15:01:02 2026] A target 1 hits YOU for 12 points of damage.\n\
             [Tue Jul 28 15:01:05 2026] You begin casting Lesser Evacuate.\n\
             [Tue Jul 28 15:01:14 2026] LOADING, PLEASE WAIT...\n\
             [Tue Jul 28 15:01:20 2026] You have entered The Ruins of Old Paineel.\n";
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing.tick(ing.now_ms());
        assert!(
            current_encounter(&ing).is_none(),
            "the evac'd fight must not be current"
        );
        assert!(live_meter(&ing).is_none());
    }

    /// why: the counter-case -- a fight that ended on its own keeps its
    /// summary (the after-fight read), zone line or not before it
    #[test]
    fn a_kill_after_zoning_in_still_shows() {
        let text = "[Tue Jul 28 15:00:00 2026] You have entered The Ruins of Old Paineel.\n\
             [Tue Jul 28 15:01:00 2026] You hit a target 1 for 5 points of fire damage by Burst of Flame.\n\
             [Tue Jul 28 15:01:01 2026] You have slain a target 1!\n";
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = text.lines().map(str::as_bytes).collect();
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing.tick(ing.now_ms());
        assert!(current_encounter(&ing).is_some());
    }

    #[test]
    fn a_real_kill_is_slain_not_wiped() {
        let ing = ingest_from(
            "[Tue Jul 28 15:01:00 2026] You hit a target 1 for 5 points of fire damage by Burst of Flame.\n\
             [Tue Jul 28 15:01:01 2026] You have slain a target 1!\n",
        );
        let list = list_encounters(&ing, None, 0, usize::MAX);
        let e = find(&list, "a target 1");
        assert!(e.slain);
        assert!(!e.wiped);
    }

    /// why: death.you_died used to land in the same slain list as a real kill
    #[test]
    fn dying_to_a_mob_that_survives_is_wiped_not_slain() {
        let ing = ingest_from(
            "[Tue Jul 28 15:01:00 2026] a rock golem slashes You for 20 points of damage.\n\
             [Tue Jul 28 15:01:01 2026] You have been slain by a rock golem!\n",
        );
        let list = list_encounters(&ing, None, 0, usize::MAX);
        let e = find(&list, "a rock golem");
        assert!(
            !e.slain,
            "the target survived -- this must not read as a kill"
        );
        assert!(e.wiped);
    }

    /// why: player's own correction -- "a death doesnt mean a wipe". A
    /// proven groupmate (or a charm pet) dying in a fight with no
    /// confirmed enemy kill used to tag the whole fight "wipe"; only
    /// ~21% of real kills get a confirmed death line at all, so this
    /// mislabeled constantly. An ally death is a death inside the
    /// fight, not the fight's own classification -- only the log
    /// owner's own death makes a no-kill fight a wipe.
    #[test]
    fn an_allys_death_without_a_kill_is_a_reset_not_a_wipe() {
        let ing = ingest_from(
            // why: the chat line proves Dippinsauce is a real player (ally)
            "[Tue Jul 28 15:00:00 2026] Dippinsauce tells the group, 'inc'\n\
             [Tue Jul 28 15:01:00 2026] Dippinsauce slashes a rock golem for 20 points of damage.\n\
             [Tue Jul 28 15:01:01 2026] a rock golem slashes Dippinsauce for 90 points of damage.\n\
             [Tue Jul 28 15:01:02 2026] Dippinsauce has been slain by a rock golem!\n",
        );
        let list = list_encounters(&ing, None, 0, usize::MAX);
        let e = find(&list, "a rock golem");
        assert!(!e.slain, "no enemy kill was confirmed");
        assert!(
            !e.wiped,
            "a groupmate's death alone must not read as a wipe -- You didn't die"
        );
    }
}

#[cfg(test)]
mod ability_mitigation_dto_tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;

    /// why: `.damage()` excludes Miss outright -- a mitigated swing must
    /// merge in from a second `.kind(EventKind::Miss)` query
    #[test]
    fn a_mitigated_swing_reaches_the_ability_dto_on_the_actor_s_own_row() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You punch a target for 5 points of damage.",
            b"[Tue Jul 28 15:01:01 2026] You try to punch a target, but a target blocks!",
            b"[Tue Jul 28 15:01:02 2026] a target hits YOU for 3 points of damage.",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let summary = summarize(&ing, None, None, Some("You"), false);
        let punch = summary
            .abilities
            .iter()
            .find(|a| a.ability == "Punch")
            .expect("a Punch row should reach the DTO");
        assert_eq!(punch.hits, 1);
        assert_eq!(punch.total, 5);
        assert_eq!(
            punch.blocked, 1,
            "the mitigated swing must reach the DTO too"
        );
        assert_eq!(punch.missed, 0);
        assert_eq!(punch.dodged, 0);
        assert_eq!(punch.parried, 0);
    }

    /// why: team-aggregate path must subtract the target's mitigated swings too
    #[test]
    fn team_aggregate_excludes_the_target_s_own_mitigated_swings() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You punch a target for 5 points of damage.",
            b"[Tue Jul 28 15:01:01 2026] You try to punch a target, but a target blocks!",
            // why: the target's own swing, dodged -- must not pollute the team's Punch row
            b"[Tue Jul 28 15:01:02 2026] a target tries to punch YOU, but YOU dodge!",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let summary = summarize(&ing, None, None, None, false);
        let punch = summary
            .abilities
            .iter()
            .find(|a| a.ability == "Punch")
            .expect("a Punch row should reach the DTO");
        assert_eq!(punch.hits, 1);
        assert_eq!(punch.blocked, 1);
        assert_eq!(
            punch.dodged, 0,
            "the target's own dodged swing belongs to them, not the team"
        );
    }
}

/// why: real ask -- a copy-out report needs accuracy/resist/enemy-heal
/// alongside totals, not just what AllyTable already showed
#[cfg(test)]
mod ally_report_tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;

    fn run(lines: &[&str]) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let bytes: Vec<&[u8]> = lines.iter().map(|l| l.as_bytes()).collect();
        backfill_lines(&mut ing, &engine, &bytes, 1);
        ing
    }

    #[test]
    fn a_landed_hit_and_a_blocked_swing_average_to_fifty_percent_hit_rate() {
        let ing = run(&[
            // why: proves "You" as Kind::Player -- Unproven defaults to
            // Enemy allegiance (see Allegiance::of), so list_allies would
            // otherwise skip "You" entirely
            "[Tue Jul 28 15:00:00 2026] You tell your party, 'ready'",
            "[Tue Jul 28 15:01:00 2026] You punch a target for 5 points of damage.",
            "[Tue Jul 28 15:01:01 2026] You try to punch a target, but a target blocks!",
        ]);
        let allies = list_allies(&ing, None, None, false);
        let you = allies
            .iter()
            .find(|a| a.name == "You")
            .expect("You should be an ally row");
        assert_eq!(you.hit_pct, Some(50.0));
        assert_eq!(
            you.resist_pct, None,
            "no casts at all -- must stay None, not 0%"
        );
    }

    #[test]
    fn a_resisted_cast_and_a_landed_cast_average_to_fifty_percent_resist_rate() {
        let ing = run(&[
            "[Tue Jul 28 15:00:00 2026] You tell your party, 'ready'",
            "[Tue Jul 28 15:01:00 2026] You begin casting Lifetap.",
            "[Tue Jul 28 15:01:02 2026] You hit a target for 10 points of magic damage by Lifetap.",
            "[Tue Jul 28 15:01:03 2026] You begin casting Lifetap.",
            "[Tue Jul 28 15:01:05 2026] a target resisted your Lifetap!",
        ]);
        let allies = list_allies(&ing, None, None, false);
        let you = allies
            .iter()
            .find(|a| a.name == "You")
            .expect("You should be an ally row");
        assert_eq!(you.resist_pct, Some(50.0));
    }

    #[test]
    fn a_self_heal_on_the_target_reaches_the_summary_as_enemy_heal() {
        let ing = run(&[
            "[Tue Jul 28 15:01:00 2026] You punch a target for 5 points of damage.",
            "[Tue Jul 28 15:01:01 2026] a target healed itself for 20 hit points by Lifetap.",
        ]);
        let summary = summarize(&ing, None, None, None, false);
        assert_eq!(
            summary.enemy_heal, 20,
            "healing landed on the target, not folded into total_damage"
        );
    }
}

#[cfg(test)]
mod live_meter_window_tests {
    use super::*;
    use crate::ingest::{backfill_lines, framed_lines, Ingest};
    use crate::parser::build_engine;

    fn ingest_from(text: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines = framed_lines(text.as_bytes());
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing
    }

    /// why: the spec -- the encounter's own timer runs from the player's
    /// first involvement; each row carries its time in the encounter and
    /// DPS over that; totals accumulate over the whole encounter
    #[test]
    fn rows_carry_time_in_encounter_under_the_encounters_timer() {
        let ing = ingest_from(
            "[Tue Jul 28 15:01:00 2026] Kaeus tells the group, 'hi'\n\
             [Tue Jul 28 15:01:00 2026] You hit a gnoll for 100 points of fire damage by Burst of Flame.\n\
             [Tue Jul 28 15:01:20 2026] Kaeus hits a gnoll for 100 points of damage.\n\
             [Tue Jul 28 15:01:40 2026] You hit a gnoll for 100 points of fire damage by Burst of Flame.\n\
             [Tue Jul 28 15:01:40 2026] Kaeus hits a gnoll for 100 points of damage.\n",
        );
        let m = live_meter(&ing).expect("live fight");
        assert_eq!(
            m.duration_ms, 40_000,
            "encounter timer: the player's first hit to the live edge"
        );
        let you = m
            .outgoing
            .iter()
            .find(|r| r.name == "You")
            .expect("You row");
        let kaeus = m
            .outgoing
            .iter()
            .find(|r| r.name == "Kaeus")
            .expect("Kaeus row");
        assert_eq!((you.total, kaeus.total), (200, 200));
        assert_eq!((you.active_ms, kaeus.active_ms), (40_000, 20_000));
        assert!((you.dps - 5.0).abs() < 1e-9);
        assert!((kaeus.dps - 10.0).abs() < 1e-9);
    }

    /// why: "it shouldn't reset damage per entity ... so it doesn't jump
    /// back to 0 as soon as a target dies" -- a kill inside the
    /// encounter, the next mob two seconds later: one encounter, totals
    /// keep climbing, the timer keeps running
    #[test]
    fn totals_survive_a_kill_inside_the_encounter() {
        let ing = ingest_from(
            "[Tue Jul 28 15:01:00 2026] You hit a gnoll for 100 points of fire damage by Burst of Flame.\n\
             [Tue Jul 28 15:01:05 2026] You have slain a gnoll!\n\
             [Tue Jul 28 15:01:07 2026] You hit a gnoll scout for 100 points of fire damage by Burst of Flame.\n\
             [Tue Jul 28 15:01:10 2026] You hit a gnoll scout for 100 points of fire damage by Burst of Flame.\n",
        );
        let m = live_meter(&ing).expect("live fight");
        let you = m
            .outgoing
            .iter()
            .find(|r| r.name == "You")
            .expect("You row");
        assert_eq!(you.total, 300, "the kill did not reset the total");
        assert_eq!(m.duration_ms, 10_000);
        assert!(m.target.contains("gnoll"));
    }

    /// why: the live clock must not jump -- measured in the real app: a
    /// stale wall reading at the backfill->live seam pushed the log clock
    /// 12s ahead of real time, so a kill's 6s window was already past and
    /// every fight closed the instant its death line arrived ("encounter
    /// is showing as ended instantly after a kill"). One tick may advance
    /// the clock by at most MAX_TICK_ELAPSED_MS.
    #[test]
    fn a_stale_wall_reading_cannot_push_the_clock_ahead() {
        let mut ing = ingest_from(
            "[Tue Jul 28 15:01:00 2026] You hit a gnoll for 100 points of fire damage by Burst of Flame.\n\
             [Tue Jul 28 15:01:05 2026] You have slain a gnoll!\n",
        );
        let death = ing.now_ms();
        ing.mark_live();
        ing.tick(1_000_000); // baseline from a reading taken 12s ago
        ing.tick(1_012_000); // the next poll, 12s "later"
        assert!(
            ing.now_ms() - death <= 2_000,
            "clock ran {}ms past the last line",
            ing.now_ms() - death
        );
        assert!(
            current_encounter(&ing).is_some_and(|e| e.is_open()),
            "still open after the kill"
        );
        // real seconds pass, one poll at a time -- the 6s window then closes it
        for k in 1..=8 {
            ing.tick(1_012_000 + k * 1_000);
        }
        assert!(
            current_encounter(&ing).is_some_and(|e| !e.is_open()),
            "closed 6s+ after the kill"
        );
    }

    /// why: ally class inference is PER PRESENCE -- Brutall left Lower Guk
    /// mid-session and came back as a different trio; the others never
    /// zoned. Silence past the absence window, a group leave/join, or
    /// your own zone line each start the votes over.
    #[test]
    fn an_allys_class_votes_reset_when_they_come_back() {
        let ing = ingest_from(
            "[Tue Jul 28 15:00:00 2026] Brutall tells the group, 'hi'\n\
             [Tue Jul 28 15:01:00 2026] Brutall hit a gnoll for 100 points of magic damage by Ice Comet.\n\
             [Tue Jul 28 15:01:05 2026] Brutall hit a gnoll for 100 points of magic damage by Ice Comet.\n\
             [Tue Jul 28 15:20:00 2026] Brutall hit a gnoll for 100 points of magic damage by Lifetap.\n",
        );
        let visit = ing.zone.index_at(ing.now_ms());
        let (classes, votes) = ing.ally_classes("Brutall", visit);
        assert_eq!(
            votes, 1,
            "the 19-minute silence started the count over, got {classes:?}"
        );
        assert!(
            !classes.iter().any(|c| c == "Wizard"),
            "the wizard votes are gone: {classes:?}"
        );
        // why: your own zone line is a new VISIT -- the new zone starts
        // clean, and the old visit still answers for its own fights
        let ing = ingest_from(
            "[Tue Jul 28 15:00:00 2026] You have entered Upper Guk.\n\
             [Tue Jul 28 15:00:10 2026] Brutall tells the group, 'hi'\n\
             [Tue Jul 28 15:01:00 2026] Brutall begins casting Ice Comet.\n\
             [Tue Jul 28 15:01:30 2026] You have entered Lower Guk.\n\
             [Tue Jul 28 15:02:00 2026] Brutall hit a gnoll for 100 points of magic damage by Lifetap.\n",
        );
        let now_visit = ing.zone.index_at(ing.now_ms());
        let (classes, votes) = ing.ally_classes("Brutall", now_visit);
        assert_eq!(
            votes, 1,
            "the new visit has only the Lifetap vote, got {classes:?}"
        );
        let old_visit = now_visit.map(|v| v - 1);
        let (old, old_votes) = ing.ally_classes("Brutall", old_visit);
        assert_eq!(
            old_votes, 1,
            "the old visit keeps its own vote, got {old:?}"
        );
        assert!(
            old.iter().any(|c| c == "Wizard"),
            "a 'begins casting' line votes: {old:?}"
        );
    }

    /// why: incoming mirrors the calc from the enemy side
    #[test]
    fn incoming_rows_carry_the_same_shape() {
        let ing = ingest_from(
            "[Tue Jul 28 15:01:00 2026] You hit a gnoll for 100 points of fire damage by Burst of Flame.\n\
             [Tue Jul 28 15:01:10 2026] A gnoll hits YOU for 50 points of damage.\n",
        );
        let m = live_meter(&ing).expect("live fight");
        let g = m
            .incoming
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case("a gnoll"))
            .expect("gnoll row");
        assert_eq!(g.total, 50);
        assert!((g.pct - 100.0).abs() < 0.01);
    }
}

#[cfg(test)]
mod engagement_scope_tests {
    use super::*;
    use crate::ingest::{backfill_lines, framed_lines, Ingest};
    use crate::parser::build_engine;

    fn ingest_from(text: &str) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines = framed_lines(text.as_bytes());
        backfill_lines(&mut ing, &engine, &lines, 1);
        ing
    }

    /// why: asked directly -- "track for the encounter as a whole, even
    /// if other mobs join the encounter late". A second open fight
    /// against a different mob within the lull folds into the SAME
    /// meter (label says +1) instead of hijacking it; totals span both.
    #[test]
    fn a_late_add_in_its_own_encounter_folds_into_the_engagement() {
        // two encounters kept separate on purpose: fight A goes quiet
        // (no shared entity with B beyond You, whose damage came later)
        let ing = ingest_from(
            "[Tue Jul 28 15:01:00 2026] You hit a gnoll for 100 points of fire damage by Burst of Flame.\n\
             [Tue Jul 28 15:01:12 2026] A giant snake hits YOU for 25 points of damage.\n\
             [Tue Jul 28 15:01:13 2026] You hit a gnoll for 100 points of fire damage by Burst of Flame.\n",
        );
        let m = live_meter(&ing).expect("live engagement");
        let you = m
            .outgoing
            .iter()
            .find(|r| r.name == "You")
            .expect("You row");
        assert_eq!(you.total, 200, "both hits counted in one engagement");
        let snake = m
            .incoming
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case("a giant snake"));
        assert!(
            snake.is_some_and(|s| s.total == 25),
            "the add's damage folds in: {:?}",
            m.incoming
        );
    }
}

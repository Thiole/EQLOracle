//! Read-side queries over `Ingest` for the Combat module: which zone visits
//! exist, which encounters are in one, and the ability breakdown for a
//! selection -- one encounter, every encounter in a zone visit, or
//! everything parsed so far.
//!
//! No parsing happens here. Every query runs against `Store`, which already
//! holds everything `ingest::Ingest::route` has classified -- nothing is
//! reparsed to answer these.

use crate::ingest::Ingest;
use eqlp_session::{series as bucket_series, Allegiance, Cause, Kind, State};
use eqlp_source::Millis;
use eqlp_store::{
    by_ability, by_actor, dps_window, flag, tag, total, AbilityId, AbilityRow, Encounter,
    EncounterId, EventKind, Filter, Sym,
};
use serde::Serialize;
use std::collections::HashMap;

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
    /// The team's own damage output -- excludes whatever the fight's own
    /// target dealt back. A number that mixes offense and incoming damage
    /// together says nothing; see `enemy_damage`/`enemy_dps` for the other
    /// half.
    pub total_damage: u64,
    pub dps: f64,
    /// Damage the target dealt to the team during this fight.
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
    /// Average of this ability's non-crit hits. A per-ability dps figure
    /// (total / the whole fight's duration) used to live here instead, but
    /// it's a rate stat for something that isn't a steady stream --
    /// "Hit" alone bundles every weapon swing, so its dps says more about
    /// how long the fight ran than about the ability itself. Avg/avg-crit
    /// match what a real EQ parser (GamParse, ...) actually reports per
    /// skill, and stay meaningful for a rarely-used nuke the same way they
    /// do for constant auto-attack.
    pub avg_hit: f64,
    pub avg_crit: f64,
    pub pct: f64,
    /// Swings of this same attack type that dealt zero damage because
    /// they were fully avoided, broken out by how -- see
    /// `eqlp_store::flag::MITIGATED`'s own doc for why these live on this
    /// row instead of a separate synthetic ability.
    pub missed: u64,
    pub blocked: u64,
    pub dodged: u64,
    pub parried: u64,
}

/// One spell's cast attempts and how they resolved -- separate from
/// `AbilityRowDto`, not merged into it. `EventKind::Cast` rows track
/// *attempts* (one per cast, landed or not, `flag::CAST_*` for the
/// outcome); `EventKind::Damage` rows track *landed hits*, which for a DoT
/// or a multi-tick effect can be more than one per cast. Blending the two
/// into one row's `hits` would conflate "how many times did I try this"
/// with "how many times did it land damage", two different questions --
/// and a pure buff or CC spell has cast attempts but never a damage row at
/// all, so it needs somewhere to show up regardless. Requested so casting
/// that doesn't deal damage (buffs, CC, a resisted or interrupted attempt)
/// is visible in the same expanded panel as the damage breakdown, not just
/// silently absent from it.
#[derive(Debug, Clone, Serialize)]
pub struct CastRowDto {
    pub spell: String,
    pub attempts: u32,
    pub landed: u32,
    pub resisted: u32,
    pub interrupted: u32,
    pub fizzled: u32,
    /// Expired with no confirming line at all -- see
    /// `eqlp_session::cast::Resolver`'s doc for when this happens.
    pub unconfirmed: u32,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CombatSummaryDto {
    pub fight_count: usize,
    /// The team's own damage output -- excludes whatever each fight's own
    /// target dealt back. See `enemy_damage`/`enemy_dps`.
    pub total_damage: u64,
    pub duration_ms: Millis,
    /// The mean of *each fight's own* dps, not `total_damage / duration_ms`
    /// pooled across every fight in the selection. Pooling weights a
    /// selection's dps by fight length -- a handful of long grinds would
    /// dominate the number, drowning out several short, sharp fights that
    /// are just as real a data point about how the selection actually
    /// went. Averaging each fight's own rate treats every fight as one
    /// sample regardless of how long it ran. Equivalent to the pooled
    /// number for a single-fight selection (nothing to average across);
    /// only diverges once `fight_count > 1`.
    pub dps: f64,
    /// Damage the enemy (each fight's own target) dealt to the team.
    /// Grouped, not mixed into `total_damage` -- a number that combines
    /// offense and incoming damage says nothing about either.
    pub enemy_damage: u64,
    /// Same averaging-per-fight treatment as `dps`, for the same reason --
    /// this is the same kind of rate stat, just measuring incoming damage.
    pub enemy_dps: f64,
    pub abilities: Vec<AbilityRowDto>,
    /// Every spell cast in this selection and how the attempts resolved --
    /// see `CastRowDto`'s doc for why this is separate from `abilities`.
    pub casts: Vec<CastRowDto>,
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

/// Every zone visit that exists at all, unfiltered and unsorted -- shared
/// by `list_zone_visits` and `zone_visits_for_configuration` so both agree
/// on what a "fight count" means from one single scan of
/// `ing.store.encounters`, rather than two separate scans that could drift.
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

/// Newest visit first; the pre-first-zone-line "Unknown" bucket sorts last.
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

/// One fight's `EncounterDto`, computed fresh from `Store`/`Ingest` every
/// call -- nothing about a fight is cached or duplicated anywhere else in
/// memory, this just reads the same rows `list_encounters` always has.
/// Shared by `list_encounters` and `list_zone_encounters` so both build a
/// fight's numbers identically instead of two copies of this drifting.
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

/// Newest-first page of `zone_visit`'s fights, `offset` fights in, at most
/// `limit` of them. Windowed for real, not just truncated after the fact:
/// `encounter_dto` runs two `total()` scans per fight (see its own doc),
/// so building every `EncounterDto` for a long-lived character's full
/// "All zones" list -- thousands of fights -- before ever slicing it down
/// to what a dropdown could show at once was the actual cost, not just
/// how many got sent over IPC. Sorting the cheap `&Encounter` refs first,
/// *then* slicing, *then* only computing DTOs for the slice, is what
/// makes a page cost O(limit), not O(total fights this zone/session ever
/// had) -- what `stores/combat.ts`'s own sliding-window loader needs this
/// to be, to stay cheap as the window advances deep into a long session.
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
        .filter(|e| matches_visit(ing, e.start_ms, zone_visit))
        .collect();
    matched.sort_by(|a, b| b.start_ms.cmp(&a.start_ms));
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
    /// Lower is rarer -- whichever of the matching NPC's own known-loot
    /// `chance_per_kill`/`chance_per_drop` this item has (see
    /// `drop_chance`'s doc). `None` when there's nothing to rank by; those
    /// sort after every ranked drop rather than being guessed at as common
    /// or rare.
    pub chance: Option<f64>,
}

/// The cheap half of an encounter's shape -- everything `list_zone_
/// encounters`' preview line and card actually show without needing a
/// single `total()` aggregation. Deliberately *not* `EncounterDto`:
/// that one's `total_damage`/`dps`/`enemy_damage`/`enemy_dps` need two
/// `total()` scans each, and checked against a real long-lived session,
/// computing those for every visible row -- data the collapsed preview
/// never even displays -- was a second real cost stacked on top of the
/// eager-drops one already fixed. `encounter_detail` (below) is where
/// those numbers live now, fetched once a card is actually expanded.
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
    // Deliberately NOT #[serde(flatten)] -- this nests as a real
    // `"encounter": {...}` object on the wire (`ze.encounter.id` on the
    // frontend, not `ze.id`), matching what `ui/app/app.js`'s
    // gdZoneEncounterRowHtml has always expected. flatten would merge
    // these fields into ZoneEncounterDto's own top level instead, silently
    // breaking that access (`ze.encounter` becomes `undefined`) with
    // nothing but an uncaught exception -- no error surfaced, no crash,
    // just the "Loading..." placeholder never getting replaced. That's a
    // real, confirmed instance of this, not a hypothetical: this struct
    // used flatten from this feature's very first version, and was very
    // likely the actual root cause of "stuck on loading" the whole time
    // the last several rounds of *performance* fixes were chasing.
    pub encounter: EncounterPreviewDto,
    /// Difficulty tier (0-4, `zone::zone_tier`'s own scale) the fight
    /// happened at -- read straight off the encounter's first row
    /// (`Store::tier`, stamped at ingest time), not recomputed here.
    pub tier: u8,
    /// This fight's own visit index (`None` for the pre-first-zone-line
    /// "Unknown" bucket) -- what the Combat module's own zone-select
    /// dropdown is keyed by, so a "view in Combat" link can drop straight
    /// into the right visit instead of just the right zone *name* (the
    /// same zone can have several visit indices across a session -- see
    /// `list_zone_encounters`'s own doc).
    pub zone_visit: Option<i64>,
    /// Best-effort display zone for this fight: the resolved wiki zone id
    /// if `Ingest::cached_wiki_zone` has one, else the raw log label, else
    /// `None` (no `zone.enter` seen yet). Meaningful on an NPC's own "your
    /// encounters" list (`list_mob_encounters`) -- the same mob can turn
    /// up in different zones -- redundant on a zone page's own list
    /// (`list_zone_encounters`, where you're already looking at that
    /// zone) but populated there too rather than leaving one caller with
    /// a field the other doesn't have.
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

/// How long after a fight's own end a loot event still counts toward it.
/// 90 seconds (this constant's original value) assumed looting always
/// follows death quickly, which turned out to be only half true: an
/// advanced-loot item with an "Always Loot"/"Always Merge" rule resolves
/// instantly, same second as the kill, but anything that rule doesn't
/// cover instead opens an interactive loot window that just sits there
/// until the player gets back to it -- which, mid-raid, can genuinely be
/// many minutes later, not seconds. 90s was silently *missing* those --
/// not misattributing them to the wrong fight, just failing to attribute
/// them to anything, which is worse: a real drop reading as "no drops
/// recorded" instead of just being credited to a slightly-off encounter.
///
/// 30 minutes trades a real but small false-attribution risk (a very
/// late manual resolution landing on the wrong same-named kill if
/// several happened in between) for closing that much larger false-miss
/// gap -- and the false-attribution risk is already blunted by
/// `Ingest::recent_encounter_for`'s claim tracking, which always prefers
/// the oldest *unclaimed* same-named kill rather than picking randomly,
/// so a longer window mostly buys correctness, not chaos.
///
/// `pub(crate)`, not private: `Ingest::record_loot` uses this exact same
/// window at ingest time now (see its doc) -- one number, not two
/// independently-tuned copies that could drift apart.
pub(crate) const LOOT_GRACE_MS: Millis = 30 * 60_000;

/// The matching NPC's own known-loot chance for this exact item, if the
/// bestiary (`npcdata::npcs`) has an entry for both -- prefers
/// `chance_per_kill` (the more consistently populated of the two in the
/// scrape) over `chance_per_drop`. `None` covers three different reasons
/// alike (mob not in the bestiary, item not in that mob's known-loot
/// table, or listed with no chance figure recorded) rather than treating
/// any of them as "common" by default.
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

/// The most recent `limit` encounters from any visit to the wiki zone
/// named by `zone_id` (`zonedata::Zone::id`, not `name` -- see
/// `Ingest::resolved_wiki_zone`'s doc for why an id, not a name: an exact
/// `==` here, not a case-insensitive compare, and directly eyeball-
/// checkable against the same id `debugview::list_debug_encounters` shows
/// per encounter and a zone page's own `Zone::id` field). There can be
/// several visits across a session (see `eqlp_session::Spans`' own doc on
/// why repeat visits to the same zone get distinct spans rather than
/// merging) -- folded together here by resolved zone, via each
/// encounter's own already-stamped `zone` field (see `Store::Encounter::
/// zone`'s doc), not by visit index, since this only ever needs "did this
/// fight happen somewhere that counts as this zone", not which specific
/// visit it was.
///
/// `ing.store.encounters` is already oldest-first (fights are pushed in
/// the order they open, never reordered) -- walked in *reverse*, stopping
/// the instant `limit` matches are found, so a zone you haven't visited
/// in months costs one exit-early scan back to the last visit, not a scan
/// of every fight ever recorded. Matching itself is `Ingest::
/// cached_wiki_zone`, an O(1) lookup into a resolution that already
/// happened once at ingest time -- not a fresh `zone::zone_matches`
/// string comparison run again here for every fight, and not a fresh
/// `Ingest::zone.at(ts)` lookup either -- see `Store::Encounter::zone`'s
/// and `Ingest::current_zone`'s docs.
///
/// Deliberately does *not* also compute damage totals or drops here --
/// both used to run eagerly for every visible row, and checked against a
/// real, long-lived session that was the entire difference between this
/// staying fast and it visibly not: `debugview::list_debug_encounters`
/// (neither computation at all) was instant on the same data this was
/// stalling on. `encounter_detail` is that work now, moved to run per
/// encounter, on demand, once a row's actually expanded -- see its own
/// doc for how it stays cheap doing that.
pub fn list_zone_encounters(ing: &Ingest, zone_id: &str, limit: usize) -> Vec<ZoneEncounterDto> {
    let now = ing.now_ms();

    let mut matched: Vec<&Encounter> = Vec::with_capacity(limit.min(256));
    for e in ing.store.encounters.iter().rev() {
        let is_match = e.zone.and_then(|z| ing.cached_wiki_zone(z)) == Some(zone_id);
        if is_match {
            matched.push(e);
            if matched.len() >= limit {
                break;
            }
        }
    }
    // Already newest-first: reverse iteration over an oldest-first vec,
    // pushed in the order found -- no separate sort needed.

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

/// An NPC page's own "Your history with this mob" section --
/// `list_zone_encounters`' twin, matching by mob name instead of zone.
/// Simpler than the zone case: a mob's log name and its Game Data name
/// are the same string (no wiki-vs-client naming-convention gap the way
/// zones have -- see `zone::zone_matches`' own doc for what that gap
/// looks like), so this needs no alias table or normalization, just
/// `eq_ignore_ascii_case`. Same reverse-scan-and-stop-early shape as
/// `list_zone_encounters`, for the same reason: bounded work regardless
/// of how many times you've fought this mob across the session.
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
    /// Best-effort, not authoritative: every loot row this exact encounter
    /// was attributed to at ingest time (`Ingest::recent_encounter_for`,
    /// see its own doc), timestamped within the fight's own span plus a
    /// short grace window after it ends (`LOOT_GRACE_MS`). Still a guess,
    /// not a real link -- resolved once, at ingest time, instead of fresh
    /// on every query, but the underlying ambiguity (two same-named mobs
    /// pulled at once) doesn't go away just because *when* it's resolved
    /// changed. Rarer first.
    pub drops: Vec<EncounterDropDto>,
}

/// The expensive half of one encounter's data -- damage totals (two
/// `total()` scans) and drops (a windowed loot join) -- computed for
/// exactly one fight, on demand, once its card is actually expanded.
/// Neither of these run as part of `list_zone_encounters` any more; see
/// that function's and `EncounterPreviewDto`'s docs for why eagerly
/// computing this for every visible row was the real cost of a zone
/// page's initial load on a long-lived session.
///
/// The drops half's own window-bounding: `Store`'s rows are appended in
/// strict log-chronological order (never reordered -- see `ingest::
/// backfill_lines`'s own doc on why application stays sequential even
/// when classification is parallelised), so `ing.store.ts` is already
/// sorted -- this binary-searches straight to the fight's own window
/// (`partition_point`, the same technique `Spans::at` uses) instead of
/// scanning from the start of the session. Falls back to the encounter's
/// own last recorded row, not "now", for a fight with no `end_ms` (never
/// cleanly closed -- a disconnect or a zone change mid-fight, both real
/// and not rare): "now" is real wall-clock time, later than every row a
/// backfilled session holds, and would silently widen that one
/// encounter's window to the entire rest of the store. A fight's own
/// window this way is typically seconds to a few minutes of log activity,
/// regardless of how many millions of rows the session holds in total.
///
/// `None` for an unknown `encounter_id`, not a zeroed DTO -- "this fight
/// doesn't exist" and "this fight did zero damage" are different facts.
pub fn encounter_detail(ing: &Ingest, encounter_id: u32) -> Option<EncounterDetailDto> {
    let now = ing.now_ms();
    let e = ing.store.encounter(EncounterId(encounter_id))?;

    let dur = e.duration_ms(now).max(0);
    let dur_secs = (dur as f64 / 1000.0).max(0.001);
    let all = total(&ing.store, &Filter::encounter(e.id).damage());
    let enemy = total(&ing.store, &Filter::encounter(e.id).damage().by(e.target));
    let dmg = all.saturating_sub(enemy);

    // Loot rows fall outside e.range() (first..last only covers combat
    // rows -- looting happens after the last swing/cast, sometimes well
    // after), so this still needs the same time-bounded scan
    // list_zone_encounters' own doc on encounter_drops used to need,
    // rather than the plain Filter::encounter(id) range_of already gives
    // damage/heal above. What's different now: the match itself is
    // `enc[i] == e.id`, not "same target name within the window" -- an
    // exact read of what Ingest::record_loot already decided at ingest
    // time (see its own doc), not a second, possibly-different guess
    // re-derived here from scratch.
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
    let mut drops: Vec<EncounterDropDto> = (lo..hi)
        .filter(|&i| ing.store.kind[i] == EventKind::Loot && ing.store.enc[i] == e.id.0)
        .map(|i| {
            let item = ing.store.ability_name(ing.store.ability[i]).to_string();
            let chance = drop_chance(&target_name, &item);
            EncounterDropDto {
                qty: ing.store.amount[i],
                item,
                chance,
            }
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

/// Every encounter id in a selection: one specific fight if `encounter_id`
/// is given, otherwise every fight in `zone_visit`, otherwise every fight
/// parsed so far. Shared by `summarize` and `list_allies` -- both aggregate
/// over "the current selection", just grouped by a different key.
fn resolve_ids(
    ing: &Ingest,
    zone_visit: Option<i64>,
    encounter_id: Option<u32>,
) -> Vec<EncounterId> {
    if let Some(eid) = encounter_id {
        vec![EncounterId(eid)]
    } else {
        ing.store
            .encounters
            .iter()
            .filter(|e| matches_visit(ing, e.start_ms, zone_visit))
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

/// Removes `rows` from an accumulator built by `merge_ability_rows`, then
/// drops any entry that's been subtracted down to nothing -- an ability
/// only the enemy used (a mob-only special attack, say) should vanish from
/// a "team" breakdown entirely, not linger as a zero row. `min`/`max`
/// aren't meaningfully subtractable and are left as the all-actors
/// extremes; a minor, known imprecision on abilities the enemy happens to
/// share with the team (in practice just melee attack-type rows like
/// "Punch" both sides can throw), not on total/hits/dps/pct, which is what
/// this exists for.
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

/// Every spell cast across `ids`, grouped by ability and by outcome.
/// `actor_sym` narrows to one entity's own casts, same as the damage side;
/// `None` (team view) excludes each fight's own target -- a mob's casts
/// aren't "the team's casts" any more than its damage is "the team's
/// damage". Walks the raw store columns directly rather than going through
/// `by_ability`: that function only tracks one OR'd `flags` bitmask per
/// ability, which can't recover "how many landed vs. resisted vs.
/// interrupted", the whole point of this breakdown.
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

/// Aggregates one encounter, every encounter in a zone visit, or every
/// encounter parsed so far. `encounter_id` wins if given; otherwise
/// `zone_visit`; otherwise everything. `actor`, if given, narrows to one
/// ally's own abilities -- the drill-down from `list_allies`.
///
/// `total_damage`/`abilities` are the team's own output: everyone in the
/// fight *except* its own target (subtracted out below), so this never
/// mixes offense and incoming damage into one meaningless number. What the
/// target did back is `enemy_damage`/`enemy_dps`, reported separately
/// rather than folded in.
///
/// One or two `by_ability`/`total` calls per encounter (`docs/design/store.md`
/// measures a single-encounter `by_ability` at 39µs) rather than teaching
/// `eqlp-store`'s `Filter` to accept a set of encounter ids: a zone visit is
/// a handful of fights, so this is cheap, and it leaves the store's query
/// contract exactly as documented rather than extending it for one caller.
pub fn summarize(
    ing: &Ingest,
    zone_visit: Option<i64>,
    encounter_id: Option<u32>,
    actor: Option<&str>,
) -> CombatSummaryDto {
    let now = ing.now_ms();
    let ids = resolve_ids(ing, zone_visit, encounter_id);
    if ids.is_empty() {
        return CombatSummaryDto::default();
    }
    let actor_sym = actor.and_then(|n| ing.store.names.get(n));

    let mut duration_ms: Millis = 0;
    let mut enemy_damage = 0u64;
    let mut merged: HashMap<eqlp_store::AbilityId, AbilityRow> = HashMap::new();
    // One dps reading per fight, averaged rather than pooled at the end --
    // see `CombatSummaryDto::dps`'s doc for why. `enemy_dps` gets the same
    // treatment for the same reason -- it's the same kind of rate stat,
    // just measuring the other direction.
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

        if let Some(sym) = actor_sym {
            // Drilling into one specific ally: their own rows only. A real
            // ally is never a fight's own target, so no exclusion needed.
            let rows = by_ability(&ing.store, &Filter::encounter(id).damage().by(sym));
            let fight_total: u64 = rows.iter().map(|r| r.total).sum();
            per_fight_dps.push(fight_total as f64 / fight_secs);
            merge_ability_rows(&mut merged, rows);
            // Separate query, not folded into the `.damage()` one above:
            // `Filter` narrows to one `kind`, and a fully-avoided swing is
            // `EventKind::Miss`, not `Damage` -- see `flag::MITIGATED`'s
            // own doc for why it still lands on the *same* ability row
            // once merged here (`merge_ability_rows` combines both).
            let avoided = by_ability(
                &ing.store,
                &Filter::encounter(id).kind(EventKind::Miss).by(sym),
            );
            merge_ability_rows(&mut merged, avoided);
        } else {
            // Team aggregate: everyone in the fight, minus whatever the
            // fight's own target contributed. Reuses `enemy_rows` from
            // above rather than re-querying the same filter twice.
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
    rows.sort_by(|a, b| b.total.cmp(&a.total));

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
}

/// Damage dealers in the current selection, sorted by total descending --
/// the Combat module's primary view. Click one (`summarize` with `actor`
/// set) to drill into their own ability breakdown.
///
/// Reads straight from `by_actor(Filter::encounter(id).damage())` -- a
/// `store::Encounter`'s own range plus its `enc` column, not
/// `Ingest::entities_by_enc` (the encounter graph's entity list). That
/// matters for merged pets: `Ingest::sym` redirects a matched pet's rows to
/// its owner's `Sym` at push time, but the owner's *name* was never
/// necessarily added to the graph's entity list for that fight (a
/// pet-class player who never personally swings still owns rows tagged
/// with their Sym). Querying the store directly finds them regardless;
/// walking `entities_by_enc` would have silently missed them. An earlier
/// version routed through `entities_by_enc` for an unrelated reason (see
/// git history) and produced a different total than `summarize` for the
/// same selection on the real reference log -- both are store-driven now,
/// so they can't disagree.
///
/// Excludes everything `Allegiance::of` calls `Enemy` as of right now --
/// `Kind` plus current `State`, so a multi-mob pull's other mobs are
/// excluded same as the fight's own anchor target, and a live-charmed mob
/// counts as an ally for as long as the charm holds. Was previously "exclude
/// only the fight's own `target` label", which missed every other mob in a
/// multi-mob pull; see git history.
pub fn list_allies(
    ing: &Ingest,
    zone_visit: Option<i64>,
    encounter_id: Option<u32>,
) -> Vec<AllyDto> {
    let ids = resolve_ids(ing, zone_visit, encounter_id);
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
            let kind = ing.encounters.entities.kind(name);
            let state = ing
                .timeline
                .state_at(sym.0, now)
                .map(|(s, _)| s)
                .unwrap_or(State::Engaged);
            if Allegiance::of(kind, state).is_enemy() {
                continue;
            }
            let e = acc.entry(sym).or_insert((0, 0, 0));
            e.0 += dmg;
            e.1 += hits;
            e.2 += crits;
        }
    }

    let duration_ms: Millis = ids
        .iter()
        .filter_map(|&id| ing.store.encounter(id))
        .map(|e| e.duration_ms(now).max(0))
        .sum();
    let dur_secs = (duration_ms.max(0) as f64 / 1000.0).max(0.001);
    let total_damage: u64 = acc.values().map(|(dmg, _, _)| dmg).sum();

    let mut out: Vec<AllyDto> = acc
        .into_iter()
        .map(|(sym, (dmg, hits, crits))| {
            let name = ing.store.name(sym).to_string();
            let kind = ing.encounters.entities.kind(&name);
            AllyDto {
                is_player: kind == Kind::Player,
                is_pet: kind == Kind::Pet,
                total: dmg,
                hits,
                crits,
                crit_pct: if hits > 0 {
                    100.0 * crits as f64 / hits as f64
                } else {
                    0.0
                },
                dps: dmg as f64 / dur_secs,
                pct: if total_damage > 0 {
                    100.0 * dmg as f64 / total_damage as f64
                } else {
                    0.0
                },
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

/// How far back from a scrubbed instant `fight_state_at` still shows a
/// recognized buff/effect landing as "recent". Wider than
/// `INSPECT_WINDOW_MS` on purpose -- a burst DPS snapshot goes stale in a
/// few seconds, but a buff a party member cast on you is still plausibly
/// relevant a while after it landed. There's no log line for when it wears
/// off (see `ingest::EffectPing`'s doc), so this can only ever be an
/// honest "landed recently", never a live "still active" claim.
const EFFECT_RECENCY_MS: Millis = 60_000;

#[derive(Debug, Clone, Serialize)]
pub struct EntitySeriesDto {
    pub name: String,
    pub is_player: bool,
    pub is_pet: bool,
    /// `Allegiance::of(kind, state)` as of the fight's end (or `now`, if
    /// still open) -- covers every mob in a multi-mob pull, not just the
    /// one the fight opened on, and reads a still-charmed mob as an ally.
    /// See `eqlp_session::allegiance`'s doc comment for the `Unproven`
    /// default and its known false-positive: an unspoken teammate.
    pub is_enemy: bool,
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
    pub is_pet: bool,
    /// `Allegiance::of(kind, state)` at `ts_ms` -- `state` is the same
    /// value `state` (below) is derived from, so a charmed mob and its
    /// flipped allegiance always agree at the instant shown.
    pub is_enemy: bool,
    pub state: &'static str,
    /// Whether `state` came from a log line (mesmerized/charmed/slain) or
    /// was inferred from silence (`Lost`, or the default `Engaged` before
    /// any transition at all). See `docs/design/timeline.md`.
    pub observed: bool,
    /// Damage over the `INSPECT_WINDOW_MS` trailing up to the clicked
    /// instant -- a snapshot reading, not a running total.
    pub dps: f64,
    /// Recognized buff/effect landing text within `EFFECT_RECENCY_MS`
    /// trailing up to the clicked instant -- see `ingest::Effects::recent`.
    /// Only ever non-empty for "You": the dictionary this is matched
    /// against holds first-person text exclusively, so it can't recognize
    /// something landing on anyone else. Recency, not a live "still up"
    /// claim -- there's no log line for when these wear off.
    pub recent_effects: Vec<String>,
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

    // Raw graph entity names, resolved through inferred pet ownership and
    // de-duplicated -- the graph doesn't know about pet merging (see
    // `Ingest::link`'s doc comment), so a merged pet's raw name and its
    // owner's raw name can both appear here naming the same effective
    // entity.
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
            continue; // healer or pure target -- nothing to plot as a dps bar
        }
        let buckets = bucket_series(&ts, &amt, start, end, bucket_ms);
        buckets_len = buckets_len.max(buckets.len());
        let total: u64 = amt.iter().sum();
        let kind = ing.encounters.entities.kind(name);
        // State as of the fight's end (or `now`, for one still open) --
        // see `Allegiance`'s doc comment for why this is a query, not a
        // stored flag, and why a still-charmed mob reads as an ally here.
        let state = ing
            .timeline
            .state_at(sym.0, end)
            .map(|(s, _)| s)
            .unwrap_or(State::Engaged);
        series.push(EntitySeriesDto {
            name: name.clone(),
            is_player: kind == Kind::Player,
            is_pet: kind == Kind::Pet,
            is_enemy: Allegiance::of(kind, state).is_enemy(),
            total,
            values: buckets.iter().map(|b| b.total).collect(),
        });
    }
    series.sort_by(|a, b| b.total.cmp(&a.total));

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

/// Every entity in the fight, their state, and a snapshot DPS reading, all
/// as of `ts_ms` -- what clicking a point on the timeline shows.
pub fn fight_state_at(ing: &Ingest, encounter_id: u32, ts_ms: Millis) -> Vec<EntityStateDto> {
    let id = EncounterId(encounter_id);
    // See fight_timeline's matching comment: resolved through inferred pet
    // ownership and de-duplicated, since the graph's raw entity list can
    // name a merged pet and its owner separately.
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
            let kind = ing.encounters.entities.kind(&name);
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
                        INSPECT_WINDOW_MS,
                    )
                })
                .unwrap_or(0.0);
            let recent_effects = sym
                .map(|s| {
                    ing.effects
                        .recent(s.0, ts_ms, EFFECT_RECENCY_MS)
                        .into_iter()
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            EntityStateDto {
                is_player: kind == Kind::Player,
                is_pet: kind == Kind::Pet,
                is_enemy: Allegiance::of(kind, state).is_enemy(),
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

// ---------------------------------------------------------------- class detection

#[derive(Debug, Clone, Serialize)]
pub struct ClassConfigurationDto {
    /// Classes confirmed for this configuration, alphabetical, always
    /// exactly `classdetect::CLASS_COUNT` long -- see
    /// `eqlp_session::classdetect::Detector::visits_by_resolved_configuration`'s
    /// doc for why a shorter, partial set never reaches this DTO on its
    /// own; above level 10 there's no such thing as a smaller real
    /// configuration.
    pub classes: Vec<String>,
    /// How many distinct zone visits resolved to exactly this
    /// configuration -- includes visits that only had partial evidence but
    /// were unambiguously folded into this one. Not a percentage or a
    /// confidence score -- membership in this model isn't graded, a
    /// configuration either happened in a given visit or it didn't -- this
    /// is just a plain count, so a loadout kept for one fight and a
    /// loadout played all night are both visible, honestly sized.
    pub zone_visits: usize,
    /// Effective player level observed across this configuration's own
    /// zone visits -- `(lowest, highest)`, sampled at both each visit's
    /// start *and* end (see `level_range_for`'s own doc for why both: a
    /// `level.up` mid-visit is still this configuration's own evidence,
    /// not just whatever level it started at). `None` if no `level.up`
    /// line landed before any of them. A range, not one number: the same
    /// configuration can be revisited at very different levels over time,
    /// and swapping a class out drops the effective level with no log
    /// line marking the drop itself -- see `Levels`'s doc in `ingest.rs`.
    pub level_range: Option<(u8, u8)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassConfigurationsDto {
    pub configurations: Vec<ClassConfigurationDto>,
    /// Zone visits with real but incomplete class evidence that
    /// subset-elimination couldn't unambiguously fold into any one
    /// confirmed configuration -- see `visits_by_resolved_configuration`'s
    /// doc. Reported as a plain count, not shown as configurations of
    /// their own: above level 10 a 1- or 2-class loadout doesn't exist, so
    /// these are genuinely unresolved, not a smaller real answer.
    pub unresolved_visits: usize,
}

/// Every class configuration `name` has confirmed, across every zone visit
/// they've ever played, most zone visits first, reconciled against the
/// fixed-3-classes rule (see `ClassConfigurationsDto::unresolved_visits`'s
/// doc). See `eqlp_session::classdetect`'s module doc for why this is a
/// *list* of configurations rather than one rolling combination: a loadout
/// used occasionally (kept for one specific fight, say) is just as real as
/// whatever's played most, and collapsing to a single "current" answer is
/// what was hiding it. Empty only if `name` has never landed a single
/// unambiguous recognised cast.
pub fn class_configurations(ing: &Ingest, name: &str) -> ClassConfigurationsDto {
    let Some(sym) = ing.store.names.get(name) else {
        return ClassConfigurationsDto {
            configurations: Vec::new(),
            unresolved_visits: 0,
        };
    };
    let (resolved, unresolved) = ing.classes.visits_by_resolved_configuration(sym.0);
    let configurations = resolved
        .into_iter()
        .map(|(classes, visits)| {
            let level_range = level_range_for(ing, &visits);
            let zone_visits = visits.len();
            ClassConfigurationDto {
                classes,
                zone_visits,
                level_range,
            }
        })
        .collect();
    ClassConfigurationsDto {
        configurations,
        unresolved_visits: unresolved.len(),
    }
}

/// `(lowest, highest)` effective level across `visits`, from real
/// `level.up` lines that fired *during* one of them (`Levels::between`) --
/// never a boundary snapshot. An earlier version sampled `Levels::at` the
/// instant each visit started and ended instead, on the assumption that
/// "whatever the tracker last said" was a fair stand-in for a visit with
/// no ding of its own. Checked against a real log and confirmed wrong: a
/// config swap drops the effective level silently (see `Levels`' own
/// doc), so the *start* sample routinely belongs to whatever different
/// configuration was played right before this one, not this one --
/// concretely, a visit that opened right after another trio dinged to 30
/// reported `(18, 30)` for a configuration that, by its own real evidence,
/// only ever actually climbed 11 through 18. 30 was never this
/// configuration's level; it was the previous one's, still sitting in the
/// tracker because nothing had re-dinged yet at the exact instant this
/// visit opened.
///
/// This version trusts nothing but real dings that happened *inside* a
/// visit's own window, so a visit with none contributes nothing (not a
/// borrowed neighbor's value) -- honestly reflects "no evidence" as
/// `None` overall rather than fabricating a number from outside it.
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

/// Every zone visit that resolved to exactly `classes` under
/// `class_configurations`'s same reconciliation, for drilling from one
/// configuration row down to the specific visits (and from there,
/// `list_encounters`, the fights) that make it up. Empty if `classes`
/// doesn't match any configuration `name` actually has.
pub fn zone_visits_for_configuration(
    ing: &Ingest,
    name: &str,
    classes: &[String],
) -> Vec<ZoneVisitDto> {
    let Some(sym) = ing.store.names.get(name) else {
        return Vec::new();
    };
    let (resolved, _) = ing.classes.visits_by_resolved_configuration(sym.0);
    let Some((_, wanted)) = resolved.into_iter().find(|(c, _)| c.as_slice() == classes) else {
        return Vec::new();
    };
    let mut out: Vec<ZoneVisitDto> = zone_visit_dtos(ing)
        .into_iter()
        .filter(|dto| wanted.contains(&dto.index))
        .collect();
    sort_zone_visits(&mut out);
    out
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

    /// A ding still inside the visit that fired it is real evidence for
    /// that visit, whether or not a later visit exists to happen to
    /// re-sample it.
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

    /// The actual bug this replaced boundary-sampling to fix: a level
    /// seeded (or dinged) *before* a visit ever started must not leak
    /// into that visit's own range just because nothing had re-dinged
    /// yet by the instant it opened -- that's a different configuration's
    /// level, not this one's. A real character arriving already
    /// mid-progression (seeded at 45, not built up from a fabricated
    /// chain of 44 prior dings) is exactly that case.
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

    /// A visit with no ding of its own contributes nothing -- not the
    /// seeded/inherited level from before it started.
    #[test]
    fn a_visit_with_no_ding_of_its_own_contributes_no_evidence() {
        let text = "[Tue Jul 28 15:00:00 2026] You have entered The Estate of Unrest.\n";
        let ing = run(text, 45);
        let visits: Vec<ZoneVisit> = vec![Some(0)];
        assert_eq!(level_range_for(&ing, &visits), None);
    }

    /// The real bug, reproduced: two zone visits, the first dinged to 30,
    /// the second (a genuinely different configuration's own visit)
    /// dinged 11 through 18 -- matching a real character's own log, where
    /// swapping in a fresh, unleveled class drops the effective level with
    /// no line marking the drop itself (`Levels`' own doc). The second
    /// visit's own range must be exactly its own dings, never 30.
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

        // Befallen (index 0) is the only zone visit here.
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

    /// 6 distinct, real-shaped kills (the exact "You hit X ... by Y." /
    /// "You have slain X!" pattern confirmed elsewhere against a real
    /// log -- see xp_tests::KILL_XP), one minute apart, each its own
    /// target so paging can be told apart by name alone.
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
        // target 6 died last -- newest first means it's page one.
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

    /// Two adjacent pages, concatenated, must equal one page big enough
    /// to hold both -- no fight lost or duplicated at the seam.
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

    /// A fight with nothing after its last line stays *open* (`expire`
    /// only runs from `tick`, and `tick` only trusts the log's own clock,
    /// which only moves forward on a real backfilled line) -- so every
    /// case here gets one harmless trailing line, well past the graph's
    /// 10s idle timeout, to force the real idle-close before querying.
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

    /// The trailing filler line (see `ingest_from`) is its own encounter --
    /// find the one under test by name rather than assuming position.
    fn find<'a>(list: &'a [EncounterDto], target: &str) -> &'a EncounterDto {
        list.iter()
            .find(|e| e.target == target)
            .expect("target should have its own encounter")
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

    /// The bug: `death.you_died` used to land in the same slain-name list
    /// as a real target kill, so dying to a mob that lived read as a kill.
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
}

#[cfg(test)]
mod ability_mitigation_dto_tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;

    /// The real gap this whole feature needed: `summarize` (what
    /// `get_combat_summary`/the AllyTable expand panel actually reads)
    /// queries `by_ability` with `.damage()`, which excludes
    /// `EventKind::Miss` outright -- so a mitigated swing's counts must
    /// be merged in from a *second*, `.kind(EventKind::Miss)` query, or
    /// they never reach the DTO at all, no matter how correct the store
    /// side is.
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

        let summary = summarize(&ing, None, None, Some("You"));
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

    /// Same point, for the team-aggregate path (`actor: None`), which
    /// also has to subtract the fight's own target's mitigated swings the
    /// same way it already subtracts their damage.
    #[test]
    fn team_aggregate_excludes_the_target_s_own_mitigated_swings() {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        let lines: Vec<&[u8]> = vec![
            b"[Tue Jul 28 15:01:00 2026] You punch a target for 5 points of damage.",
            b"[Tue Jul 28 15:01:01 2026] You try to punch a target, but a target blocks!",
            // The target's own swing, dodged by the player -- must not
            // pollute the *team's* Punch row.
            b"[Tue Jul 28 15:01:02 2026] a target tries to punch YOU, but YOU dodge!",
        ];
        backfill_lines(&mut ing, &engine, &lines, 1);

        let summary = summarize(&ing, None, None, None);
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

//! The IPC surface.
//!
//! Two shapes: `get_status` / `pick_log_directory` / `set_log_directory` for
//! the toolbar and first-launch setup, and the Combat module's read-only
//! queries (`list_zone_visits`, `list_encounters`, `get_combat_summary`),
//! which run straight against the shared `Ingest` -- the parsed db -- with
//! no reparsing. Everything live-updating besides that goes over the
//! `parse-tick` / `parse-error` events emitted from `tail_worker`.

use crate::aadata;
use crate::character::{self, CharacterEstimateDto};
use crate::combat::{
    self, AllyDto, ClassConfigurationsDto, CombatSummaryDto, EncounterDetailDto, EncounterDto,
    EntityStateDto, FightTimelineDto, ZoneEncounterDto, ZoneVisitDto,
};
use crate::config::{self, AppConfig};
use crate::debugview::{self, DebugEncounterDto, UnmatchedCoverageDto};
use crate::dpscalc::{self, DamageSpellDto};
use crate::gearplanner::{self, InventoryDumpDto, ItemDto, SlotRecommendationDto};
use crate::history::{self, ParseRecord};
use crate::ingest::LineCounts;
use crate::inventory;
use crate::mapsdata;
use crate::mobalias;
use crate::monsters::{self, LootEventDto, MobDto, MobStatsDto};
use crate::notifications;
use crate::npcdata;
use crate::overview::{self, SessionDto};
use crate::pathfind;
use crate::preferences::{self, Preferences};
use crate::profile;
use crate::progression::{self, AaLogDto, SpellbookEntryDto};
use crate::raiding::{self, RaidRowDto};
use crate::routing;
use crate::settings;
use crate::skyquests;
use crate::spelldata;
use crate::spelleffect;
use crate::state::AppState;
use crate::tail_worker::{self, TailStatus};
use crate::uifiles;
use crate::zonedata;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, State};

#[derive(Debug, Clone, Serialize)]
pub struct StatusDto {
    pub configured: bool,
    pub status: TailStatus,
    pub counts: LineCounts,
}

#[tauri::command]
pub fn get_status(state: State<AppState>) -> StatusDto {
    StatusDto {
        configured: state.config.lock().unwrap().is_some(),
        status: state.status.lock().unwrap().clone(),
        counts: state.ingest.lock().unwrap().counts.clone(),
    }
}

/// Opens the native folder picker. Returns `None` if the user cancels --
/// that is not an error, it just means nothing changes.
///
/// Uses the plugin's async callback API, not `blocking_pick_folder`.
/// Blocking a command thread on the dialog result ties this to whatever
/// thread that command happened to run on, and on Linux the dialog goes
/// through GTK's main loop / xdg-desktop-portal -- a context blocking
/// doesn't reliably mesh with. The callback form is the one path the
/// plugin runs through the right thread on every platform; we just await
/// it instead of blocking for it.
#[tauri::command]
pub async fn pick_log_directory(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Select your EverQuest Legends install folder")
        .pick_folder(move |folder| {
            let _ = tx.send(folder);
        });
    rx.await.ok().flatten().map(|p| p.to_string())
}

/// Commits to a directory: persists it, then (re)starts the tail worker.
/// Called both from first-launch setup and from "change folder" later, so
/// switching a running app to a new directory is not a special case.
///
/// `path` is the game's *base* install folder (see `AppConfig`'s doc) --
/// validated here as a directory in its own right, but the tail worker
/// still gets pointed at `cfg.log_dir()` (its `Logs` subfolder), not
/// `path` directly.
#[tauri::command]
pub fn set_log_directory(
    app: AppHandle,
    state: State<AppState>,
    path: String,
) -> Result<StatusDto, String> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(format!("{path} is not a directory"));
    }

    let cfg = AppConfig { base_dir: dir };
    config::save(&app, &cfg)?;
    let log_dir = cfg.log_dir();
    *state.config.lock().unwrap() = Some(cfg);

    if let Some(old) = state.worker.lock().unwrap().take() {
        old.stop();
    }
    let handle = tail_worker::spawn(
        app.clone(),
        log_dir,
        state.ingest.clone(),
        state.status.clone(),
    );
    *state.worker.lock().unwrap() = Some(handle);

    Ok(StatusDto {
        configured: true,
        status: state.status.lock().unwrap().clone(),
        counts: state.ingest.lock().unwrap().counts.clone(),
    })
}

/// Every zone visit seen so far, newest first, with how many fights each
/// holds. The Combat module's first dropdown.
#[tauri::command]
pub fn list_zone_visits(state: State<AppState>) -> Vec<ZoneVisitDto> {
    combat::list_zone_visits(&state.ingest.lock().unwrap())
}

/// A newest-first list of encounters, optionally narrowed to one zone
/// visit. The Combat module's second dropdown can run into the thousands
/// for a long-lived character's "All zones" view -- that turned out to be
/// a rendering cost (the frontend virtualizes what it mounts; see
/// Combat.svelte), not a fetch one, so this defaults to the whole list.
/// `offset`/`limit` still exist (see `combat::list_encounters`'s own doc)
/// for whatever future caller actually wants a bounded page. `zone_visit`
/// is `None` for no filter, `-1` for the "Unknown" (pre-first-zone-line)
/// bucket, otherwise a visit index -- see `combat::matches_visit`.
#[tauri::command]
pub fn list_encounters(
    state: State<AppState>,
    zone_visit: Option<i64>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Vec<EncounterDto> {
    combat::list_encounters(
        &state.ingest.lock().unwrap(),
        zone_visit,
        offset.unwrap_or(0),
        limit.unwrap_or(usize::MAX),
    )
}

/// A zone page's "Your parsed encounters here" section -- the most recent
/// `limit` (default 30) fights from any visit to the wiki zone identified
/// by `zone_id` (`zonedata::Zone::id`, not its display name -- see
/// `combat::list_zone_encounters`'s doc for why an id). Cheap on its own:
/// no damage totals, no drops -- see `EncounterPreviewDto`'s doc for why,
/// and `get_encounter_detail` for where that work moved to.
#[tauri::command]
pub fn list_zone_encounters(
    state: State<AppState>,
    zone_id: String,
    limit: Option<usize>,
) -> Vec<ZoneEncounterDto> {
    combat::list_zone_encounters(&state.ingest.lock().unwrap(), &zone_id, limit.unwrap_or(30))
}

/// One encounter's damage totals and drop list, fetched separately from
/// `list_zone_encounters` (which no longer computes either eagerly -- see
/// its doc) so a zone page's initial list never waits on them; called
/// once a row is actually expanded. `None` for an unknown `encounter_id`.
#[tauri::command]
pub fn get_encounter_detail(
    state: State<AppState>,
    encounter_id: u32,
) -> Option<EncounterDetailDto> {
    combat::encounter_detail(&state.ingest.lock().unwrap(), encounter_id)
}

/// An NPC page's "Your history with this mob" section -- kills/pulls
/// totals plus the most recent `limit` (default 30) fights against
/// `mob_name`, mirroring what a zone page's own encounter list shows.
#[tauri::command]
pub fn get_mob_stats(state: State<AppState>, mob_name: String) -> MobStatsDto {
    monsters::mob_stats(&state.ingest.lock().unwrap(), &mob_name)
}

#[tauri::command]
pub fn list_mob_encounters(
    state: State<AppState>,
    mob_name: String,
    limit: Option<usize>,
) -> Vec<ZoneEncounterDto> {
    combat::list_mob_encounters(
        &state.ingest.lock().unwrap(),
        &mob_name,
        limit.unwrap_or(30),
    )
}

/// The Debug module's one table: the most recent `limit` (default 100)
/// encounters with exactly what zone they're tagged with, raw and
/// resolved -- see `debugview::list_debug_encounters`'s doc.
#[tauri::command]
pub fn list_debug_encounters(
    state: State<AppState>,
    limit: Option<usize>,
) -> Vec<DebugEncounterDto> {
    debugview::list_debug_encounters(&state.ingest.lock().unwrap(), limit.unwrap_or(100))
}

/// The Debug module's "Unparsed" tab: every unmatched-line shape seen
/// this session, ranked by count -- see `debugview::unmatched_coverage`'s
/// doc.
#[tauri::command]
pub fn get_unmatched_coverage(state: State<AppState>, top: Option<usize>) -> UnmatchedCoverageDto {
    debugview::unmatched_coverage(&state.ingest.lock().unwrap(), top.unwrap_or(100))
}

/// Damage dealers in the current selection, sorted by total descending --
/// the Combat module's primary view (a "menu of allies", not a flat
/// ability table).
#[tauri::command]
pub fn list_allies(
    state: State<AppState>,
    zone_visit: Option<i64>,
    encounter_id: Option<u32>,
) -> Vec<AllyDto> {
    combat::list_allies(&state.ingest.lock().unwrap(), zone_visit, encounter_id)
}

/// The Combat module's drill-down: one ally's own ability breakdown if
/// `actor` is given, else the whole selection's combined breakdown.
#[tauri::command]
pub fn get_combat_summary(
    state: State<AppState>,
    zone_visit: Option<i64>,
    encounter_id: Option<u32>,
    actor: Option<String>,
) -> CombatSummaryDto {
    combat::summarize(
        &state.ingest.lock().unwrap(),
        zone_visit,
        encounter_id,
        actor.as_deref(),
    )
}

/// Per-entity damage-over-time bars for one fight's scrub bar.
#[tauri::command]
pub fn get_fight_timeline(state: State<AppState>, encounter_id: u32) -> Option<FightTimelineDto> {
    combat::fight_timeline(&state.ingest.lock().unwrap(), encounter_id)
}

/// What clicking a point on the scrub bar shows: every entity's state and a
/// snapshot DPS reading as of that instant.
#[tauri::command]
pub fn get_fight_state_at(
    state: State<AppState>,
    encounter_id: u32,
    ts_ms: i64,
) -> Vec<EntityStateDto> {
    combat::fight_state_at(&state.ingest.lock().unwrap(), encounter_id, ts_ms)
}

/// Every class configuration seen for one entity, across every zone visit
/// they've been played in, most zone visits first. Empty if `name` hasn't
/// been seen casting anything the spell/class lookup recognises yet -- see
/// `eqlp_session::classdetect`'s doc for what this can and can't promise.
#[tauri::command]
pub fn get_class_configurations(state: State<AppState>, name: String) -> ClassConfigurationsDto {
    combat::class_configurations(&state.ingest.lock().unwrap(), &name)
}

/// The Endgame module's Raiding tab: the curated row/raid/boss/miniboss
/// list, with this character's own confirmed kills/tiers/loot folded in
/// -- see `raiding::list_raid_rows`'s own doc.
#[tauri::command]
pub fn get_raids(state: State<AppState>) -> Vec<RaidRowDto> {
    raiding::list_raid_rows(&state.ingest.lock().unwrap())
}

/// The Endgame module's "Sky - Primary Class Unlocks" tab: just the
/// final reward items each class's quests earn, cross-referenced
/// against this character's own loot/inventory/achievements -- see
/// `skyquests::list_class_unlocks`'s own doc for why this is scoped to
/// rewards only, never the raw materials (that's `get_sky_quests`).
#[tauri::command]
pub fn get_sky_class_unlocks(state: State<AppState>) -> Vec<skyquests::SkyClassUnlockDto> {
    let base_dir = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.base_dir.clone());
    skyquests::list_class_unlocks(&state.ingest.lock().unwrap(), base_dir.as_deref())
}

/// The Endgame module's "Sky - Quests" tab: every individual material
/// turn-in (rune + drop items -> one gear reward), full detail -- see
/// `skyquests::list_quests`'s own doc.
#[tauri::command]
pub fn get_sky_quests(state: State<AppState>) -> Vec<skyquests::SkyClassDto> {
    let base_dir = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.base_dir.clone());
    skyquests::list_quests(&state.ingest.lock().unwrap(), base_dir.as_deref())
}

/// One configuration's own zone visits, for drilling from a configuration
/// row down to the specific visits (and from there, via
/// `list_encounters(zoneVisit)`, the fights) that make it up.
#[tauri::command]
pub fn get_configuration_zone_visits(
    state: State<AppState>,
    name: String,
    classes: Vec<String>,
) -> Vec<ZoneVisitDto> {
    combat::zone_visits_for_configuration(&state.ingest.lock().unwrap(), &name, &classes)
}

/// `configuration_of_visit` needs "You"'s own interned symbol, read-only
/// (`Interner::get`, not `sym`, which would need `&mut`) -- `None` only
/// before a single line has ever been processed, in which case there is
/// no history to refresh loadouts *against* either, so an empty/unchanged
/// `loadout` on every record is already the right answer.
fn you_sym(ing: &crate::ingest::Ingest) -> Option<u32> {
    ing.store.names.get("You").map(|s| s.0)
}

/// Past parses against `target`, newest first. `confirmed_only` narrows to
/// encounters that actually ended in a death line -- see
/// `ParseRecord::confirmed_kill`'s doc for why an unfiltered comparison
/// mixes a full kill with a truncated reset as if they measured the same
/// thing. Reads the persisted records from `parse_history.jsonl` (this is
/// the record meant to outlive the live store's own eviction), but
/// re-resolves each one's `loadout` against `Ingest`'s *live* class
/// evidence before returning -- see `history::refresh_loadouts`'s own doc
/// for why a record's own "as of close" snapshot can go stale within the
/// very same zone visit.
#[tauri::command]
pub fn get_mob_history(
    app: AppHandle,
    state: State<AppState>,
    target: String,
    confirmed_only: bool,
) -> Vec<ParseRecord> {
    let mut records = history::all(&app);
    let ing = state.ingest.lock().unwrap();
    if let Some(you) = you_sym(&ing) {
        history::refresh_loadouts(&mut records, &ing.classes, you);
    }
    drop(ing);
    history::mob_history_view(records, &target, confirmed_only)
}

/// Past parses against `target`, bundled by which class combination was
/// active -- "my average as Wizard/Enchanter/Magician vs. this mob" as its
/// own row, separate from "my average as Necromancer/Shadow Knight",
/// instead of one number blending playstyles that don't otherwise compare.
/// Same `confirmed_only` meaning, and the same live-loadout-refresh
/// treatment, as `get_mob_history`.
#[tauri::command]
pub fn get_loadout_summary(
    app: AppHandle,
    state: State<AppState>,
    target: String,
    confirmed_only: bool,
) -> Vec<history::LoadoutSummary> {
    let mut records = history::for_target(&app, &target);
    let ing = state.ingest.lock().unwrap();
    if let Some(you) = you_sym(&ing) {
        history::refresh_loadouts(&mut records, &ing.classes, you);
    }
    drop(ing);
    if confirmed_only {
        records = history::only_confirmed_kills(records);
    }
    history::by_loadout(&records)
}

/// The Overview module's session stats: plat/hour, xp%/hour, and an
/// estimated time to the next level, all scoped to "this session" -- see
/// `overview`'s own module doc for exactly what that means and why it
/// isn't just "since the log started".
#[tauri::command]
pub fn get_session(state: State<AppState>) -> SessionDto {
    overview::session(&state.ingest.lock().unwrap())
}

/// Every AA rank purchase seen this session, oldest first, plus the total
/// ability points spent -- see `progression`'s own module doc; no UI
/// consumes this yet.
#[tauri::command]
pub fn get_aa_log(state: State<AppState>) -> AaLogDto {
    progression::aa_log(&state.ingest.lock().unwrap())
}

/// Every spell confirmed known this session, enriched with the wiki
/// catalog's own stats -- see `progression`'s own module doc; the
/// Character module's Spellbook subpage.
#[tauri::command]
pub fn get_spellbook(state: State<AppState>) -> Vec<SpellbookEntryDto> {
    progression::spellbook(&state.ingest.lock().unwrap())
}

/// Highest live in-game rank observed cast this session, "You" only, by
/// catalog base spell name -- see `progression::spell_ranks`' own doc.
/// The Spellbook builder's suggestion picker, so a spell already ranked
/// up shows that instead of implying it's freshly unranked.
#[tauri::command]
pub fn get_spell_ranks(state: State<AppState>) -> HashMap<String, u8> {
    progression::spell_ranks(&state.ingest.lock().unwrap())
}

/// Every catalog spell with a parseable damage effect, rank-adjusted --
/// see `dpscalc`'s own module doc for the (stated, not hidden) model
/// this uses. Unfiltered by class/level; the Spellbook builder's DPS
/// auto-suggest applies the same class/level-cap filtering it already
/// uses for its spell picker.
#[tauri::command]
pub fn get_damage_spells(state: State<AppState>) -> Vec<DamageSpellDto> {
    dpscalc::list_damage_spells(&state.ingest.lock().unwrap())
}

/// Every mob type fought so far, kill counts and loot -- the Loot History
/// module's one view.
#[tauri::command]
pub fn list_mobs(state: State<AppState>) -> Vec<MobDto> {
    monsters::list_mobs(&state.ingest.lock().unwrap())
}

/// Every zone the wiki scrape carries -- the Game Data module's Zones tab,
/// and what a drop-source zone name matches against to link in-app.
/// Small and static enough (117 zones) to ship whole in one call rather
/// than a separate per-zone fetch.
#[tauri::command]
pub fn list_zones() -> Vec<zonedata::Zone> {
    zonedata::zones().to_vec()
}

/// Every NPC the wiki scrape carries -- the Game Data module's NPCs tab.
/// 6,532 entries; still one call, not paginated -- see `list_zones` for
/// the same reasoning at a tenth the size, still comfortably local-IPC
/// cheap at this one (`itemdata::items` already ships a similarly sized
/// list, unfiltered, via `list_gear_items`).
#[tauri::command]
pub fn list_npcs() -> Vec<npcdata::Npc> {
    npcdata::npcs().to_vec()
}

/// Log mob name -> wiki `Npc::name`, for Game Data's own cross-links
/// (`gdFind`'s npc case) to resolve the same real mismatches
/// `mobalias::mob_matches` already closes for backend lookups like
/// `combat::drop_chance` -- one table, not a second copy hand-kept in
/// sync on the frontend. `(from, to)` pairs, small enough to ship whole.
#[tauri::command]
pub fn get_mob_aliases() -> Vec<(String, String)> {
    mobalias::all()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Every spell the wiki scrape carries -- the Game Data module's Spells
/// tab, and what `get_spellbook` joins the log's own confirmed-known
/// names against. 1928 entries, same "one call, unfiltered" stance as
/// `list_zones`/`list_npcs`.
#[tauri::command]
pub fn list_spells() -> Vec<spelldata::Spell> {
    spelldata::spells().to_vec()
}

/// Derived spell mechanics for the whole catalog -- duration (seconds),
/// damage/heal/buff/debuff/control-effect components, and category tags
/// -- keyed by spell id. See `spelleffect`'s own module doc for exactly
/// what's real vs. best-effort here. Computed once, cached; see `spelleffect::all_effects`.
#[tauri::command]
pub fn list_spell_effects() -> Vec<spelleffect::SpellEffectsEntry> {
    spelleffect::all_effects().to_vec()
}

/// Every AA the wiki scrape carries -- the full 142-entry reference
/// catalog for the Game Data module's AAs tab. Distinct from `get_aa_log`
/// (this character's own confirmed purchases): this is the whole book
/// anyone could buy from, not what you actually own.
#[tauri::command]
pub fn list_aa() -> Vec<aadata::Aa> {
    aadata::aas().to_vec()
}

/// Every time you've actually looted `item`, oldest first -- an item
/// page's "your history with this item" section. See
/// `monsters::item_loot_history`'s doc for exactly what each event does
/// and doesn't carry.
#[tauri::command]
pub fn get_item_loot_history(state: State<AppState>, item: String) -> Vec<LootEventDto> {
    monsters::item_loot_history(&state.ingest.lock().unwrap(), &item)
}

/// The Character Planner's one call: a full attribute sheet (race, each
/// active class's own add, naked, gear, total) and a gear-inclusive
/// mana-pool estimate for `race` + up to 3 `classes`, each at its own
/// `class_levels` entry -- see `character`'s module doc for the trio
/// mechanic this is modeled on and exactly how much to trust the numbers.
/// `gear` is attribute name -> total across whatever's currently resolved
/// on the Gear Planner's own doll (the frontend sums it there -- this
/// command's own Rust side never touches an item); omitted or empty reads
/// as no gear, same as `character::estimate`'s own default. Stateless on
/// purpose: nothing here is persisted, so a fresh app launch always starts
/// blank rather than restoring a previous session's picks.
#[tauri::command]
pub fn get_character_estimate(
    race: String,
    classes: Vec<String>,
    class_levels: Vec<u8>,
    gear: Option<std::collections::HashMap<String, f64>>,
) -> Option<CharacterEstimateDto> {
    character::estimate(&race, &classes, &class_levels, &gear.unwrap_or_default())
}

/// `name`'s confirmed classes, as full class names -- what the Gear
/// Planner module pre-selects on open instead of asking you to re-tell it
/// what you're playing. Empty if nothing's confirmed yet.
#[tauri::command]
pub fn get_default_gear_classes(state: State<AppState>, name: String) -> Vec<String> {
    gearplanner::default_classes(&state.ingest.lock().unwrap(), &name)
}

/// The item browser: every item usable by `classes` (full class names),
/// optionally narrowed to one slot key and/or to an era at or before
/// `max_era` (an `eqlp_app::gearplanner::ERA_ORDER` name). `owned`/`owned_
/// tier` are the frontend's already-loaded `InventoryDumpDto` fields,
/// passed back in so browsed items can show real ownership (and be shown
/// at the tier actually owned) -- this command has no dump of its own to
/// read.
#[tauri::command]
pub fn list_gear_items(
    classes: Vec<String>,
    slot: Option<String>,
    max_era: Option<String>,
    owned: Option<std::collections::HashMap<String, u32>>,
    owned_tier: Option<std::collections::HashMap<String, u8>>,
) -> Vec<ItemDto> {
    gearplanner::list_items(
        &classes,
        slot.as_deref(),
        max_era.as_deref(),
        owned.as_ref(),
        owned_tier.as_ref(),
    )
}

/// The doll/preview panel's tier picker: re-derives `id`'s stats/exalts
/// as if it were sitting at `tier` (0-10, clamped), independent of
/// whatever it's actually shown at elsewhere -- a "what if I upgrade
/// this" preview, not a write to any real ownership state.
#[tauri::command]
pub fn get_item_at_tier(id: String, tier: u8) -> Option<ItemDto> {
    gearplanner::item_at_tier(&id, tier)
}

/// The doll/preview panel's exaltation display, re-derived with `exalts`
/// (socket key -> source item id) socketed in instead of `id`'s own
/// native effects -- see `gearplanner::item_with_exalts`'s own doc.
#[tauri::command]
pub fn get_item_with_exalts(
    id: String,
    tier: u8,
    exalts: std::collections::HashMap<String, String>,
) -> Option<ItemDto> {
    gearplanner::item_with_exalts(&id, tier, &exalts)
}

/// The exaltation picker's own candidate list -- see
/// `gearplanner::exalt_candidates`'s own doc for exactly what "legal"
/// means here. `other_assignments` is every socket on `id` already
/// filled *except* `socket_key` itself (its own not-yet-committed pick).
#[tauri::command]
pub fn get_exalt_candidates(
    id: String,
    socket_key: String,
    other_assignments: std::collections::HashMap<String, String>,
    classes: Vec<String>,
    max_era: Option<String>,
) -> Vec<ItemDto> {
    gearplanner::exalt_candidates(
        &id,
        &socket_key,
        &other_assignments,
        &classes,
        max_era.as_deref(),
    )
}

/// Top candidates for every slot, scored against `classes`/`race` -- see
/// `gearplanner::recommend`'s doc for exactly what this does and doesn't
/// account for. `level` (from `get_current_level`) is what lets INT/WIS
/// score as actual mana-pool value instead of `derived_weights`' flat
/// max-based fallback -- see that function's doc. `equipped`/`owned`/
/// `owned_tier` are the frontend's already-loaded `InventoryDumpDto`
/// fields (slot -> item name, base name -> copies owned, base name ->
/// highest tier owned) -- all `None` for a plain browsing call with no
/// dump loaded yet.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // each param is its own real, independently-optional filter -- see doc above
pub fn get_gear_recommendations(
    classes: Vec<String>,
    race: Option<String>,
    max_era: Option<String>,
    per_slot: Option<usize>,
    weights: Option<std::collections::HashMap<String, f64>>,
    level: Option<u8>,
    equipped: Option<std::collections::HashMap<String, String>>,
    owned: Option<std::collections::HashMap<String, u32>>,
    owned_tier: Option<std::collections::HashMap<String, u8>>,
) -> Vec<SlotRecommendationDto> {
    gearplanner::recommend(
        &classes,
        race.as_deref(),
        max_era.as_deref(),
        per_slot.unwrap_or(50),
        weights,
        level,
        equipped.as_ref(),
        owned.as_ref(),
        owned_tier.as_ref(),
    )
}

/// The scoring vector currently in force for `classes` -- what the
/// planner's "weights" panel shows, so a ranking is explainable instead of
/// opaque.
#[tauri::command]
pub fn get_gear_weights(
    classes: Vec<String>,
    level: Option<u8>,
) -> std::collections::HashMap<String, f64> {
    gearplanner::weights_for(&classes, level)
}

#[derive(Debug, Clone, Serialize)]
pub struct EraOptionsDto {
    /// `ERA_ORDER`, oldest first -- the Settings module's era dropdown,
    /// alongside a synthetic "All" this app adds on the frontend side
    /// (not itself an era the wiki scrape ever produced).
    pub eras: Vec<String>,
    pub current: String,
}

/// What the Settings module's era picker needs to build itself --
/// `gearplanner::ERA_ORDER`/`CURRENT_ERA` aren't otherwise reachable over
/// IPC (Rust `const`s, not data this app stores or computes per-request).
#[tauri::command]
pub fn get_era_options() -> EraOptionsDto {
    EraOptionsDto {
        eras: gearplanner::ERA_ORDER
            .iter()
            .map(|s| s.to_string())
            .collect(),
        current: gearplanner::CURRENT_ERA.to_string(),
    }
}

/// The Settings module's own volume/era preferences -- see
/// `preferences::Preferences`'s doc for what each field means and why
/// this is a separate file from `settings::NotificationSettings`.
#[tauri::command]
pub fn get_preferences(app: AppHandle) -> Preferences {
    preferences::load(&app)
}

#[tauri::command]
pub fn set_preferences(app: AppHandle, prefs: Preferences) -> Result<Preferences, String> {
    preferences::save(&app, &prefs)?;
    Ok(prefs)
}

/// Your most recently observed level (`Ingest::levels`) -- what
/// `get_gear_recommendations`/`get_gear_weights` need `level` to be, so
/// the Gear Planner's mana weighting can turn INT/WIS into an actual
/// pool estimate instead of falling back to a flat per-class number. See
/// `Levels::latest`'s doc for the (common) case this returns `None`: no
/// `level.up` line anywhere in this session's log history, which mostly
/// means you've been this level for the whole file, not that your level
/// is unknown in any deeper sense.
#[tauri::command]
pub fn get_current_level(state: State<AppState>) -> Option<u8> {
    state.ingest.lock().unwrap().levels.latest()
}

/// The inv-toast's "Load into Gear Planner" action: reads `file` (an
/// `/outputfile inventory` dump named by an `outputfile.complete` line,
/// see `inventory-dump`'s emit in tail_worker.rs) out of the game's base
/// folder, parses its equipped-item rows, and matches each against this
/// app's own item catalog so the doll can show real icons/stats for
/// what's actually equipped instead of just a bare name.
#[tauri::command]
pub fn get_inventory_dump(
    state: State<AppState>,
    file: String,
) -> Result<InventoryDumpDto, String> {
    let base_dir = {
        let cfg = state.config.lock().unwrap();
        cfg.as_ref()
            .ok_or("no install folder configured yet")?
            .base_dir
            .clone()
    };
    let path = inventory::dump_path(&base_dir, &file).map_err(|e| e.to_string())?;
    let parsed = inventory::parse(&path).map_err(|e| e.to_string())?;
    let ing = state.ingest.lock().unwrap();
    Ok(gearplanner::resolve_inventory(
        &parsed,
        Some(&ing.exaltation_procs),
    ))
}

#[derive(Debug, Clone, Serialize)]
pub struct ExistingInventoryDumpDto {
    pub file: String,
    pub character: Option<String>,
}

/// The Character module's own init check: is there already a real
/// inventory dump sitting in the game folder from a past session, not
/// just a brand new one this run happens to catch live -- see
/// `inventory::find_existing_dump`'s doc. `None` if the folder has no
/// dump at all, not just none written yet this session.
#[tauri::command]
pub fn find_existing_inventory_dump(state: State<AppState>) -> Option<ExistingInventoryDumpDto> {
    let base_dir = state.config.lock().unwrap().as_ref()?.base_dir.clone();
    let (file, character) = inventory::find_existing_dump(&base_dir)?;
    Some(ExistingInventoryDumpDto { file, character })
}

/// The Maps module's pack picker -- subfolders of `maps/` under the
/// game's base install (e.g. `Brewall`). Empty is valid: base game maps
/// only, no community pack installed. See `mapsdata::list_map_packs`.
#[tauri::command]
pub fn list_map_packs(state: State<AppState>) -> Vec<String> {
    let Some(base_dir) = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.base_dir.clone())
    else {
        return Vec::new();
    };
    mapsdata::list_map_packs(&base_dir)
}

/// The Maps module's zone picker -- every zone with at least one map file
/// under `maps/` (or `maps/<pack>` when `pack` is given). See
/// `mapsdata::list_zone_names`.
#[tauri::command]
pub fn list_map_zones(state: State<AppState>, pack: Option<String>) -> Vec<String> {
    let Some(base_dir) = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.base_dir.clone())
    else {
        return Vec::new();
    };
    mapsdata::list_zone_names(&base_dir, pack.as_deref())
}

/// The Maps module's zone-first picker: every zone with at least one map
/// file anywhere under `maps/`, whichever source (base game or any pack)
/// it comes from -- replaces making the user pick a pack before they can
/// even see whether their zone has a map. See `mapsdata::list_all_zone_names`.
#[tauri::command]
pub fn list_all_map_zones(state: State<AppState>) -> Vec<String> {
    let Some(base_dir) = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.base_dir.clone())
    else {
        return Vec::new();
    };
    mapsdata::list_all_zone_names(&base_dir)
}

/// Which source(s) have a map for `zone` -- `null` for the base game, a
/// pack name for each community pack that also covers it (e.g. Befallen:
/// base game + Brewall). Drives the "available versions" picker once a
/// zone is chosen. See `mapsdata::list_zone_versions`.
#[tauri::command]
pub fn list_zone_versions(state: State<AppState>, zone: String) -> Vec<Option<String>> {
    let Some(base_dir) = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .map(|c| c.base_dir.clone())
    else {
        return Vec::new();
    };
    mapsdata::list_zone_versions(&base_dir, &zone)
}

/// The Maps module's own render data: every wall segment and labeled
/// marker for `zone`, merged from its base map file and every numbered
/// sibling -- see `mapsdata::load_zone_map`'s own doc for why merging,
/// not picking one file, is correct here.
#[tauri::command]
pub fn get_map_file(
    state: State<AppState>,
    pack: Option<String>,
    zone: String,
) -> Result<mapsdata::MapFileDto, String> {
    let base_dir = {
        let cfg = state.config.lock().unwrap();
        cfg.as_ref()
            .ok_or("no install folder configured yet")?
            .base_dir
            .clone()
    };
    let parsed =
        mapsdata::load_zone_map(&base_dir, pack.as_deref(), &zone).map_err(|e| e.to_string())?;
    Ok(parsed.into())
}

/// A real walking route within one zone's map, waypoint by waypoint --
/// see `pathfind::find_path`'s own doc for what "real" means here (grid
/// A* over the zone's own wall geometry, Z-banded to the *starting*
/// point's own floor) and its stated limits (a route needing a floor
/// change within the zone isn't found).
#[derive(Debug, Clone, Serialize)]
pub struct PathDto {
    pub waypoints: Vec<[f32; 3]>,
}

/// Same `base_dir`-required shape as `get_map_file` -- a missing route is
/// a real, retryable outcome (no path exists on this floor, or an install
/// isn't configured yet), not folded into a generic "empty result".
#[tauri::command]
pub fn find_walk_path(
    state: State<AppState>,
    pack: Option<String>,
    zone: String,
    from: [f32; 3],
    to: [f32; 3],
) -> Result<PathDto, String> {
    let base_dir = {
        let cfg = state.config.lock().unwrap();
        cfg.as_ref()
            .ok_or("no install folder configured yet")?
            .base_dir
            .clone()
    };
    let parsed =
        mapsdata::load_zone_map(&base_dir, pack.as_deref(), &zone).map_err(|e| e.to_string())?;
    let path = pathfind::find_path(&parsed, (from[0], from[1], from[2]), (to[0], to[1], to[2]))
        .ok_or("no walkable route found between those points")?;
    Ok(PathDto {
        waypoints: path.into_iter().map(|(x, y, z)| [x, y, z]).collect(),
    })
}

/// One leg of a `ZoneRouteDto` -- see `routing::HopKind`'s own doc for
/// why a teleport hop is never folded into a generic "shortcut": it names
/// its own spell so the frontend (and the player) can judge whether they
/// actually have access to it, rather than the backend assuming they do.
#[derive(Debug, Clone, Serialize)]
pub struct RouteHopDto {
    pub zone: String,
    pub kind: String,
    pub via_spell: Option<String>,
    pub distance: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ZoneRouteDto {
    pub hops: Vec<RouteHopDto>,
    pub total_distance: f64,
}

impl From<routing::ZoneRoute> for ZoneRouteDto {
    fn from(r: routing::ZoneRoute) -> Self {
        ZoneRouteDto {
            hops: r
                .hops
                .into_iter()
                .map(|h| {
                    let (kind, via_spell) = match h.kind {
                        routing::HopKind::Walk => ("walk".to_string(), None),
                        routing::HopKind::Teleport(spell) => ("teleport".to_string(), Some(spell)),
                        routing::HopKind::Succor => ("succor".to_string(), None),
                    };
                    RouteHopDto {
                        zone: h.zone,
                        kind,
                        via_spell,
                        distance: h.distance,
                    }
                })
                .collect(),
            total_distance: r.total_distance,
        }
    }
}

/// A route from `from_zone` to `to_zone` across zone lines and/or
/// teleport shortcuts, weighted by real in-zone walking distance -- see
/// `routing::find_zone_route`'s own doc for the two-stage (cheap
/// candidates, then real-distance scoring) design. Which teleport
/// shortcuts even get considered is gated by the log owner's own *assumed*
/// class/level -- the dominant (most zone-visits) confirmed configuration
/// from `combat::class_configurations`, and that configuration's own
/// `level_range` upper bound as the assumed level, the same "assumed"
/// framing the user asked for rather than chasing an exact per-class
/// level this app has no way to derive (`Ingest::levels` only ever tracks
/// one *effective* level across the whole loadout, not one per class --
/// see that struct's own doc). No confirmed configuration yet (a fresh
/// session, or a character below the level-10 fixed-3-classes rule) means
/// no teleport shortcuts are offered at all -- walk-only, not a guess.
/// The player's real, confirmed position in `from_zone` right now, in
/// map-file space -- a real `/loc` reading or a confirmed teleport
/// landing, whichever is more specific, or `None` if neither is available
/// *for that zone specifically*. Per the user's own direct point: a zone
/// entered via a recognized teleport cast, or a real `/loc` reading, is
/// 100% known -- exactly the "confirmed" tier docs/design/maps.md's "You
/// are here" ladder already uses for the map marker, now also feeding
/// `routing::find_zone_route`'s own first-hop distance rather than only
/// the visual overlay. Both sources need the same real, verified
/// transform a raw reading needs before it means anything in map-file
/// space -- see `Ingest::last_loc`'s own doc for the `(-y, -x, z)` mapping
/// this reapplies, and `entered_via_teleport`'s own callers (`MapViewer.
/// svelte`) for why that field shares the same raw coordinate space.
/// Zone-matched against `from_zone` independently for each source (a
/// `/loc` reading and the current teleport landing can each be stale in
/// different ways -- a `/loc` typed in a zone visited hours ago is not
/// "now", and neither is a landing from a zone visit that's already over).
fn live_start_position(
    ing: &crate::ingest::Ingest,
    base_dir: &std::path::Path,
    from_zone: &str,
) -> Option<(f32, f32, f32)> {
    // Real, reported bug this fixes: a real `/loc` reading used to win
    // unconditionally whenever its own zone matched, even when a *later*
    // teleport/Origin confirmation existed for the same zone -- an old
    // `/loc` typed before teleporting/Origin-ing back to a zone kept
    // outranking a fresher, equally-real confirmation just because `/loc`
    // was checked first, not because it was actually more recent. All
    // three real sources now compete on timestamp alone -- whichever one
    // is genuinely the newest *for this zone* wins, full stop. No
    // separate "prefer /loc, fall back to teleport" tiering left to get
    // this backwards again.
    let mut best: Option<(eqlp_source::Millis, (f32, f32, f32))> = None;
    let mut consider = |ts: eqlp_source::Millis, pos: (f32, f32, f32)| {
        if best.is_none_or(|(best_ts, _)| ts > best_ts) {
            best = Some((ts, pos));
        }
    };

    if let Some((ts, x, y, z)) = ing.last_loc {
        if ing
            .zone
            .at(ts)
            .is_some_and(|raw| crate::zone::zone_matches(raw, from_zone))
        {
            consider(ts, (-y as f32, -x as f32, z as f32));
        }
    }
    if let Some((ts, landing)) = &ing.entered_via_teleport {
        if ing
            .zone
            .at(*ts)
            .is_some_and(|raw| crate::zone::zone_matches(raw, from_zone))
        {
            consider(
                *ts,
                (-landing.y as f32, -landing.x as f32, landing.z as f32),
            );
        }
    }
    // Origin's own learned landing (see `Ingest::learned_origin`'s own
    // doc) -- a real zone, confirmed by direct observation, but no
    // wiki-quoted coordinate the way the two sources above have; `routing::
    // best_start_position`'s own succor-point lookup stands in once the
    // zone itself is known.
    if let Some((ts, raw)) = &ing.learned_origin {
        if crate::zone::zone_matches(raw, from_zone) {
            consider(*ts, routing::best_start_position(base_dir, from_zone));
        }
    }
    best.map(|(_, pos)| pos)
}

#[tauri::command]
pub fn find_zone_route(
    app: AppHandle,
    state: State<AppState>,
    from_zone: String,
    to_zone: String,
) -> Result<ZoneRouteDto, String> {
    let base_dir = {
        let cfg = state.config.lock().unwrap();
        cfg.as_ref()
            .ok_or("no install folder configured yet")?
            .base_dir
            .clone()
    };
    let (player_classes, player_level, known_start) = {
        let ing = state.ingest.lock().unwrap();
        let dto = combat::class_configurations(&ing, "You");
        let (live_classes, level) = dto
            .configurations
            .first()
            .map(|c| {
                (
                    c.classes.clone(),
                    c.level_range.map(|(_, hi)| hi).unwrap_or(0),
                )
            })
            .unwrap_or_default();
        // why: level always comes from *this* live session, never the
        // saved profile -- level changes constantly and `profile.rs`
        // never stores it (see that module's own doc). Classes fall back
        // to the saved profile only when this session's own replay
        // hasn't confirmed a configuration for "You" yet at all -- once
        // it has, live evidence wins outright, full stop, regardless of
        // what's saved. See `preferences::Preferences::save_profile`'s
        // own doc for the whole policy this is one half of.
        let player_classes = if !live_classes.is_empty() {
            live_classes
        } else if preferences::load(&app).save_profile {
            state
                .status
                .lock()
                .unwrap()
                .character
                .as_deref()
                .and_then(|c| profile::for_character(&app, c))
                .map(|p| p.classes)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        (
            player_classes,
            level,
            live_start_position(&ing, &base_dir, &from_zone),
        )
    };
    routing::find_zone_route(
        &base_dir,
        &from_zone,
        &to_zone,
        &player_classes,
        player_level,
        known_start,
    )
    .map(ZoneRouteDto::from)
    .ok_or_else(|| format!("no route found from {from_zone} to {to_zone}"))
}

#[derive(Debug, Clone, Serialize)]
pub struct LastLocationDto {
    pub ts_ms: eqlp_source::Millis,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// The raw `zone.enter` label current at `ts_ms` -- the frontend
    /// matches this (loosely, case-insensitive) against whichever map is
    /// currently open before showing the marker, so a stale `/loc` from a
    /// zone visited hours ago never gets plotted on an unrelated map.
    /// `None` if no zone was known yet at that instant.
    pub zone: Option<String>,
    /// `map_zones_for_raw_label(zone)` -- real map-file shortname(s) for
    /// `zone`, resolved independently of `ZoneContextDto::current_map_
    /// zones` (not just reused) since a `/loc` reading's own zone can lag
    /// behind "right now" by however long ago it was actually typed --
    /// see MapViewer.svelte for why matching on this instead of
    /// `zone_context`'s own resolution matters.
    pub map_zones: Vec<String>,
}

/// The Maps module's "you are here" marker -- the most recent `/loc`
/// reading, if the player has typed one this session. Rare (only fires
/// on the manual `/loc` command) -- `None` most of the time, and even
/// when present it's a timestamped snapshot, not a live position; the
/// frontend shows the timestamp alongside it rather than implying
/// continuous tracking. See `Ingest::last_loc`'s own doc.
#[tauri::command]
pub fn get_last_location(state: State<AppState>) -> Option<LastLocationDto> {
    let ing = state.ingest.lock().unwrap();
    let (ts_ms, x, y, z) = ing.last_loc?;
    let zone = ing.zone.at(ts_ms).map(str::to_string);
    let map_zones = map_zones_for_raw_label(zone.as_deref());
    Some(LastLocationDto {
        ts_ms,
        x,
        y,
        z,
        zone,
        map_zones,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ZoneContextDto {
    /// Raw `zone.enter` label current right now, if any.
    pub current: Option<String>,
    /// Raw `zone.enter` label of the visit immediately before this one --
    /// where the player almost certainly walked in *from*.
    pub previous: Option<String>,
    /// The exact, wiki-confirmed landing (if any) the *current* visit was
    /// entered via, rather than an ordinary zone-line walk -- see
    /// `Ingest::entered_via_teleport`'s own doc and `teleportdata`'s own
    /// doc for the coordinate-space caveat. When `Some`, the frontend
    /// plots this coordinate directly instead of the weaker `previous`-
    /// zone entrance guess.
    pub teleport_landing: Option<crate::teleportdata::TeleportLanding>,
    /// The confirming `zone.enter`'s own timestamp -- whichever source
    /// `teleport_landing` actually reflects, a real Gate/Translocate/
    /// Circle/Ring landing or an Origin-derived one, both count equally
    /// here. `None` exactly when `teleport_landing` is `None`. Real,
    /// reported bug this exists to fix: the frontend used to prefer a
    /// real `/loc` reading unconditionally whenever its own zone matched,
    /// even when a *later* teleport/Origin confirmation existed for that
    /// same zone -- an old `/loc` outranking fresher, equally-real
    /// evidence just because `/loc` was checked first, not because it was
    /// actually more recent. The frontend now compares this against its
    /// own `/loc` reading's timestamp and uses whichever is genuinely
    /// newer, the same "freshest wins" rule `commands::live_start_position`
    /// already applies backend-side for routing.
    pub teleport_landing_ts: Option<eqlp_source::Millis>,
    /// Real map-file shortname(s) for `current` (e.g. `["gukbottom"]` for
    /// "The Ruins of Old Guk 4 (Refined)"), via the wiki's own scraped
    /// `who_name` field -- see `zonedata::map_shortnames`'s own doc for
    /// why this replaces guessing a match from the raw label's text
    /// (which fails for most real zones: their internal map shortname
    /// bears no textual resemblance to the display name at all). The
    /// frontend's "is the map I have open actually my current zone" check
    /// is membership in this list, not a substring heuristic -- see
    /// MapViewer.svelte. Empty when `current` never resolved to a wiki
    /// zone, or that zone's own `who_name` is empty -- both real, stated
    /// gaps, not silently papered over with the old guess.
    pub current_map_zones: Vec<String>,
}

/// `map_shortnames` for whichever zone `raw` names. Matches directly
/// against `zonedata::zones()` via `zone::zone_matches` -- the same check
/// `Ingest::resolved_wiki_zone` does internally -- rather than reusing
/// `Ingest`'s own cache of it (`cached_wiki_zone`): that cache is only
/// ever primed as a side effect of `current_zone`, called when an
/// encounter needs stamping, which a zone with no combat yet (freshly
/// walked into, nothing fought) may never trigger -- confirmed by a real
/// test that hit exactly this gap. A fresh 117-entry linear scan, run at
/// most a few times a second (`get_zone_context` is a per-tick query, not
/// a hot per-line one), costs nothing worth caching for. Empty if `raw` is
/// `None`, never resolved to a wiki zone, or that zone carries no
/// `who_name` -- every one a real, honest "don't know", not a guess.
fn map_zones_for_raw_label(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else { return Vec::new() };
    zonedata::zones()
        .iter()
        .find(|z| crate::zone::zone_matches(raw, &z.name))
        .and_then(|z| z.who_name.as_deref())
        .map(zonedata::map_shortnames)
        .unwrap_or_default()
}

/// The Maps module's zone-identity + entrance-guess input. `current_map_
/// zones` (real map-file shortnames -- see its own doc) is what the
/// frontend now uses to confirm the currently-open map really is the
/// player's real current zone, for both the confirmed `/loc` dot and the
/// entrance guess -- `previous`/`teleport_landing` then decide *which*
/// entrance guess: a `to_<previous zone>` marker, or, if `teleport_landing`
/// is `Some`, that exact wiki-confirmed coordinate, used only when no real
/// `/loc` snapshot exists yet (the marker-matching fallback only applies
/// to the `previous`-zone path -- a known teleport landing is plotted
/// directly, with no marker-matching ambiguity at all).
#[tauri::command]
pub fn get_zone_context(state: State<AppState>) -> ZoneContextDto {
    let ing = state.ingest.lock().unwrap();
    let ts = ing.now_ms();
    let current = ing.zone.at(ts).map(str::to_string);
    let current_map_zones = map_zones_for_raw_label(current.as_deref());
    // Two real, independent confirmation sources -- a wiki-fixed teleport
    // (`entered_via_teleport`) and Origin's own learned landing (see
    // `Ingest::learned_origin`'s own doc) -- compete on timestamp, same
    // "freshest wins" rule `commands::live_start_position` applies for
    // routing. In practice they're almost never both set for the same
    // zone visit (each only fires from its own recent cast), but when
    // they are, recency decides it honestly rather than one kind always
    // beating the other. No `base_dir` configured yet is a real, honest
    // "can't compute a position" for the Origin side specifically, not an
    // error -- falls through to whichever other source is available.
    let wiki_landing = ing.entered_via_teleport.clone();
    let origin_landing = (|| {
        let (origin_ts, raw) = ing.learned_origin.as_ref()?;
        if !crate::zone::zone_matches(raw, current.as_deref()?) {
            return None;
        }
        let base_dir = state.config.lock().unwrap().as_ref()?.base_dir.clone();
        let (x, y, z) = routing::best_start_position(&base_dir, current.as_deref()?);
        Some((
            *origin_ts,
            crate::teleportdata::TeleportLanding {
                class: crate::teleportdata::TeleportClass::Any,
                x: x as f64,
                y: y as f64,
                z: z as f64,
                zone: current.clone().unwrap_or_default(),
                level: 1,
            },
        ))
    })();
    let (teleport_landing, teleport_landing_ts) = match (wiki_landing, origin_landing) {
        (Some((wts, _)), Some((ots, ol))) if ots > wts => (Some(ol), Some(ots)),
        (Some((wts, wl)), _) => (Some(wl), Some(wts)),
        (None, Some((ots, ol))) => (Some(ol), Some(ots)),
        (None, None) => (None, None),
    };
    ZoneContextDto {
        current,
        previous: ing.zone.label_before(ts).map(str::to_string),
        teleport_landing,
        teleport_landing_ts,
        current_map_zones,
    }
}

/// The Maps module's NPC-overlay candidate list -- every distinct real
/// `Npc::zone` value that loosely resembles the currently-open map's own
/// display name. Shown as toggle-able options, not auto-applied -- see
/// `npcdata::candidate_zones`'s own doc for why this app can't reliably
/// resolve the wiki's zone names against the map format's internal EQ
/// shortcodes on its own.
#[tauri::command]
pub fn list_npc_zone_candidates(map_zone_name: String) -> Vec<String> {
    npcdata::candidate_zones(&map_zone_name)
}

#[derive(Debug, Clone, Serialize)]
pub struct NpcMarkerDto {
    pub name: String,
    pub x: f32,
    pub y: f32,
    /// `None` when the wiki scrape only gave a 2D spot -- most real
    /// entries. The frontend has to pick *some* height to render these
    /// at either way; see MapViewer.svelte for how it handles that.
    pub z: Option<f32>,
}

/// Real NPC spawn points for `zone` (an exact `Npc::zone` value, already
/// resolved from `list_npc_zone_candidates`'s own output -- not a name to
/// fuzzy-match again here). See `npcdata::markers_for_zone`.
#[tauri::command]
pub fn get_npc_markers_for_zone(zone: String) -> Vec<NpcMarkerDto> {
    npcdata::markers_for_zone(&zone)
        .into_iter()
        .map(|(name, x, y, z)| NpcMarkerDto { name, x, y, z })
        .collect()
}

/// The Settings module's own list: `notifications::ALL_KINDS` paired with
/// its human label, so the frontend never needs to hardcode that mapping
/// itself and a fifth kind shows up here automatically once it's added to
/// `notifications.rs`.
#[derive(Debug, Clone, Serialize)]
pub struct NotificationKindDto {
    pub kind: String,
    pub label: String,
}

#[tauri::command]
pub fn list_notification_kinds() -> Vec<NotificationKindDto> {
    notifications::ALL_KINDS
        .iter()
        .map(|&kind| NotificationKindDto {
            kind: kind.to_string(),
            label: notifications::kind_label(kind).to_string(),
        })
        .collect()
}

/// Current enabled/custom-sound state for every kind -- `settings::
/// NotificationSettings` derives `Serialize` itself, so this is a direct
/// pass-through, not a separate DTO.
#[tauri::command]
pub fn get_notification_settings(app: AppHandle) -> settings::NotificationSettings {
    settings::load(&app)
}

#[tauri::command]
pub fn set_notification_enabled(
    app: AppHandle,
    kind: String,
    on: bool,
) -> Result<settings::NotificationSettings, String> {
    let mut s = settings::load(&app);
    s.set_enabled(&kind, on);
    settings::save(&app, &s)?;
    Ok(s)
}

/// Opens the OS file picker scoped to common audio extensions, copies
/// whatever the user chose into this app's own sounds directory (see
/// `settings::store_custom_sound`'s doc for why copied, not referenced by
/// its original path), and saves it as `kind`'s custom sound. `Ok(None)`
/// (not an error) if the user cancels the dialog -- same "cancel is a
/// real, unremarkable outcome" stance `pick_log_directory` already takes.
#[tauri::command]
pub async fn pick_notification_sound(
    app: AppHandle,
    kind: String,
) -> Result<Option<settings::NotificationSettings>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose a notification sound")
        .add_filter("Audio", &["mp3", "wav", "ogg", "m4a"])
        .pick_file(move |file| {
            let _ = tx.send(file);
        });
    let Some(file) = rx.await.ok().flatten() else {
        return Ok(None); // user cancelled
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let filename = settings::store_custom_sound(&app, &kind, &path)?;
    let mut s = settings::load(&app);
    s.set_custom_sound(&kind, Some(filename));
    settings::save(&app, &s)?;
    Ok(Some(s))
}

/// Reverts `kind` to the frontend's own synthesized default sound --
/// deletes the stored custom file (if any) and clears the setting.
#[tauri::command]
pub fn clear_notification_sound(
    app: AppHandle,
    kind: String,
) -> Result<settings::NotificationSettings, String> {
    let mut s = settings::load(&app);
    if let Some(filename) = s.custom_sound(&kind) {
        settings::delete_custom_sound(&app, filename);
    }
    s.set_custom_sound(&kind, None);
    settings::save(&app, &s)?;
    Ok(s)
}

/// `kind`'s custom sound, ready to hand straight to `new Audio(url)` --
/// `None` when there isn't one (the frontend falls back to its own
/// synthesized default in that case, not an error).
#[tauri::command]
pub fn get_notification_sound_data(app: AppHandle, kind: String) -> Option<String> {
    let s = settings::load(&app);
    settings::custom_sound_data_url(&app, &kind, &s)
}

/// The Spellbook builder's own file picker: every real `<Character>_
/// <Zone>_LO1.ini`/`UI_<Character>_<Zone>_LO1.ini` sitting in the game's
/// base folder -- see `uifiles::list_ui_files`'s own doc for what each
/// kind actually holds.
#[tauri::command]
pub fn list_ui_files(state: State<AppState>) -> Result<Vec<uifiles::UiFileInfoDto>, String> {
    let base_dir = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .ok_or("no install folder configured yet")?
        .base_dir
        .clone();
    Ok(uifiles::list_ui_files(&base_dir))
}

/// One UI file's real content, read-only -- see `uifiles::parse_ini`'s
/// own doc for why this doesn't write anything back yet.
#[tauri::command]
pub fn get_ui_file(
    state: State<AppState>,
    file: String,
) -> Result<uifiles::ParsedUiFileDto, String> {
    let base_dir = state
        .config
        .lock()
        .unwrap()
        .as_ref()
        .ok_or("no install folder configured yet")?
        .base_dir
        .clone();
    let path = uifiles::ui_file_path(&base_dir, &file).map_err(|e| e.to_string())?;
    uifiles::parse_ini(&path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod live_start_position_tests {
    use super::*;
    use std::path::Path;

    /// Real, reported bug this fixes: a real `/loc` reading used to win
    /// unconditionally whenever its own zone matched, even when a *later*
    /// teleport confirmation existed for that same zone -- an old `/loc`
    /// outranking fresher, equally-real evidence just because `/loc` was
    /// checked first, not because it was actually more recent. Recency
    /// alone must decide it now.
    #[test]
    fn a_later_teleport_confirmation_wins_over_an_earlier_loc_reading() {
        let mut ing = crate::ingest::Ingest::default();
        ing.zone.enter(1_000, "Oggok".to_string());
        ing.last_loc = Some((1_000, 100.0, 200.0, 5.0));
        // A later, fresher confirmation for the same zone.
        ing.zone.enter(2_000, "Oggok".to_string());
        ing.entered_via_teleport = Some((
            2_000,
            crate::teleportdata::TeleportLanding {
                class: crate::teleportdata::TeleportClass::Wizard,
                x: 300.0,
                y: 400.0,
                z: 10.0,
                zone: "Oggok".to_string(),
                level: 1,
            },
        ));
        let pos = live_start_position(&ing, Path::new("/nonexistent"), "Oggok");
        // /loc-space -> map-file transform: (-y, -x, z).
        assert_eq!(
            pos,
            Some((-400.0, -300.0, 10.0)),
            "the fresher teleport landing should win, not the earlier /loc"
        );
    }

    /// The reverse must also hold: a genuinely *fresher* `/loc` reading
    /// (typed after teleporting somewhere and then walking around) beats
    /// a now-stale teleport confirmation from earlier in the same visit.
    #[test]
    fn a_later_loc_reading_wins_over_an_earlier_teleport_confirmation() {
        let mut ing = crate::ingest::Ingest::default();
        ing.zone.enter(1_000, "Oggok".to_string());
        ing.entered_via_teleport = Some((
            1_000,
            crate::teleportdata::TeleportLanding {
                class: crate::teleportdata::TeleportClass::Wizard,
                x: 300.0,
                y: 400.0,
                z: 10.0,
                zone: "Oggok".to_string(),
                level: 1,
            },
        ));
        ing.last_loc = Some((2_000, 100.0, 200.0, 5.0));
        let pos = live_start_position(&ing, Path::new("/nonexistent"), "Oggok");
        assert_eq!(
            pos,
            Some((-200.0, -100.0, 5.0)),
            "the fresher /loc reading should win, not the earlier teleport landing"
        );
    }
}

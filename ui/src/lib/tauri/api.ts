// Typed wrappers, one per Tauri command this module actually uses so far
// -- field-for-field against the real Rust DTOs (crates/app/src/combat.rs,
// commands.rs, tail_worker.rs), wire format left snake_case (what serde
// actually emits; no camelCase transform layer). Extend this file
// alongside each later module's own port, not ahead of it.
//
// Every call goes through `invoke` from `./invoke`, never `@tauri-apps/
// api` directly -- that's the one place real-vs-mock is decided (see
// `docs/ci.md`'s "mock IPC harness").

import { invoke } from './invoke';

// ---------------------------------------------------------------- status

export interface LineCounts {
  total: number;
  matched: number;
  unmatched: number;
  headerless: number;
  blank: number;
  by_kind: Record<string, number>;
}

export interface TailStatus {
  log_dir: string | null;
  file: string | null;
  character: string | null;
  server: string | null;
  watching: boolean;
  tail_status: string;
  backfilling: boolean;
  pets_attributed: number;
}

export interface StatusDto {
  configured: boolean;
  status: TailStatus;
  counts: LineCounts;
}

// ---------------------------------------------------------------- combat

export interface ZoneVisitDto {
  index: number | null;
  label: string;
  fight_count: number;
  current: boolean;
}

export interface EncounterDto {
  id: number;
  target: string;
  entities: string[];
  start_ms: number;
  end_ms: number | null;
  duration_ms: number;
  total_damage: number;
  dps: number;
  enemy_damage: number;
  enemy_dps: number;
  slain: boolean;
  wiped: boolean;
  open: boolean;
}

export interface AbilityRowDto {
  ability: string;
  tags: string[];
  total: number;
  hits: number;
  min: number;
  max: number;
  crits: number;
  avg_hit: number;
  avg_crit: number;
  pct: number;
  missed: number;
  blocked: number;
  dodged: number;
  parried: number;
}

export interface CastRowDto {
  spell: string;
  attempts: number;
  landed: number;
  resisted: number;
  interrupted: number;
  fizzled: number;
  unconfirmed: number;
}

export interface CombatSummaryDto {
  fight_count: number;
  total_damage: number;
  duration_ms: number;
  dps: number;
  enemy_damage: number;
  enemy_dps: number;
  abilities: AbilityRowDto[];
  casts: CastRowDto[];
}

export interface AllyDto {
  name: string;
  is_player: boolean;
  is_pet: boolean;
  total: number;
  hits: number;
  crits: number;
  crit_pct: number;
  dps: number;
  pct: number;
}

export interface EntitySeriesDto {
  name: string;
  is_player: boolean;
  is_pet: boolean;
  is_enemy: boolean;
  total: number;
  values: number[];
}

export interface FightTimelineDto {
  start_ms: number;
  duration_ms: number;
  bucket_ms: number;
  buckets: number[];
  series: EntitySeriesDto[];
}

export interface EntityStateDto {
  name: string;
  is_player: boolean;
  is_pet: boolean;
  is_enemy: boolean;
  state: string;
  observed: boolean;
  dps: number;
  /** Recognized buff/effect landing text, recent as of this instant --
   * recency, not a live "still active" claim (the log has no wears-off
   * line for these). Only ever populated for "You". */
  recent_effects: string[];
}

// ---------------------------------------------------------------- character

export interface ClassConfigurationDto {
  classes: string[];
  zone_visits: number;
  level_range: [number, number] | null;
}

export interface ClassConfigurationsDto {
  configurations: ClassConfigurationDto[];
  unresolved_visits: number;
}

// ---------------------------------------------------------------- endgame

export interface RaidDropDto {
  item: string;
  looted: boolean;
  /** why: total quantity looted so far, 0 for a wiki-known drop never gotten */
  count: number;
}

/** why: one boss or miniboss -- a raid's own `boss` and each of its `minibosses` share this shape */
export interface RaidTargetDto {
  name: string;
  /** why: the wiki's own raw level text ("66", "55-56", "?") -- never parsed to a number */
  level: string | null;
  kills: number;
  /** why: index 0 = base/untiered, 1-4 = Awakened/Adaptive/Fused/Refined, confirmed while the
   * zone was in its Solo form -- see zone::zone_tier and raiding.rs's own doc on Solo vs Group */
  solo_tiers_cleared: [boolean, boolean, boolean, boolean, boolean];
  /** why: same 5-tier scale, confirmed while the zone was in its "- Group" form -- a genuinely
   * different real instance, not a duplicate of solo_tiers_cleared */
  group_tiers_cleared: [boolean, boolean, boolean, boolean, boolean];
  drops: RaidDropDto[];
}

/** why: one confirmed best time -- duration plus when that run happened, so the UI
 * can show "achieved <date>" as real evidence, not a bare number */
export interface BestTimeDto {
  duration_ms: number;
  achieved_ms: number;
}

/** why: a real speedrun timer, not a completion metric, split the same way
 * RaidTargetDto's own difficulty grid is (index = tier 0-4, solo/group) -- see the
 * Rust RaidTimesDto's own doc for why "full clear" isn't computed yet at all. */
export interface RaidTimesDto {
  solo: [BestTimeDto | null, BestTimeDto | null, BestTimeDto | null, BestTimeDto | null, BestTimeDto | null];
  group: [BestTimeDto | null, BestTimeDto | null, BestTimeDto | null, BestTimeDto | null, BestTimeDto | null];
}

export interface RaidDto {
  zone: string;
  boss: RaidTargetDto;
  /** why: empty for a raid with no separate named minibosses (e.g. Lady Vox) */
  minibosses: RaidTargetDto[];
  times: RaidTimesDto;
}

export interface RaidRowDto {
  row: string;
  raids: RaidDto[];
}

// -------------------------------------------------------- sky class unlocks

export interface TurnInItemDto {
  item: string;
  /** why: which island/boss the wiki names as this item's own source, e.g. "3-Gorga" */
  source: string | null;
  ever_looted: boolean;
  looted_count: number;
  /** why: null if no /outputfile inventory dump exists yet -- unknown, not zero */
  currently_owned: number | null;
  /** why: looted at some point, but auto-sold rather than kept -- not sitting in storage */
  sold_without_keeping: boolean;
}

export interface TurnInDto {
  quest: string;
  trigger: string;
  rune: TurnInItemDto | null;
  items: TurnInItemDto[];
  reward: string | null;
  /** why: real achievement-confirmed completion (Achievements.txt), null if no dump found yet */
  completed: boolean | null;
}

/** why: the Sky Quests tab -- every individual material turn-in (rune + drop items -> one gear
 * reward), full detail. The *final* reward items themselves are a separate DTO
 * (SkyClassUnlockDto) -- see the Rust skyquests.rs module doc for why the two are split. */
export interface SkyClassDto {
  class: string;
  quest_giver: string | null;
  quests: TurnInDto[];
  /** why: real achievement-confirmed, null if no Achievements dump found yet */
  unlocked: boolean | null;
}

/** why: one final reward item -- what "Primary Class Unlocks" tracks, never the raw
 * materials a quest is built from */
/** why: name plus where the wiki says it comes from -- no loot/ownership tracking of its
 * own, that's the Sky Quests tab's own job */
export interface QuestMaterialDto {
  item: string;
  source: string | null;
}

export interface SkyRewardDto {
  name: string;
  /** why: which quest earns this reward -- context only */
  quest: string;
  ever_looted: boolean;
  looted_count: number;
  currently_owned: number | null;
  sold_without_keeping: boolean;
  completed: boolean | null;
  /** why: rune first, then drop items, in quest order -- where a not-yet-secured reward
   * actually comes from */
  materials: QuestMaterialDto[];
}

export interface SkyClassUnlockDto {
  class: string;
  quest_giver: string | null;
  unlocked: boolean | null;
  /** why: the final reward items only (e.g. Bard: Mask of Song, Mantle of the Songweaver,
   * Ervaj's Flute of Flight, Amulet of the Fae, Denon's Horn of Disaster, Spear of Harmony) --
   * never the Wind Runes/drop items each is built from */
  rewards: SkyRewardDto[];
}

// ------------------------------------------------------------------ ui files

/** why: EQ's own per-character UI config files sitting in the game's base install folder --
 * see the Rust uifiles.rs module doc for what each real kind (hotbuttons vs layout) holds */
export interface UiFileInfoDto {
  file: string;
  character: string;
  zone: string;
  /** why: "hotbuttons" (<Character>_<Zone>_LO1.ini, real button contents) or "layout"
   * (UI_<Character>_<Zone>_LO1.ini, window position/size only, never contents) */
  kind: 'hotbuttons' | 'layout';
  is_backup: boolean;
}

export interface UiSectionDto {
  name: string;
  /** why: [key, value] pairs, in file order */
  entries: [string, string][];
}

export interface ParsedUiFileDto {
  sections: UiSectionDto[];
  /** why: nonzero flags a real corrupted file (unrelated pasted text before the first
   * real [Section] header) -- 0 for an ordinary clean file */
  skipped_garbage_lines: number;
}

/** why: one real slot in a saved [SpellLoadouts] entry -- see spellbookfiles.rs's own doc.
 * Named LoadoutSlotDto, not SpellSlotDto -- that name's already taken by SpellDto's own raw
 * wiki-slot-text shape below, a completely different thing. */
export interface LoadoutSlotDto {
  slot: number;
  /** why: -1 is the real "empty" sentinel the game's own files use */
  spell_id: number;
  /** why: resolved via the install folder's own spells_us.txt; null for an empty slot */
  name: string | null;
  /** why: packs/spells.json's own id, best-effort name match -- null is common, not an error */
  catalog_id: string | null;
}

export interface SpellLoadoutDto {
  index: number;
  in_use: boolean;
  name: string | null;
  /** why: always 14 long when in_use, empty when not */
  slots: LoadoutSlotDto[];
}

export interface SpellbookFileDto {
  file: string;
  /** why: always all 60 real loadout slots, most typically unused -- save_spellbook_file
   * expects the same full shape back */
  loadouts: SpellLoadoutDto[];
}

export interface CostModifier {
  kind: string;
  scope: string;
  per_rank_pct: number[];
}

export interface AaGrantDto {
  ts_ms: number;
  name: string;
  rank: number;
  cost: number;
  category: string | null;
  description: string | null;
  max_rank: number | null;
  cost_progression: string | null;
  catalog_certain: boolean | null;
  relevant_stats: string[];
  cost_modifiers: CostModifier[];
}

export interface AaLogDto {
  grants: AaGrantDto[];
  total_spent: number;
}

export interface AaDto {
  name: string;
  category: string;
  ranks: number;
  cost_raw: string;
  certain: boolean;
  description: string | null;
}

export interface SpellClassDto {
  class: string;
  level: number | null;
}

/** A catalog spell with a parseable damage effect, rank-adjusted -- see `dpscalc`'s own module doc (Rust) for the model. */
export interface DamageSpellDto {
  name: string;
  icon: string | null;
  classes: SpellClassDto[];
  /** What this spell's own damage checks against, e.g. "Cold (-10)"; null for Unresistable. */
  resist: string | null;
  is_dot: boolean;
  /** This session's own observed live rank, 0 if never cast this session. */
  rank: number;
  /** DoTs only -- rank-adjusted duration in seconds. */
  duration_secs: number | null;
  mana: number;
  casting_time: number;
  recast_time: number;
  /** Full damage from one application, rank-adjusted. */
  total_damage: number;
  /** Portion of total_damage that's genuinely instant -- all of it for a nuke; for a DoT, just its one-time "on cast" component (0 for most). */
  instant_damage: number;
  dpm: number;
  dps_with_reuse: number;
  /** No reuse wait -- instant_damage per second of casting time invested. For a DoT this is NOT its tick-stream rate (see instant_damage's own doc); use dps_with_reuse for "is this DoT worth maintaining". */
  dps_ignoring_reuse: number;
}

export interface SpellbookEntryDto {
  name: string;
  confidence: 'known' | 'possible';
  first_seen_ms: number;
  description: string | null;
  mana: number | null;
  casting_time: number | null;
  recast_time: number | null;
  duration: string | null;
  target_type: string | null;
  resist: string | null;
  classes: SpellClassDto[];
  icon: string | null;
}

export interface AttrRowDto {
  attr: string;
  base: number;
  class_adds: number[];
  naked: number;
  gear: number;
  total: number;
}

export interface ClassManaDto {
  class: string;
  casting_stat: string;
  pool: number;
  counted: boolean;
}

export interface VitalsDto {
  hp: number;
  ac: number;
  attack: number;
  velocity: number;
  endurance: number;
  hp_regen: number;
  mana_regen: number;
  end_regen: number;
}

export interface ResistsDto {
  magic: number;
  fire: number;
  cold: number;
  disease: number;
  poison: number;
  void: number;
}

export interface CharacterEstimateDto {
  race: string;
  classes: string[];
  class_levels: number[];
  character_level: number;
  limiting_class: string | null;
  attrs: AttrRowDto[];
  mana: ClassManaDto[];
  total_mana: number;
  vitals: VitalsDto;
  resists: ResistsDto;
  attr_cap: number;
  verified: boolean;
  bad_class_adds: string[];
}

export interface ItemEffectDto {
  name: string;
  detail: string | null;
}

export interface ExaltSlotDto {
  key: string;
  label: string;
  req_tier: number;
  unlocked: boolean;
  effect: ItemEffectDto | null;
}

export interface ItemDto {
  id: string;
  name: string;
  tags: string[];
  slots: string[];
  classes: string[];
  stats: Record<string, number>;
  dmg: number | null;
  delay: number | null;
  skill: string | null;
  era: string | null;
  icon: string | null;
  /** First drop source, "zone — mob" formatted (or a non-drop source). */
  source: string | null;
  zones: string[];
  mobs: string[];
  url: string | null;
  wt: number | null;
  size: string | null;
  tier: number;
  /** why: copies owned anywhere (bags/bank/equipped), 0 with no dump loaded */
  owned: number;
  /** why: this item's own native effects, keyed focus/click/worn/proc */
  effects: Record<string, ItemEffectDto>;
  /** why: the 5 exaltation sockets, evaluated at this item's own tier */
  exalts: ExaltSlotDto[];
  /** why: real "(Exaltation)" log evidence this specific equipped item's
   * proc has actually fired -- only ever set by getInventoryDump, never
   * for a browsed/recommended item. Never says which effect resulted,
   * only that the socket is genuinely live. */
  proc_evidence: ProcEvidenceDto | null;
}

export interface ProcEvidenceDto {
  fires: number;
  first_seen_ms: number;
}

export interface ScoredItemDto extends ItemDto {
  score: number;
}

export interface SlotRecommendationDto {
  slot: string;
  items: ScoredItemDto[];
}

export interface InventoryDumpDto {
  /** why: slot -> matched item */
  resolved: Record<string, ItemDto>;
  /** why: slot -> unmatched printed name */
  unresolved: Record<string, string>;
  /** why: base item name -> total copies owned, whole dump summed */
  owned: Record<string, number>;
  /** why: base item name -> highest "+N" tier owned of it */
  owned_tier: Record<string, number>;
}

// ---------------------------------------------------------------- maps

/** why: a wall/boundary segment, in the map file's own coordinate order --
 * not necessarily the same order `/loc` prints in, see LastLocationDto. */
export interface MapLineDto {
  a: [number, number, number];
  b: [number, number, number];
  color: [number, number, number];
}

export interface MapMarkerDto {
  pos: [number, number, number];
  color: [number, number, number];
  size: number;
  label: string;
}

export interface MapFileDto {
  lines: MapLineDto[];
  markers: MapMarkerDto[];
}

/** why: a snapshot, not live tracking -- only set when the player types
 * "/loc", which the reference log shows happening rarely. Always show
 * ts_ms alongside it rather than implying a continuously moving dot. */
export interface LastLocationDto {
  ts_ms: number;
  x: number;
  y: number;
  z: number;
  /** why: the raw zone.enter label at ts_ms -- match loosely against the
   * currently-open map before showing the marker, see MapViewer.svelte */
  zone: string | null;
  /** why: real map-file shortname(s) for `zone`, resolved independently
   * of ZoneContextDto's own (a /loc reading can lag "right now" by
   * however long ago it was typed) -- see MapViewer.svelte. */
  map_zones: string[];
}

/** why: the exact, wiki-confirmed landing spot for a recognized teleport
 * cast (Wizard Translocate/Gate/Portal, Druid Circle/Ring) -- see
 * `crates/app/src/teleportdata.rs`'s own doc for how this is sourced
 * (eqlwiki states the destination coordinate directly on the spell's own
 * page) and its stated coordinate-space caveat (assumed to match `/loc`
 * output, not independently re-verified against a real `/loc` reading --
 * no real log data point exists to check that against). Coordinate order
 * matches `LastLocationDto`'s -- apply the same `(-y, -x, z)` transform
 * before plotting, see MapViewer.svelte. */
export interface TeleportLandingDto {
  /** why: `'any'` is not from the wiki-scraped pack -- it's Origin's own
   * *learned* landing (see `Ingest::learned_origin`'s own doc), built at
   * query time once a real cast has confirmed which zone it actually
   * sends this character to. No fixed class the way Wizard's Translocate/
   * Gate/Portal or Druid's Circle/Ring are. */
  class: 'wizard' | 'druid' | 'any';
  x: number;
  y: number;
  z: number;
}

/** why: the player's current + immediately-prior zone, raw log labels --
 * feeds the entrance guess (a `to_<previous>` marker, or the exact
 * `teleport_landing` coordinate when set) when no real `/loc` exists yet
 * for the currently-open map. See MapViewer.svelte. */
export interface ZoneContextDto {
  current: string | null;
  previous: string | null;
  /** why: `null` for an ordinary zone-line walk or an unrecognized/
   * uncovered teleport spell; set when "You" or a proven ally cast a
   * teleport spell with a known wiki-confirmed landing shortly before
   * this visit began -- see `Ingest::entered_via_teleport`'s own doc. */
  teleport_landing: TeleportLandingDto | null;
  /** why: the confirming timestamp behind `teleport_landing` (whichever of
   * the wiki-landing or Origin-derived candidate is newer -- see
   * `commands::get_zone_context`'s own doc) so callers can compare it
   * against a real `/loc` reading's own `ts_ms` and use whichever is
   * genuinely more recent, instead of one source always winning outright.
   * `null` iff `teleport_landing` is `null`. */
  teleport_landing_ts: number | null;
  /** why: real map-file shortname(s) for `current`, from the wiki's own
   * scraped who_name field -- what "is the map I have open actually my
   * current zone" checks membership in now, replacing a substring guess
   * that silently failed for most real zones (their internal map
   * shortname bears no resemblance to the display name at all -- e.g.
   * "gukbottom" for "The Ruins of Old Guk"). Empty when unresolvable. */
  current_map_zones: string[];
}

/** why: a real walking route within one zone's map, waypoint by waypoint
 * -- see `crates/app/src/pathfind.rs`'s own doc for what "real" means
 * here (grid A* over the zone's own wall geometry, Z-banded to the
 * *starting* point's own floor) and its stated limits (a route needing a
 * floor change within the zone isn't found, and a narrow enough real
 * structure can still be missed by the grid's own resolution). */
export interface PathDto {
  waypoints: [number, number, number][];
}

/** why: one leg of a ZoneRouteDto -- a teleport hop always names its own
 * spell (`via_spell`) rather than folding into a generic "shortcut", so
 * the UI can show plainly that it requires a specific class/spell the
 * player might not have -- see `routing::TELEPORT_HOP_COST`'s own doc for
 * why the backend doesn't (and can't) know whether the player actually
 * has access to it. A `'succor'` hop arrives in the *same* zone the walk
 * hop after it starts from (see `routing::HopKind::Succor`'s own doc) --
 * a real, separate, required action (Lesser Evacuate, or a
 * difficulty-tier change), not folded into that walk hop's own distance
 * the way it used to be. */
export interface RouteHopDto {
  zone: string;
  kind: 'walk' | 'teleport' | 'succor';
  via_spell: string | null;
  distance: number;
}

export interface ZoneRouteDto {
  hops: RouteHopDto[];
  total_distance: number;
}

/** why: a real wiki NPC spawn point -- z is null for most real entries
 * (the scrape only gives 2D for the majority of mobs), see
 * MapViewer.svelte for how that's rendered. */
export interface NpcMarkerDto {
  name: string;
  x: number;
  y: number;
  z: number | null;
}

// ---------------------------------------------------------------- game data

export interface ZoneDto {
  id: string;
  name: string;
  url: string;
  level_range: string | null;
  monster_types: string | null;
  notable_npcs: string[];
  city_races: string[];
  guilds: string[];
  tradeskill_facilities: string[];
  related_quests: string[];
  unique_items: string[];
  adjacent_zones: string[];
  spawn_timer: string | null;
  who_name: string | null;
  succor_evacuate: string | null;
  image: string | null;
  era: string | null;
  categories: string[];
}

export interface KnownLootDto {
  item: string;
  rarity: string | null;
  stack: number | null;
  chance_per_kill: number | null;
  chance_per_drop: number | null;
}

export interface NpcDto {
  id: string;
  name: string;
  url: string;
  race: string | null;
  class: string | null;
  level: string | null;
  zone: string | null;
  location: string | null;
  respawn_time: string | null;
  aggro_radius: number | null;
  run_speed: number | null;
  AC: number | null;
  HP: number | null;
  hp_regen: number | null;
  mana_regen: number | null;
  attacks_per_round: number | null;
  attack_speed: string | null;
  damage_per_hit: string | null;
  special: string | null;
  known_loot: KnownLootDto[];
  imagefilename: string | null;
  images: string[];
  era: string | null;
  categories: string[];
}

export interface SpellSlotDto {
  slot: number;
  effect: string;
}

export interface SpellDto {
  id: string;
  name: string;
  url: string | null;
  description: string | null;
  classes: SpellClassDto[];
  skill: string | null;
  mana: number | null;
  range: number | null;
  casting_time: number | null;
  fizzle_time: number | null;
  recast_time: number | null;
  duration: string | null;
  target_type: string | null;
  spell_type: string | null;
  resist: string | null;
  msg_cast_on_you: string | null;
  msg_cast_on_other: string | null;
  msg_wears_off: string | null;
  slots: SpellSlotDto[];
  items_with_effect: string[];
  where_to_obtain: string | null;
  era: string | null;
  categories: string[];
  icon: string | null;
}

export interface SpellDurationDto {
  min_secs: number | null;
  max_secs: number | null;
  is_instant: boolean;
  is_permanent: boolean;
  raw: string | null;
}

export interface EffectComponentDto {
  stat: string;
  direction: string;
  per_tick: boolean;
  unit: string;
  min_amount: number | null;
  max_amount: number | null;
  raw: string;
}

export interface DescriptionAmountDto {
  min_amount: number;
  max_amount: number;
  is_over_time: boolean;
  repetitions: number | null;
}

export interface SpellEffectsEntryDto {
  id: string;
  duration: SpellDurationDto;
  components: EffectComponentDto[];
  control: string[];
  description_damage: DescriptionAmountDto | null;
  description_heal: DescriptionAmountDto | null;
  tags: string[];
}

export interface MobStatsDto {
  kills: number;
  pulls: number;
}

export interface EncounterPreviewDto {
  id: number;
  target: string;
  start_ms: number;
  end_ms: number | null;
  duration_ms: number;
  slain: boolean;
  wiped: boolean;
  open: boolean;
}

export interface ZoneEncounterDto {
  encounter: EncounterPreviewDto;
  /** why: 0-4, zone::zone_tier's own difficulty scale */
  tier: number;
  zone_visit: number | null;
  /** why: only meaningful on an NPC's own encounter list -- the same mob can turn up in more than one zone */
  zone: string | null;
}

export interface EncounterDropDto {
  item: string;
  qty: number;
  /** why: lower is rarer; null when there's nothing to rank by */
  chance: number | null;
}

export interface EncounterDetailDto {
  total_damage: number;
  dps: number;
  enemy_damage: number;
  enemy_dps: number;
  drops: EncounterDropDto[];
}

export interface LootEventDto {
  ts_ms: number;
  mob: string;
  qty: number;
  zone: string | null;
}

// ---------------------------------------------------------------- preferences

export interface EraOptionsDto {
  /** why: ERA_ORDER, oldest first -- "All" is a frontend-added option, not one of these */
  eras: string[];
  current: string;
}

export interface PreferencesDto {
  /** why: 0-100, not yet wired to any actual sound playback */
  volume: number;
  /** why: an eras[] name, "All", or null -- null means "follow current era" */
  era: string | null;
  /** why: false (default) = every launch replays the log and lets class
   * detection reconfirm everything fresh, same as always. true = also
   * keeps a saved per-character class profile across restarts, used only
   * as a fallback for zone routing when this session's own live replay
   * hasn't confirmed a configuration for "You" yet -- see the Rust
   * `Preferences::save_profile` field's own doc for the full policy. */
  save_profile: boolean;
  /** why: 'public' (default) checks the `latest` GitHub release (main,
   * deliberate releases only); 'beta' checks `testing` (every push to
   * `testing`, prerelease) -- see `.github/workflows/3-release.yml`. */
  update_channel: 'public' | 'beta';
}

export interface UpdateInfoDto {
  version: string;
  current_version: string;
  /** why: the release's own body text, whatever that channel's notes say -- may be empty */
  notes: string | null;
}

// ---------------------------------------------------------------- history

export interface ParseRecordDto {
  target: string;
  zone: string;
  loadout: string[];
  /** why: which zone visit this fight belongs to -- backend-only key used
   * to re-resolve `loadout` against live class evidence; frontend has no
   * use for the raw index itself. */
  zone_visit: number | null;
  start_ms: number;
  duration_ms: number;
  player_damage: number;
  player_dps: number;
  confirmed_kill: boolean;
  /** why: null if no baseline yet; always null for backfilled records */
  score_ratio: number | null;
}

export interface LoadoutSummaryDto {
  loadout: string[];
  fights: number;
  confirmed_kills: number;
  avg_dps: number;
  avg_score_ratio: number | null;
}

// ---------------------------------------------------------------- debug

export interface DebugEncounterDto {
  id: number;
  target: string;
  start_ms: number;
  duration_ms: number;
  raw_zone: string | null;
  resolved_zone_id: string | null;
  tier: number;
  player_classes: string[];
}

export interface UnmatchedShapeDto {
  shape: string;
  count: number;
  example: string;
}

export interface UnmatchedCoverageDto {
  shapes: UnmatchedShapeDto[];
  distinct_shapes: number;
  shapes_overflow: number;
  unmatched_total: number;
  total_lines: number;
}

export const api = {
  getStatus: () => invoke<StatusDto>('get_status'),

  pickLogDirectory: () => invoke<string | null>('pick_log_directory'),

  setLogDirectory: (path: string) => invoke<StatusDto>('set_log_directory', { path }),

  listZoneVisits: () => invoke<ZoneVisitDto[]>('list_zone_visits'),

  /** why: offset/limit are optional -- omitted means "the whole list" (the
   * backend defaults to that). The fight dropdown can run into the
   * thousands for a long-lived character, but that turned out to be a
   * *rendering* problem, not a fetch-cost one -- see Combat.svelte's own
   * row virtualization, which is what actually keeps this bounded now. */
  listEncounters: (zoneVisit: number | null, offset?: number, limit?: number) =>
    invoke<EncounterDto[]>('list_encounters', { zoneVisit, offset, limit }),

  getCombatSummary: (zoneVisit: number | null, encounterId: number | null, actor: string | null = null) =>
    invoke<CombatSummaryDto>('get_combat_summary', { zoneVisit, encounterId, actor }),

  listAllies: (zoneVisit: number | null, encounterId: number | null) =>
    invoke<AllyDto[]>('list_allies', { zoneVisit, encounterId }),

  getFightTimeline: (encounterId: number) => invoke<FightTimelineDto | null>('get_fight_timeline', { encounterId }),

  getFightStateAt: (encounterId: number, tsMs: number) =>
    invoke<EntityStateDto[]>('get_fight_state_at', { encounterId, tsMs }),

  // -------------------------------------------------------------- character

  // `name` is always the literal "You" -- the log is first-person, so the
  // player's own actions land under that exact symbol in the store, not
  // their character's real name. See dump_fixtures.rs's own note.
  getClassConfigurations: () => invoke<ClassConfigurationsDto>('get_class_configurations', { name: 'You' }),

  getConfigurationZoneVisits: (classes: string[]) =>
    invoke<ZoneVisitDto[]>('get_configuration_zone_visits', { name: 'You', classes }),

  getCurrentLevel: () => invoke<number | null>('get_current_level'),

  getDefaultGearClasses: () => invoke<string[]>('get_default_gear_classes', { name: 'You' }),

  getAaLog: () => invoke<AaLogDto>('get_aa_log'),

  listAa: () => invoke<AaDto[]>('list_aa'),

  getSpellbook: () => invoke<SpellbookEntryDto[]>('get_spellbook'),

  /** Highest live in-game rank observed cast this session, by catalog base spell name -- e.g. `{ "Ice Comet": 10 }`. */
  getSpellRanks: () => invoke<Record<string, number>>('get_spell_ranks'),

  /** `assumeMaxRank`: substitutes a flat rank 10 for every spell instead of this session's observed rank. */
  getDamageSpells: (assumeMaxRank: boolean) => invoke<DamageSpellDto[]>('get_damage_spells', { assumeMaxRank }),

  getCharacterEstimate: (race: string, classes: string[], classLevels: number[], gear: Record<string, number>) =>
    invoke<CharacterEstimateDto | null>('get_character_estimate', { race, classes, classLevels, gear }),

  getGearRecommendations: (
    classes: string[],
    race: string | null,
    level: number | null,
    equipped: Record<string, string> | null = null,
    owned: Record<string, number> | null = null,
    ownedTier: Record<string, number> | null = null,
    maxEra: string | null = null,
  ) =>
    invoke<SlotRecommendationDto[]>('get_gear_recommendations', {
      classes,
      race,
      maxEra,
      perSlot: 30,
      weights: null,
      level,
      equipped,
      owned,
      ownedTier,
    }),

  getGearWeights: (classes: string[], level: number | null) =>
    invoke<Record<string, number>>('get_gear_weights', { classes, level }),

  getInventoryDump: (file: string) => invoke<InventoryDumpDto>('get_inventory_dump', { file }),

  /** why: doll/preview tier picker -- "what if I upgrade this to +N" */
  getItemAtTier: (id: string, tier: number) => invoke<ItemDto | null>('get_item_at_tier', { id, tier }),

  /** why: re-derives an item's exalts with sources socketed in, instead of its own native effects */
  getItemWithExalts: (id: string, tier: number, exalts: Record<string, string>) =>
    invoke<ItemDto | null>('get_item_with_exalts', { id, tier, exalts }),

  /** why: legal ("relevant") sources for one open exaltation socket */
  getExaltCandidates: (
    id: string,
    socketKey: string,
    otherAssignments: Record<string, string>,
    classes: string[],
    maxEra: string | null = null,
  ) => invoke<ItemDto[]>('get_exalt_candidates', { id, socketKey, otherAssignments, classes, maxEra }),

  findExistingInventoryDump: () => invoke<{ file: string; character: string | null } | null>('find_existing_inventory_dump'),

  /** why: subfolders of maps/ under the game install (e.g. "Brewall") -- used by Settings' "N packs known" display. */
  listMapPacks: () => invoke<string[]>('list_map_packs'),
  /** why: zone picker -- every zone with a map file under maps/ (or maps/<pack> when pack is given) */
  listMapZones: (pack: string | null = null) => invoke<string[]>('list_map_zones', { pack }),
  /** why: the Maps module's zone-first picker -- every zone with a map anywhere (base game or any pack), deduped */
  listAllMapZones: () => invoke<string[]>('list_all_map_zones'),
  /** why: which source(s) cover `zone` -- null = base game, else a pack name -- drives the "available versions" picker */
  listZoneVersions: (zone: string) => invoke<(string | null)[]>('list_zone_versions', { zone }),
  getMapFile: (pack: string | null, zone: string) => invoke<MapFileDto>('get_map_file', { pack, zone }),
  /** why: a real walking route within one zone's map -- rejects (not just empty-array) when no route exists, see PathDto's own doc */
  findWalkPath: (pack: string | null, zone: string, from: [number, number, number], to: [number, number, number]) =>
    invoke<PathDto>('find_walk_path', { pack, zone, from, to }),
  /** why: a real cross-zone route, weighted by real in-zone walking distance -- see ZoneRouteDto's own doc */
  findZoneRoute: (fromZone: string, toZone: string) => invoke<ZoneRouteDto>('find_zone_route', { fromZone, toZone }),
  /** why: the most recent "/loc" reading this session, if any -- a snapshot, not live tracking */
  getLastLocation: () => invoke<LastLocationDto | null>('get_last_location'),
  /** why: current + previous zone labels, for the entrance guess when no real /loc exists yet */
  getZoneContext: () => invoke<ZoneContextDto>('get_zone_context'),
  /** why: fuzzy candidates only -- see npcdata::candidate_zones' own doc for why this can't be exact */
  listNpcZoneCandidates: (mapZoneName: string) => invoke<string[]>('list_npc_zone_candidates', { mapZoneName }),
  getNpcMarkersForZone: (zone: string) => invoke<NpcMarkerDto[]>('get_npc_markers_for_zone', { zone }),

  /** why: the item browser's unfiltered catalog -- Game Data's Items tab */
  listGearItems: (classes: string[] = [], slot: string | null = null, maxEra: string | null = null) =>
    invoke<ItemDto[]>('list_gear_items', { classes, slot, maxEra, owned: null, ownedTier: null }),

  // -------------------------------------------------------------- game data

  listZones: () => invoke<ZoneDto[]>('list_zones'),

  listNpcs: () => invoke<NpcDto[]>('list_npcs'),

  /** why: log mob name -> wiki Npc name, for gdFind's own npc lookup --
   * see mobalias.rs's own doc for a real example (Innoruuk). */
  getMobAliases: () => invoke<[string, string][]>('get_mob_aliases'),

  listSpells: () => invoke<SpellDto[]>('list_spells'),

  listSpellEffects: () => invoke<SpellEffectsEntryDto[]>('list_spell_effects'),

  /** why: spell name -> real stacking group id (only 48 entries -- see stackingdata.rs's own doc). */
  getSpellStackingGroups: () => invoke<Record<string, number>>('get_spell_stacking_groups'),

  getItemLootHistory: (item: string) => invoke<LootEventDto[]>('get_item_loot_history', { item }),

  getMobStats: (mobName: string) => invoke<MobStatsDto>('get_mob_stats', { mobName }),

  /** why: a zone page's "your parsed encounters here" section */
  listZoneEncounters: (zoneId: string, limit: number | null = null) =>
    invoke<ZoneEncounterDto[]>('list_zone_encounters', { zoneId, limit }),

  /** why: an NPC page's "your history with this mob" encounter list */
  listMobEncounters: (mobName: string, limit: number | null = null) =>
    invoke<ZoneEncounterDto[]>('list_mob_encounters', { mobName, limit }),

  /** why: one encounter's damage totals + drops, fetched only once a row expands */
  getEncounterDetail: (encounterId: number) =>
    invoke<EncounterDetailDto | null>('get_encounter_detail', { encounterId }),

  // ----------------------------------------------------------------- endgame

  /** why: the Raiding tab's whole data source -- curated rows of raids, each with a boss + minibosses */
  getRaids: () => invoke<RaidRowDto[]>('get_raids'),

  /** why: the "Sky - Primary Class Unlocks" tab's whole data source */
  getSkyClassUnlocks: () => invoke<SkyClassUnlockDto[]>('get_sky_class_unlocks'),

  /** why: the "Sky - Quests" tab's whole data source */
  getSkyQuests: () => invoke<SkyClassDto[]>('get_sky_quests'),

  // ------------------------------------------------------------------ ui files

  /** why: the Spellbook builder's own file picker */
  listUiFiles: () => invoke<UiFileInfoDto[]>('list_ui_files'),
  /** why: one UI file's real content, read-only */
  getUiFile: (file: string) => invoke<ParsedUiFileDto>('get_ui_file', { file }),

  /** why: a real character's saved spell loadouts -- `file` is one of listUiFiles's own
   * "hotbuttons"-kind entries, the non-UI_-prefixed one (that's where loadouts live) */
  loadSpellbookFile: (file: string) => invoke<SpellbookFileDto>('load_spellbook_file', { file }),

  /** why: writes back exactly what loadSpellbookFile returned (after edits) -- a real write
   * to a real game file, backed up first (see spellbookfiles.rs's own doc) */
  saveSpellbookFile: (file: string, loadouts: SpellLoadoutDto[]) =>
    invoke<void>('save_spellbook_file', { file, loadouts }),

  /** why: forks sourceFile's pair (hotbuttons + its UI_ layout counterpart) under a new
   * <Character>_<Zone> stem, leaving sourceFile untouched -- returns the new hotbuttons filename */
  saveSpellbookFileAs: (sourceFile: string, newStem: string, loadouts: SpellLoadoutDto[]) =>
    invoke<string>('save_spellbook_file_as', { sourceFile, newStem, loadouts }),

  /** why: resolves a catalog spell name to its real numeric id, for placing it into a loadout
   * slot -- null means spells_us.txt has no exact-name entry (real, ~7% of the catalog) */
  resolveSpellbookSpellId: (name: string) => invoke<number | null>('resolve_spellbook_spell_id', { name }),

  // -------------------------------------------------------------- preferences

  getEraOptions: () => invoke<EraOptionsDto>('get_era_options'),

  getPreferences: () => invoke<PreferencesDto>('get_preferences'),

  setPreferences: (prefs: PreferencesDto) => invoke<PreferencesDto>('set_preferences', { prefs }),

  // -------------------------------------------------------------- updater

  /** why: null means no update found -- not an error, the common case.
   * Checks whichever channel Preferences.update_channel currently says. */
  checkForUpdate: () => invoke<UpdateInfoDto | null>('check_for_update'),

  /** why: installs whatever the last checkForUpdate call found, then
   * restarts the app -- this call does not resolve on success (the
   * process exits first), only on failure. */
  installPendingUpdate: () => invoke<void>('install_pending_update'),

  // -------------------------------------------------------------- history

  getMobHistory: (target: string, confirmedOnly: boolean) =>
    invoke<ParseRecordDto[]>('get_mob_history', { target, confirmedOnly }),

  getLoadoutSummary: (target: string, confirmedOnly: boolean) =>
    invoke<LoadoutSummaryDto[]>('get_loadout_summary', { target, confirmedOnly }),

  // -------------------------------------------------------------- debug

  listDebugEncounters: (limit: number | null = null) =>
    invoke<DebugEncounterDto[]>('list_debug_encounters', { limit }),

  getUnmatchedCoverage: (top: number | null = null) =>
    invoke<UnmatchedCoverageDto>('get_unmatched_coverage', { top }),
};

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
  class: 'wizard' | 'druid';
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
  /** why: real map-file shortname(s) for `current`, from the wiki's own
   * scraped who_name field -- what "is the map I have open actually my
   * current zone" checks membership in now, replacing a substring guess
   * that silently failed for most real zones (their internal map
   * shortname bears no resemblance to the display name at all -- e.g.
   * "gukbottom" for "The Ruins of Old Guk"). Empty when unresolvable. */
  current_map_zones: string[];
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

  // -------------------------------------------------------------- preferences

  getEraOptions: () => invoke<EraOptionsDto>('get_era_options'),

  getPreferences: () => invoke<PreferencesDto>('get_preferences'),

  setPreferences: (prefs: PreferencesDto) => invoke<PreferencesDto>('set_preferences', { prefs }),

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

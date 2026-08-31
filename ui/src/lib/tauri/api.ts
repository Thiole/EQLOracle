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
  /** Healing landed on the target during the selection. */
  enemy_heal: number;
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
  /** null when this ally never threw a melee-avoidable swing. */
  hit_pct: number | null;
  /** null when this ally never cast a resistable spell. */
  resist_pct: number | null;
  /** why: portion of `total` that arrived via this ally's own pet(s) --
   * possessive-named pets fold into their owner's row now, this is how
   * much of it was theirs. 0 when none. */
  pet_total: number;
  /** why: a SUGGESTED ally (charm pet or co-occurrence), not a proven
   * one -- rendered visibly tentative, see combat.rs AllyDto's own doc */
  suggested: boolean;
}

/** why: one source+ability line of a death recap -- see deathrecap.rs */
export interface RecapRowDto {
  source: string;
  ability: string;
  total: number;
  hits: number;
  max_hit: number;
  avoided: number;
}

export interface KillingBlowDto {
  source: string;
  ability: string;
  amount: number;
  ts_ms: number;
}

export interface DeathRecapDto {
  death_ts_ms: number;
  window_ms: number;
  killing_blow: KillingBlowDto | null;
  incoming: RecapRowDto[];
  heals: RecapRowDto[];
  total_incoming: number;
  total_healed: number;
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

export interface RecentEffectDto {
  /** Who caused it, if attribute_effect found exactly one real recent
   * caster to explain it -- null means genuinely unresolved (0 or 2+ real
   * candidates), not "you"/unknown as a guess. */
  source: string | null;
  /** The real spell name explaining `text`, independent of whether
   * `source` also resolved. */
  skill: string | null;
  /** The raw landing/wears-off/state text, straight off the log line. */
  text: string;
}

export interface EntityStateDto {
  name: string;
  is_player: boolean;
  is_pet: boolean;
  is_enemy: boolean;
  state: string;
  observed: boolean;
  dps: number;
  /** Recognized buff/effect landings, recent as of this instant --
   * recency, not a live "still active" claim (the log has no wears-off
   * line for most of these), each with best-effort source/skill
   * attribution -- see RecentEffectDto's own doc. */
  recent_effects: RecentEffectDto[];
}

// ------------------------------------------------------------------ overlay

/** why: window role is a negotiated capability, never assumed -- see windowcap.rs's own doc */
export type WindowCapability = 'docked' | 'floating' | 'click_through';

export interface WindowCapabilityDto {
  capability: WindowCapability;
  /** why: plain-language, shown directly -- set only when capped below click_through */
  reason: string | null;
}

export interface LiveMeterDto {
  target: string;
  open: boolean;
  /** why: players and assumed pets, ranked by their own trailing dps */
  outgoing: EntityStateDto[];
  /** why: the enemy side -- same trailing dps calc, so this is real incoming damage per source */
  incoming: EntityStateDto[];
}

export interface CharmStatusDto {
  who: string;
  active: boolean;
  since_ms: number;
}

export interface InvisStatusDto {
  active: boolean;
  /** true = still invisible, but about to end -- the early-warning line landed */
  fading: boolean;
  since_ms: number;
}

export interface MomentaryStatusDto {
  /** why: 'uncertain' is root/fear only -- an enemy death that MIGHT
   * have been the caster's (same-named mobs are ambiguous); resolved by
   * the effect's own wear-off line, a fresh landing, or every enemy dying */
  outcome: 'success' | 'failure' | 'ended' | 'uncertain';
  since_ms: number;
}

/** why: Charm/Invisibility/Hide/Sneak/CC (Stun/Root/Fear) -- see
 * effects.rs's own doc. Each field null when nothing of that kind has
 * happened yet this session. CC fields carry 'success' (landed/on),
 * 'ended' (off), or -- root/fear only -- 'uncertain' (a possible-caster
 * death, see MomentaryStatusDto); no 'failure' case exists for them. */
export interface StatusEffectsDto {
  charm: CharmStatusDto | null;
  invis: InvisStatusDto | null;
  hide: MomentaryStatusDto | null;
  sneak: MomentaryStatusDto | null;
  stun: MomentaryStatusDto | null;
  root: MomentaryStatusDto | null;
  fear: MomentaryStatusDto | null;
  /** why: the generic "You lose control of yourself!" landing -- fear,
   * charm-on-you, or captivate; see the pack's state.you_lose_control doc */
  control: ControlStatusDto | null;
}

/** why: MomentaryStatusDto plus the probable enemy caster/spell -- mob
 * casts name their spells, so the Ctrl square can say what took you */
export interface ControlStatusDto {
  outcome: 'success' | 'failure' | 'ended' | 'uncertain';
  since_ms: number;
  caster: string | null;
  spell: string | null;
}

/** why: Skill Tracker's own-cooldowns section -- see skilltracker.rs's own doc */
export interface SkillStatusDto {
  skill: string;
  last_outcome: 'landed' | 'avoided';
  last_used_ms: number;
  /** why: already resolved as max(reuse, recovery) server-side -- a real
   * absolute deadline, not a relative duration that would go stale
   * between polls (same shape as TargetEffectDto's own ready_at_ms).
   * null only when there's no data to estimate from at all yet. */
  ready_at_ms: number | null;
  /** why: the raw learned interval behind ready_at_ms -- see
   * skilltracker.rs's own doc on why the smallest observed real gap is
   * this server's only trustworthy reuse-timer source, and why that's
   * already AA/haste/gear-upgrade-aware without modeling any of them
   * separately: it's measured off the player's own real casts. null
   * until a second real attempt exists to measure a gap from. */
  reuse_gap_ms: number | null;
  /** why: the recovery-anchor counterpart -- landing-to-next-attempt,
   * tracked independently of reuse_gap_ms (see SkillTrack::ready_at's
   * own doc). null until a landing AND a later attempt both exist. */
  recovery_gap_ms: number | null;
}

/** why: Skill Tracker's target-effects section -- see targeteffects.rs's own doc */
export interface TargetEffectDto {
  spell: string;
  /** why: the wiki scrape's own icon filename -- real assets are bundled
   * at /planner/icons (see character/constants.ts's own ICON_BASE), null
   * for an unrecognized spell name */
  icon: string | null;
  /** why: false when the most recent real observation was a resisted cast, not a landing */
  landed: boolean;
  since_ms: number;
  /** why: null for a failed cast, or a landed effect with no known real duration */
  duration_ms: number | null;
  ready_at_ms: number | null;
}

export interface TargetEffectsDto {
  /** why: null when there's no live enemy target to report against */
  target: string | null;
  effects: TargetEffectDto[];
}

export interface DropWatchRowDto {
  mob: string;
  /** why: this mob's full known drop list, unfiltered -- intersect with
   * tracked_drop_items client-side, same split get_skill_status uses */
  drops: string[];
}

export interface TrackedLootDto {
  item: string;
  count: number;
  last_looted_ms: number;
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

/** why: Overview module's own session-scoped rate stats -- see overview.rs's own doc */
export interface SessionDto {
  afk: boolean;
  /** why: null only before a single line has been parsed at all */
  session_start_ms: number | null;
  session_duration_ms: number;
  /** why: null below overview.rs's own MIN_SESSION_MS_FOR_RATE */
  platinum_per_hour: number | null;
  xp_pct_per_hour: number | null;
  /** why: null means no level.up line yet, not "level unknown" */
  current_level: number | null;
  /** why: summed Xp since the last level.up -- doesn't reset on AFK, only on ding */
  progress_pct: number | null;
  /** why: null if either half unavailable, or rate is 0 (would be infinity) */
  eta_hours: number | null;
  /** why: every "Mote of <tier> Potential" tier summed together */
  motes_found: number;
  /** why: null below overview.rs's own MIN_SESSION_MS_FOR_RATE */
  motes_per_hour: number | null;
  /** why: null when the level *at session start* was never itself
   * confirmed by a real level.up line -- never guessed as 0 */
  levels_gained: number | null;
  /** why: real AA cost sum since session start, not a rate -- AA grants
   * are too bursty/rare for a per-hour number to mean anything */
  aa_spent: number;
  /** why: per-tier breakdown of motes_found, only tiers seen this
   * session, ascending by tier */
  mote_tiers: MoteTierDto[];
}

/** why: `tier` is a derived ordinal (ascending by the tier *names'* own
 * English magnitude), not a wiki-confirmed number -- the scrape has no
 * real tier field for Motes at all. Use `name` for display. `tier` is
 * null for a real "Mote of X" loot whose name isn't one of the 9 known
 * tiers (e.g. the wiki's own bare "Mote of Potential") -- still counted,
 * just not ranked; `name` stays unique either way, so key lists on it,
 * not `tier`. */
export interface MoteTierDto {
  tier: number | null;
  name: string;
  count: number;
}

/** why: Game Data's own top-of-page disclaimer */
export interface GameDataMetaDto {
  source: string;
  /** why: null if the scrape never recorded one -- shown as "unknown", never guessed */
  scraped: string | null;
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

// ---------------------------------------------------------------- tradeskills

export interface RecipeIngredientDto {
  item: string;
  qty: number;
  /** why: a container/tool that's consumed but handed back either way (e.g. a Pie Tin) */
  returned: boolean;
}

/** why: real wiki recipe data -- see tradeskilldata.rs's own doc for the
 * real table-shape quirks this survived, and what's still not captured
 * (a long tail of one-off armor-material sub-tables) */
export interface RecipeDto {
  item: string;
  /** why: some output cells carry their own yield prefix ("2x [[Item]]") */
  yield_qty: number;
  ingredients: RecipeIngredientDto[];
  implements: string | null;
  yield: string | null;
  /** why: null when the wiki's own value isn't a plain integer -- see trivial_raw */
  trivial: number | null;
  trivial_raw: string | null;
  use: string | null;
}

export interface TradeskillSkillDto {
  skill: string;
  recipes: RecipeDto[];
}

/** why: real craft attempts this file has ever recorded, joined against
 * the catalog above by output item name -- see craftlog.rs's own doc */
export interface CraftLogEntryDto {
  item: string;
  /** why: null when this item isn't a known recipe output anywhere in the catalog */
  tradeskill: string | null;
  trivial: number | null;
  attempts: number;
  successes: number;
  failures: number;
  /** why: true if any attempt at this item ever hit the skill cap */
  skill_capped: boolean;
}

/** why: one final reward item -- what "Primary Class Unlocks" tracks, never the raw
 * materials a quest is built from */
export interface SkyRewardDto {
  name: string;
  /** why: which quest earns this reward -- context only */
  quest: string;
  ever_looted: boolean;
  looted_count: number;
  currently_owned: number | null;
  sold_without_keeping: boolean;
  completed: boolean | null;
  /** why: rune first, then drop items, in quest order, with full ownership status --
   * same shape the Quests tab's own chips render */
  materials: TurnInItemDto[];
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

/** why: one real copy's own resting place -- "where is my X", GdLink's own locate affordance */
export interface ItemLocationDto {
  label: string;
  tier: number;
  count: number;
}

/** why: one real item sitting in a storage container -- the Inventory tab's own per-row payload */
export interface InventorySlotDto {
  slot: string;
  item: string;
  tier: number;
  count: number;
}

/** why: one real storage container (a bag, the bank, the depot, key ring, ...) */
export interface InventoryContainerDto {
  label: string;
  bag_item: string | null;
  slots: InventorySlotDto[];
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
export interface ZoneNpcDto {
  name: string;
  /** raw wiki string -- "37-39" is a real shape; con math is ours */
  level: string | null;
  race: string | null;
  class: string | null;
  drops: string[];
  has_markers: boolean;
}

export interface PathDto {
  waypoints: [number, number, number][];
  /** why: which engine routed -- 'navmesh' (EQEmu Detour mesh, true
   * walkable surfaces) or 'lines' (grid A* over map wall geometry, the
   * fallback while a zone's mesh isn't cached). */
  source: 'navmesh' | 'lines';
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

/** why: one spawn point for "navigate to this NPC" -- zone is the raw wiki
 * value (what the map's npc-overlay bridge matches), route_zone the
 * zonedata name find_zone_route accepts, null when unroutable. */
export interface NpcNavPointDto {
  zone: string;
  route_zone: string | null;
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

export interface LootRowDto {
  item: string;
  /** why: stack sizes summed in, not a line count; 0 for a not-yet-gotten known drop */
  count: number;
}

export interface MobDto {
  name: string;
  /** why: confirmed death lines only -- a Reset isn't evidence either way */
  kills: number;
  /** why: every encounter, kills and resets alike */
  pulls: number;
  /** why: whether `monsterdata` recognizes this mob -- whether `loot` is
   * the complete wiki-known list or just what's actually been looted */
  known: boolean;
  /** why: gotten-first by count, then alphabetically */
  loot: LootRowDto[];
  /** why: mean over kills with a matched Xp row; null if none do */
  avg_xp_pct: number | null;
}

// -------------------------------------------------------------------- chat

export interface ChatMessageDto {
  ts_ms: number;
  /** why: the real sender -- "You" for the player's own outgoing line */
  who: string;
  text: string;
}

export interface PmThreadDto {
  /** why: the other side of the conversation, regardless of who sent the most recent line */
  player: string;
  last_ts_ms: number;
  last_text: string;
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
  /** why: a slug matching one of themes.css's own `[data-theme="X"]`
   * blocks -- 'eqlp' is this app's own original identity, everything
   * else is a real preset, see themes.css's own doc for where they're from */
  theme: string;
  /** why: each overlay widget owns its own opacity, not one shared
   * window-wide value -- 0.0 (invisible) to 1.0 (fully opaque), this
   * widget's own panel background alpha. NOT the same as
   * overlay_dps_meter_overall_opacity below -- this one never touches
   * text/icons, just the panel behind them. */
  overlay_dps_meter_opacity: number;
  /** why: the SEPARATE "everything" fade -- a real CSS opacity on the
   * widget's whole outer element, so text/icons fade right along with
   * the panel instead of staying fully readable no matter how
   * see-through the background is. 1.0 (fully opaque) by default. */
  overlay_dps_meter_overall_opacity: number;
  /** why: same pattern as overlay_dps_meter_opacity -- covers all three
   * of the Skill Tracker's own sections (status effects, cooldowns,
   * target effects), one window, one panel, one alpha */
  overlay_skill_tracker_opacity: number;
  /** why: see overlay_dps_meter_overall_opacity's own doc -- same
   * "everything" fade, this widget's own */
  overlay_skill_tracker_overall_opacity: number;
  /** why: any ability/spell name the player has "track"ed for the
   * cooldowns section (Combat's ability rows, or the Skill Tracker's
   * own settings card) -- not a fixed list, empty until the user
   * tracks something. Not per-target -- see tracked_target_effects. */
  tracked_skills: string[];
  /** why: a separate list from tracked_skills -- a spell added here
   * (Spellbook's own "Overlay spell tracking" section, the only real
   * entry point) shows up ONLY in the target-effects section (landed?
   * how much duration left?), never its own cooldown/READY row. Empty
   * by default -- nothing baked in here, unlike tracked_skills' 4
   * status pseudo-entries. */
  tracked_target_effects: string[];
  /** why: same pattern as overlay_dps_meter_opacity -- see dropwatch.rs's
   * own doc for what this widget shows */
  overlay_drop_watch_opacity: number;
  /** why: see overlay_dps_meter_overall_opacity's own doc -- same
   * "everything" fade, this widget's own */
  overlay_drop_watch_overall_opacity: number;
  /** why: same pattern as overlay_dps_meter_opacity -- CC Tracker's own
   * widget (Root/Stun/Fear squares), see CCTrackerWidget.svelte's own doc */
  overlay_cc_tracker_opacity: number;
  /** why: see overlay_dps_meter_overall_opacity's own doc -- same
   * "everything" fade, this widget's own */
  overlay_cc_tracker_overall_opacity: number;
  /** why: 'small' | 'medium' | 'large' -- see ccSize.ts's own doc. A
   * plain string, not a union, same "unrecognized value just falls back"
   * contract as `theme` above -- ccSize.ts's asCcSize() is what actually
   * validates it on read. */
  overlay_cc_tracker_size: string;
  /** why: item names the player wants a heads-up on when currently
   * fighting a mob known to drop one -- entry points are Sky Quests'
   * material chips and Primary Class Unlocks' reward materials. Empty by
   * default -- nothing baked in. */
  tracked_drop_items: string[];
  /** why: baseline count already prompted-about (or auto-dismissed) per
   * tracked item -- a fresh loot past this count is a new prompt, an
   * already-accounted-for one isn't. Persisted so a restart doesn't
   * re-prompt about the same old loot line. */
  tracked_drop_seen_counts: Record<string, number>;
  /** why: real epoch ms, refreshed roughly every 5 minutes while Drop
   * Watch has anything tracked -- see dropWatchLoot.ts's own doc. What
   * "new" means for the loot-removal prompt: after this, not just
   * within a fixed window of whenever the app happens to check. null
   * until Drop Watch has tracked anything at least once. */
  drop_watch_checkpoint_ms: number | null;
}

export interface UpdateInfoDto {
  version: string;
  current_version: string;
  /** why: the release's own body text, whatever that channel's notes say -- may be empty */
  notes: string | null;
  /** why: a real link to the GitHub release page for this channel, so the update prompt can link out to the changelog instead of just showing notes inline */
  release_url: string;
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
  /** why: false = someone else's fight -- parsed for clean data, hidden
   * from Combat/overlay; Debug is where it stays visible */
  involves_you: boolean;
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

export interface PartyMemberDto {
  name: string;
  via: 'you' | 'joined' | 'strong' | 'weak';
  sessions: number;
}

export interface GameStateDto {
  party: PartyMemberDto[];
  /** why: everyone ever proven a real player across the whole log -- a
   * permanent identity fact, deliberately a count and never party rows */
  known_players: number;
  your_classes: string[];
  your_level: number | null;
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

  /** confirmedOnly drops closed "reset" fights from an aggregate --
   * the copy-report path; on-screen views pass nothing and keep all. */
  getCombatSummary: (
    zoneVisit: number | null,
    encounterId: number | null,
    actor: string | null = null,
    confirmedOnly = false,
  ) => invoke<CombatSummaryDto>('get_combat_summary', { zoneVisit, encounterId, actor, confirmedOnly }),

  listAllies: (zoneVisit: number | null, encounterId: number | null, confirmedOnly = false) =>
    invoke<AllyDto[]>('list_allies', { zoneVisit, encounterId, confirmedOnly }),

  getFightTimeline: (encounterId: number) => invoke<FightTimelineDto | null>('get_fight_timeline', { encounterId }),

  getFightStateAt: (encounterId: number, tsMs: number) =>
    invoke<EntityStateDto[]>('get_fight_state_at', { encounterId, tsMs }),

  // -------------------------------------------------------------- character

  // `name` is always the literal "You" -- the log is first-person, so the
  // player's own actions land under that exact symbol in the store, not
  // their character's real name. See dump_fixtures.rs's own note.
  getClassConfigurations: () => invoke<ClassConfigurationsDto>('get_class_configurations', { name: 'You' }),

  // why: levelRange disambiguates which row -- more than one row can now
  // share the same classes (separate real sessions of the same trio,
  // see combat.rs's SESSION_GAP_MS), so classes alone no longer picks
  // a unique row.
  getConfigurationZoneVisits: (classes: string[], levelRange: [number, number] | null) =>
    invoke<ZoneVisitDto[]>('get_configuration_zone_visits', { name: 'You', classes, levelRange }),

  getCurrentLevel: () => invoke<number | null>('get_current_level'),

  getDefaultGearClasses: () => invoke<string[]>('get_default_gear_classes', { name: 'You' }),

  /** why: Overview's plat/hr, xp%/hr, current level + progress, ETA to next ding */
  getSession: () => invoke<SessionDto>('get_session'),

  /** why: Overview Session card's own "restart" button -- see Ingest::reset_session's own doc */
  resetSession: () => invoke<SessionDto>('reset_session'),

  /** why: Game Data's own disclaimer banner -- source + last scraped date */
  getGameDataMeta: () => invoke<GameDataMetaDto>('get_game_data_meta'),

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

  /** why: empty (not an error) whenever there's no dump yet -- unknown, not "not found" */
  locateItem: (name: string) => invoke<ItemLocationDto[]>('locate_item', { name }),

  getInventoryBrowser: () => invoke<InventoryContainerDto[]>('get_inventory_browser'),

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
  /** why: fetches a zone's EQEmu nav+collision files into the app-data
   * cache -- fire-and-forget on zone open; pathfinding upgrades itself
   * once cached. */
  ensureEmuZone: (zone: string) => invoke<{ nav: boolean; geo: boolean }>('ensure_emu_zone', { zone }),
  /** why: the Maps left panel's NPC browser -- catalog NPCs whose wiki
   * zone matches the open map, with drops for the selection expansion */
  listZoneNpcs: (mapZoneName: string) => invoke<ZoneNpcDto[]>('list_zone_npcs', { mapZoneName }),
  /** why: a real cross-zone route, weighted by real in-zone walking distance -- see ZoneRouteDto's own doc */
  findZoneRoute: (fromZone: string, toZone: string) => invoke<ZoneRouteDto>('find_zone_route', { fromZone, toZone }),
  /** why: the most recent "/loc" reading this session, if any -- a snapshot, not live tracking */
  getLastLocation: () => invoke<LastLocationDto | null>('get_last_location'),
  /** why: current + previous zone labels, for the entrance guess when no real /loc exists yet */
  getZoneContext: () => invoke<ZoneContextDto>('get_zone_context'),
  /** why: fuzzy candidates only -- see npcdata::candidate_zones' own doc for why this can't be exact */
  listNpcZoneCandidates: (mapZoneName: string) => invoke<string[]>('list_npc_zone_candidates', { mapZoneName }),
  getNpcMarkersForZone: (zone: string) => invoke<NpcMarkerDto[]>('get_npc_markers_for_zone', { zone }),
  getNpcNavPoints: (name: string) => invoke<NpcNavPointDto[]>('get_npc_nav_points', { name }),

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

  /** why: every mob type fought this session, grouped -- the Loot History tab's whole data source */
  listMobs: () => invoke<MobDto[]>('list_mobs'),

  /** why: Overlay tab's own runtime capability check -- see windowcap.rs's own doc */
  getWindowCapability: () => invoke<WindowCapabilityDto>('get_window_capability'),
  /** why: whether the main window is frameless with the in-app title
   * bar (Windows only) -- a backend platform fact, see get_ui_shell's
   * own doc on the Linux drag-region limitation. */
  getUiShell: () => invoke<{ custom_titlebar: boolean }>('get_ui_shell'),
  /** why: Character Planner persistence -- hand-set race + ONLY the
   * user-typed levels (presence = the "user updated" flag). Its own
   * commands, deliberately outside PreferencesDto -- see backend
   * set_preferences' doc on clobber-proofing. */
  getPlannerState: () => invoke<{ race: string | null; levels: Record<string, number> }>('get_planner_state'),
  setPlannerState: (race: string | null, levels: Record<string, number>) =>
    invoke<void>('set_planner_state', { race, levels }),
  /** why: the DPS meter overlay's whole data source */
  getLiveMeter: () => invoke<LiveMeterDto | null>('get_live_meter'),
  /** why: the timed-effects overlay's whole data source */
  getStatusEffects: () => invoke<StatusEffectsDto>('get_status_effects'),
  /** why: the Skill Tracker's own-cooldowns section data source */
  getSkillStatus: () => invoke<SkillStatusDto[]>('get_skill_status'),
  /** why: the Skill Tracker's target-effects section data source */
  getTargetEffects: () => invoke<TargetEffectsDto>('get_target_effects'),
  /** why: Drop Watch widget -- see dropwatch.rs's own doc. Unfiltered,
   * frontend intersects with tracked_drop_items same as skill status. */
  getDropWatch: () => invoke<DropWatchRowDto[]>('get_drop_watch'),
  /** why: Drop Watch's "you just got one, remove it?" prompt -- see
   * TrackedLootDto's own doc. One call covers every currently-tracked name. */
  getTrackedLootStatus: (items: string[]) => invoke<TrackedLootDto[]>('get_tracked_loot_status', { items }),
  /** why: "why did I just die" -- recap (null before any death) plus
   * every death timestamp this session, one call. See deathrecap.rs. */
  getDeathRecap: (deathTs: number | null = null) =>
    invoke<[DeathRecapDto | null, number[]]>('get_death_recap', { deathTs }),
  /** why: each widget is its own real OS window -- opens/closes just that
   * one; rejects with a plain-language reason if the session's
   * capability caps below click-through */
  setOverlayEnabled: (widget: string, enabled: boolean) => invoke<void>('set_overlay_enabled', { widget, enabled }),
  /** why: live-pushes to that widget's own open window only -- pair with setPreferences to persist */
  setOverlayOpacity: (widget: string, opacity: number) => invoke<void>('set_overlay_opacity', { widget, opacity }),
  /** why: the SEPARATE "everything" fade -- same live-push/persist split as setOverlayOpacity above */
  setOverlayOverallOpacity: (widget: string, opacity: number) =>
    invoke<void>('set_overlay_overall_opacity', { widget, opacity }),
  /** why: same live-push/persist split as setOverlayOpacity above, but
   * resizes the real OS window instead of a CSS value -- only CC Tracker
   * uses this today, shaped generically (a `widget` param, same as every
   * other overlay setting here) so the next widget with a size preset
   * doesn't need a new command. See ccSize.ts's own doc. */
  setOverlaySize: (widget: string, size: string) => invoke<void>('set_overlay_size', { widget, size }),
  /** why: brings that widget's own window to front and tells it to flash --
   * see commands::locate_overlay's own doc. No-op if it isn't open. */
  locateOverlay: (widget: string) => invoke<void>('locate_overlay', { widget }),
  /** why: unlock to drag that widget's own window into position, lock to make it click-through again */
  setOverlayLocked: (widget: string, locked: boolean) => invoke<void>('set_overlay_locked', { widget, locked }),

  /** why: Social tab's 3 shared channels */
  getGuildChat: () => invoke<ChatMessageDto[]>('get_guild_chat'),
  getPartyChat: () => invoke<ChatMessageDto[]>('get_party_chat'),
  getRaidChat: () => invoke<ChatMessageDto[]>('get_raid_chat'),

  /** why: Social tab's PM player list, most-recent-message first */
  listPmThreads: () => invoke<PmThreadDto[]>('list_pm_threads'),
  /** why: one PM thread's whole history, oldest first */
  getPmHistory: (player: string) => invoke<ChatMessageDto[]>('get_pm_history', { player }),

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

  // ------------------------------------------------------------------ tradeskills

  /** why: static recipe catalog, every core tradeskill -- no Ingest needed */
  getTradeskillCatalog: () => invoke<TradeskillSkillDto[]>('get_tradeskill_catalog'),

  /** why: real craft attempts this file has recorded, joined against the catalog above */
  getCraftLog: () => invoke<CraftLogEntryDto[]>('get_craft_log'),

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

  /** why: resolves a batch of catalog spell names to their real numeric ids in one call --
   * null per entry means spells_us.txt has no exact-name entry (real, ~7% of the catalog).
   * Batched on purpose: one spells_us.txt parse for the whole request, not one per name. */
  resolveSpellbookSpellIds: (names: string[]) => invoke<(number | null)[]>('resolve_spellbook_spell_ids', { names }),

  // -------------------------------------------------------------- preferences

  getEraOptions: () => invoke<EraOptionsDto>('get_era_options'),

  getPreferences: () => invoke<PreferencesDto>('get_preferences'),

  setPreferences: (prefs: PreferencesDto) => invoke<PreferencesDto>('set_preferences', { prefs }),

  // -------------------------------------------------------------- updater

  /** why: null means no update found -- not an error, the common case.
   * Checks whichever channel Preferences.update_channel currently says. */
  checkForUpdate: () => invoke<UpdateInfoDto | null>('check_for_update'),

  /** why: the deferred second step -- restarts into an update
   * installPendingUpdate already put on disk. Never resolves. */
  restartApp: () => invoke<void>('restart_app'),

  /** why: installs whatever the last checkForUpdate call found -- swaps
   * the file on disk, emits 'update-progress' ([received, total|null])
   * while downloading, and resolves WITHOUT restarting (two-step flow;
   * restartApp is the second step). On Windows the plugin's installer
   * exits the process itself, so this never resolves there. */
  installPendingUpdate: () => invoke<void>('install_pending_update'),

  /** why: what's actually installed right now -- no network round trip,
   * unlike checkForUpdate (a real check against GitHub). Info page's
   * own "current version information" ask. */
  getAppVersion: () => invoke<string>('get_app_version'),

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

  getGameState: () => invoke<GameStateDto>('get_game_state'),
};

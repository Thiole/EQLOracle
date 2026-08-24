// why: single source of truth for Character's 4 subpages, in-memory only
import { writable, get } from 'svelte/store';
import {
  api,
  type ClassConfigurationsDto,
  type AaLogDto,
  type AaDto,
  type SpellbookEntryDto,
  type CharacterEstimateDto,
  type SlotRecommendationDto,
  type ScoredItemDto,
  type InventoryDumpDto,
  type DamageSpellDto,
} from '../tauri/api';
import { ALL_CLASSES, MAX_ACTIVE_CLASSES, MAX_CHARACTER_LEVEL } from '../character/constants';
import { effectiveEra } from './settings';

export const race = writable<string>('');
/** Up to 3 full class names, selection order -- the active trio. */
export const activeClasses = writable<string[]>([]);
/** Class name -> level (1-50), for all 16, not just the active trio. */
export const levels = writable<Record<string, number>>({});

export const classConfigurations = writable<ClassConfigurationsDto | null>(null);
export const defaultClasses = writable<string[]>([]);
export const currentLevel = writable<number | null>(null);
export const estimate = writable<CharacterEstimateDto | null>(null);

export const aaLog = writable<AaLogDto | null>(null);
export const aaCatalog = writable<AaDto[]>([]);
export const spellbook = writable<SpellbookEntryDto[]>([]);
/** Highest live rank observed cast this session, by catalog base spell name -- e.g. `{ "Ice Comet": 10 }`. No entry = never cast this session, not rank 0. */
export const spellRanks = writable<Record<string, number>>({});
/** why: every damage-capable spell, rank-adjusted to this session's real
 * observed ranks -- the shared source both DpsSuggest and the Spellbook
 * builder's "Suggest Combat" button rank against, so the two never disagree. */
export const damageSpells = writable<DamageSpellDto[]>([]);

export const gearRecommendations = writable<SlotRecommendationDto[] | null>(null);
export const gearWeights = writable<Record<string, number>>({});
/** why: manual per-slot picks, overriding the top recommendation -- lives
 * here (not local GearPanel state) so refreshEstimate's gear totals see
 * the exact same "what's worn" GearPanel's own doll shows. */
export const gearChosen = writable<Record<string, string>>({});

/** why: equipped item as ScoredItemDto; score NaN, nothing to rank -- shared
 * by GearPanel's doll and gearStatTotals below, so they can never disagree
 * about what's equipped in a slot. */
export function equippedGearItem(key: string): ScoredItemDto | undefined {
  const it = get(equippedInventory)?.resolved[key];
  return it ? { ...it, score: NaN } : undefined;
}

/** why: equipped beats a manual pick beats the top recommendation -- the
 * doll's own priority order, shared so gearStatTotals matches it exactly. */
export function chosenGearItem(key: string, items: ScoredItemDto[]): ScoredItemDto | undefined {
  const equipped = equippedGearItem(key);
  if (equipped) return equipped;
  const id = get(gearChosen)[key];
  return (id && items.find((it) => it.id === id)) || items[0];
}

export function isTwoHandItem(item: ScoredItemDto | undefined): boolean {
  return !!item?.skill?.startsWith('2H');
}

/** why: the Character sheet's own "Gear" column -- sums whatever's
 * actually worn per slot (same priority chosenGearItem uses everywhere
 * else), so it can never silently disagree with what GearPanel's doll
 * shows. A 2H Primary occupies Secondary too, so Secondary is skipped
 * rather than double-counted -- same rule the doll itself uses. */
function gearStatTotals(): Record<string, number> {
  const totals: Record<string, number> = {};
  const recs = get(gearRecommendations);
  if (!recs) return totals;
  const bySlot = new Map(recs.map((r) => [r.slot, r.items]));
  const primary = chosenGearItem('PRIMARY', bySlot.get('PRIMARY') ?? []);
  const primaryIsTwoHand = isTwoHandItem(primary);
  for (const [key, items] of bySlot) {
    if (key === 'SECONDARY' && primaryIsTwoHand) continue;
    const item = chosenGearItem(key, items);
    if (!item) continue;
    for (const [stat, val] of Object.entries(item.stats)) {
      totals[stat] = (totals[stat] ?? 0) + val;
    }
  }
  return totals;
}

// ---------------------------------------------------------------- equipped inventory

/** why: dump ready, not yet loaded; stays until user acts */
export const pendingInventoryDump = writable<{ file: string; character: string | null } | null>(null);
/** why: doll priority: equipped beats manual pick beats top rec */
export const equippedInventory = writable<InventoryDumpDto | null>(null);
export const inventoryDumpError = writable<string | null>(null);
/** why: bumped only on a real fresh dump load -- NOT on clearEquippedSlot's
 * own in-place edits to the same dump, which GearPanel must not mistake
 * for a new dump and use as a reason to wipe the pick it's mid-committing. */
export const inventoryDumpVersion = writable(0);

export function onInventoryDumpDetected(file: string, character: string | null) {
  pendingInventoryDump.set({ file, character });
}

export function dismissInventoryDump() {
  pendingInventoryDump.set(null);
}

export async function loadInventoryDump() {
  const pending = get(pendingInventoryDump);
  if (!pending) return;
  pendingInventoryDump.set(null);
  inventoryDumpError.set(null);
  try {
    equippedInventory.set(await api.getInventoryDump(pending.file));
    inventoryDumpVersion.update((v) => v + 1);
    void refreshGear();
  } catch (e) {
    inventoryDumpError.set(String(e));
  }
}

/** why: manual pick must override equipped for just this slot */
export function clearEquippedSlot(key: string) {
  equippedInventory.update((dump) => {
    if (!dump || !(key in dump.resolved)) return dump;
    const resolved = { ...dump.resolved };
    delete resolved[key];
    return { ...dump, resolved };
  });
}

let checkedForExistingDump = false;

/** why: an already-on-disk dump, not just one caught live -- only worth asking once a session */
async function checkForExistingInventoryDump() {
  if (checkedForExistingDump || get(pendingInventoryDump) || get(equippedInventory)) return;
  checkedForExistingDump = true;
  const found = await api.findExistingInventoryDump();
  if (found) onInventoryDumpDetected(found.file, found.character);
}

/** why: loaded once on entering Character; input: none; output: void */
export async function loadCharacterModule() {
  const [cfgs, defaults, lvl, aa, catalog, book, ranks, dmg] = await Promise.all([
    api.getClassConfigurations(),
    api.getDefaultGearClasses(),
    api.getCurrentLevel(),
    api.getAaLog(),
    api.listAa(),
    api.getSpellbook(),
    api.getSpellRanks(),
    api.getDamageSpells(false),
  ]);
  classConfigurations.set(cfgs);
  defaultClasses.set(defaults);
  currentLevel.set(lvl);
  aaLog.set(aa);
  aaCatalog.set(catalog);
  spellbook.set(book);
  damageSpells.set(dmg ?? []);
  spellRanks.set(ranks);

  let dirty = false;
  // why: gear shouldn't sit blank waiting for a manual class pick every time
  if (!get(activeClasses).length && defaults.length) {
    activeClasses.set(defaults);
    dirty = true;
  }
  // why: levels shouldn't sit at the 1-50 default guess every launch when the log already knows better
  if (!Object.keys(get(levels)).length) {
    applyEstimatedLevels();
    dirty = true;
  }
  if (dirty) {
    void refreshEstimate();
    void refreshGear();
  }

  void checkForExistingInventoryDump();
}

export function setRace(r: string) {
  race.set(r);
  void refreshEstimate();
  void refreshGear();
}

export function toggleActiveClass(className: string) {
  const current = get(activeClasses);
  if (current.includes(className)) {
    activeClasses.set(current.filter((c) => c !== className));
  } else if (current.length < MAX_ACTIVE_CLASSES) {
    activeClasses.set([...current, className]);
  } else {
    return;
  }
  void refreshEstimate();
  void refreshGear();
}

export function setLevel(className: string, level: number) {
  const clamped = Math.min(MAX_CHARACTER_LEVEL, Math.max(1, Math.round(level) || 1));
  levels.update((l) => ({ ...l, [className]: clamped }));
  if (get(activeClasses).includes(className)) {
    void refreshEstimate();
    void refreshGear();
  }
}

/** why: the actual computation, split out so loadCharacterModule's own
 * auto-run on launch doesn't have to double up estimateLevelsFromLog's
 * refresh calls alongside its own activeClasses one.
 *
 * Reverted: an earlier version of this took the max `level_range[1]`
 * across every configuration and applied it to every class in any of
 * them, on the assumption that character level is shared across
 * configurations. Real play data says that's wrong for this game -- a
 * freshly-formed configuration can start its own leveling arc back at a
 * low level independent of what other configurations of the same
 * character have reached. Each configuration's own range is the real,
 * class-scoped fact; taking each one in isolation (this version) is
 * correct, not the bug the earlier rewrite thought it was fixing. */
function applyEstimatedLevels() {
  const cfgs = get(classConfigurations);
  const next: Record<string, number> = {};
  for (const c of ALL_CLASSES) next[c] = 10;
  if (cfgs) {
    for (const cfg of cfgs.configurations) {
      if (!cfg.level_range) continue;
      const best = cfg.level_range[1];
      for (const c of cfg.classes) {
        if (best > next[c]) next[c] = best;
      }
    }
  }
  levels.set(next);
}

/** why: fills levels from confirmed log evidence; a guess, not fact --
 * the manual "Estimate levels" button's own handler. */
export function estimateLevelsFromLog() {
  applyEstimatedLevels();
  void refreshEstimate();
  void refreshGear();
}

let estimateToken = 0;

/** why: exported so GearPanel can re-trigger it directly after a manual
 * pick (gearChosen) changes what's "worn" -- refreshGear's own chained
 * call (below) covers every other gear-affecting change already. */
export async function refreshEstimate() {
  const r = get(race);
  const classes = get(activeClasses);
  if (!r || !classes.length) {
    estimate.set(null);
    return;
  }
  const token = ++estimateToken;
  const lv = get(levels);
  const classLevels = classes.map((c) => lv[c] ?? 1);
  const gear = gearStatTotals();
  const est = await api.getCharacterEstimate(r, classes, classLevels, gear);
  if (token !== estimateToken) return;
  estimate.set(est);
}

let gearToken = 0;

async function refreshGear() {
  const r = get(race);
  const classes = get(activeClasses);
  if (!classes.length) {
    gearRecommendations.set(null);
    gearWeights.set({});
    return;
  }
  const token = ++gearToken;
  const lvl = get(currentLevel);
  // why: real dump, if loaded, feeds lore-pairing + owned counts server-side
  const dump = get(equippedInventory);
  const equipped = dump
    ? Object.fromEntries(Object.entries(dump.resolved).map(([slot, it]) => [slot, it.name]))
    : null;
  const owned = dump?.owned ?? null;
  const ownedTier = dump?.owned_tier ?? null;
  const maxEra = get(effectiveEra);
  const [recs, weights] = await Promise.all([
    api.getGearRecommendations(classes, r || null, lvl, equipped, owned, ownedTier, maxEra),
    api.getGearWeights(classes, lvl),
  ]);
  if (token !== gearToken) return;
  gearRecommendations.set(recs);
  gearWeights.set(weights);
  // why: gear totals just changed (new recs, or a fresh dump's own
  // equipped items) -- the Character sheet's gear column must follow.
  void refreshEstimate();
}

// why: the Gear tab's own recommendations are era-filtered server-side
// (gearplanner::in_era via maxEra above) -- a real re-fetch whenever the
// Settings module's era preference changes, not just on the next class/
// race/level edit. Placed at module end, after every `let`/function this
// closure touches is already initialized (this store's own immediate
// first invocation, per Svelte's subscribe contract, would otherwise run
// during module evaluation, before gearToken exists).
effectiveEra.subscribe(() => {
  if (get(activeClasses).length) void refreshGear();
});

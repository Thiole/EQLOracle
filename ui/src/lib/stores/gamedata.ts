// why: single source of truth for Game Data's 5 static wiki catalogs, plus
// the cross-page navigation state every category's own links share -- a
// zone's notable NPC, an NPC's known-loot item, a spell's class list, an
// item's drop source, all need to open a *different* category's page
// without threading a callback down through however many component
// layers sit between wherever the link was clicked and the module root.
// That link isn't only ever clicked from inside Game Data itself either
// (the Gear Planner's own item preview reuses GdLink/GdZoneOrMobLink for
// its "drops in"/"from" lines) -- see gdOpenPage's own doc.
import { writable, get } from 'svelte/store';
import {
  api,
  type ZoneDto,
  type NpcDto,
  type SpellDto,
  type SpellEffectsEntryDto,
  type AaDto,
  type ItemDto,
} from '../tauri/api';
import { activeModule } from './shell';

export const zones = writable<ZoneDto[]>([]);
export const npcs = writable<NpcDto[]>([]);
export const spells = writable<SpellDto[]>([]);
/** why: keyed by spell id -- the detail page's own per-spell lookup */
export const spellEffects = writable<Record<string, SpellEffectsEntryDto>>({});
export const aas = writable<AaDto[]>([]);
export const items = writable<ItemDto[]>([]);
export const gameDataLoaded = writable(false);
/** why: log mob name -> wiki Npc name -- see mobalias.rs's own doc */
export const mobAliases = writable<Map<string, string>>(new Map());

export type GdKind = 'zone' | 'item' | 'npc' | 'aa' | 'spell';

/** why: which detail page is open, if any -- null shows the active tab's
 * own list instead. Cleared whenever the active tab itself changes
 * (GameData.svelte's own job -- this store doesn't know about tabs). */
export const gdOpen = writable<{ kind: GdKind; key: string } | null>(null);

let loading: Promise<void> | null = null;

/** why: loaded once for the app's whole life -- these are static wiki
 * scrapes, nothing here changes mid-session the way combat/character
 * data does, so there's nothing a second load would ever pick up.
 * `items` is deliberately not fetched here -- see `refreshItems`, below,
 * for why that one's era-dependent and needs to be re-fetchable. */
export function loadGameDataModule(): Promise<void> {
  if (loading) return loading;
  loading = (async () => {
    const [z, n, s, se, a, ma] = await Promise.all([
      api.listZones(),
      api.listNpcs(),
      api.listSpells(),
      api.listSpellEffects(),
      api.listAa(),
      api.getMobAliases(),
    ]);
    // defensive -- invoke<T>()'s type is an assertion, not a guarantee
    zones.set(z ?? []);
    npcs.set(n ?? []);
    spells.set(s ?? []);
    spellEffects.set(Object.fromEntries((se ?? []).map((e) => [e.id, e])));
    aas.set(a ?? []);
    // why: case-insensitive lookup key, matching mobalias.rs's own eq_ignore_ascii_case
    mobAliases.set(new Map((ma ?? []).map(([from, to]) => [from.toLowerCase(), to])));
    gameDataLoaded.set(true);
  })();
  return loading;
}

let itemsToken = 0;

/** why: unlike the other 4 catalogs, the item list is era-filtered
 * server-side (`gearplanner::in_era`, an item's own era resolution is a
 * multi-field chain a flat client-side compare can't redo) -- so it has
 * to be a real re-fetch, not a one-time load, whenever the Settings
 * module's era preference changes. Token-guarded the same way every
 * other "user changed something, an in-flight fetch might land late"
 * spot in this app is. */
export async function refreshItems(maxEra: string): Promise<void> {
  const token = ++itemsToken;
  const i = await api.listGearItems([], null, maxEra);
  if (token !== itemsToken) return;
  // why: defensive, not just a mock-mode quirk -- invoke<T>()'s type is
  // an assertion, not a guarantee; a genuinely empty/failed response
  // should leave the list empty, not crash every filter() downstream.
  items.set(i ?? []);
}

/** why: a cross-reference name only becomes a real link if that other
 * dataset actually has a match -- both scrapes are independent passes
 * over the same wiki, so one naming something the other doesn't cover is
 * real, not a bug to paper over with a dead link. Lookup order per kind
 * mirrors the legacy planner's own `gdFind` exactly. */
export function gdFind(kind: 'zone', key: string): ZoneDto | undefined;
export function gdFind(kind: 'item', key: string): ItemDto | undefined;
export function gdFind(kind: 'npc', key: string): NpcDto | undefined;
export function gdFind(kind: 'aa', key: string): AaDto | undefined;
export function gdFind(kind: 'spell', key: string): SpellDto | undefined;
/** why: a dynamic, not-yet-narrowed kind (GdLink/GdOpen's own callers,
 * which only know `kind` as a prop, not a literal) -- the union of every
 * category's own DTO, same as calling each specific overload above would
 * return for its own kind. */
export function gdFind(kind: GdKind, key: string): ZoneDto | ItemDto | NpcDto | AaDto | SpellDto | undefined;
export function gdFind(kind: GdKind, key: string) {
  const k = key.toLowerCase();
  switch (kind) {
    case 'zone':
      return get(zones).find((z) => z.name.toLowerCase() === k);
    case 'item':
      return get(items).find((it) => it.id === key) ?? get(items).find((it) => it.name.toLowerCase() === k);
    case 'npc': {
      const aliased = get(mobAliases).get(k)?.toLowerCase();
      return (
        get(npcs).find((n) => n.name.toLowerCase() === k) ??
        get(npcs).find((n) => n.id.replace(/_/g, ' ').toLowerCase() === k) ??
        (aliased ? get(npcs).find((n) => n.name.toLowerCase() === aliased) : undefined)
      );
    }
    case 'aa':
      // why: AA names aren't unique -- "Quick Evacuation" is a real,
      // separate Druid AA and Wizard AA both, confirmed in packs/aa.json
      // (the only such collision in the catalog, but a real one). `key`
      // is the composite `name::category` form GameData's own AA row
      // clicks pass; a plain name (any future cross-link that only ever
      // knows the name) still resolves, just to whichever matches first.
      return (
        get(aas).find((a) => `${a.name}::${a.category}`.toLowerCase() === k) ??
        get(aas).find((a) => a.name.toLowerCase() === k)
      );
    case 'spell':
      return get(spells).find((s) => s.id === key) ?? get(spells).find((s) => s.name.toLowerCase() === k);
  }
}

/** why: items/spells key by id (stable even if two ever shared a name);
 * everything else keys by name -- matches `gdKeyOf`. */
function gdKeyOf(kind: GdKind, entry: { id?: string; name: string; category?: string }): string {
  if ((kind === 'item' || kind === 'spell') && entry.id) return entry.id;
  if (kind === 'aa' && entry.category) return `${entry.name}::${entry.category}`;
  return entry.name;
}

/** why: opens a page by name -- what every GdLink click actually calls.
 * A silent no-op when nothing matches (the plain-text fallback a GdLink
 * itself already rendered for that case means this is never reachable
 * from a real click, but a name typed elsewhere could still miss). */
/** why: switches to the Game Data module too, not just gdOpen -- every
 * caller of this (GdLink/GdZoneOrMobLink) is a cross-reference link, and
 * those get reused outside Game Data itself (the Gear Planner's own item
 * preview), where opening a page silently behind the currently-visible
 * module would look like nothing happened at all. */
export function gdOpenPage(kind: GdKind, name: string) {
  const entry = gdFind(kind, name);
  if (!entry) return;
  gdOpen.set({ kind, key: gdKeyOf(kind, entry) });
  activeModule.set('gamedata');
}

export const GD_LABELS: Record<GdKind, string> = {
  zone: 'Zones',
  item: 'Items',
  npc: 'NPCs',
  aa: 'AAs',
  spell: 'Spells',
};

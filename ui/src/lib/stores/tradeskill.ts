// why: the Tradeskill module's own data -- static recipe catalog (all 9
// skills) plus the real craft log, loaded once and shared across the
// central hub + every per-skill tab. A client-side name->skill index
// (mirrors tradeskilldata.rs's own crafted_via) answers "is this
// ingredient itself craftable somewhere" for RecipeList's crosslinks --
// computed here, not a per-ingredient IPC round trip.
import { writable, get } from 'svelte/store';
import {
  api,
  type TradeskillSkillDto,
  type CraftLogEntryDto,
  type TradeskillLevelDto,
  type RecentCraftDto,
} from '../tauri/api';

export const tradeskillCatalog = writable<TradeskillSkillDto[]>([]);
export const craftLog = writable<CraftLogEntryDto[]>([]);
export const tradeskillLevels = writable<TradeskillLevelDto[]>([]);
export const recentCrafts = writable<RecentCraftDto[]>([]);
export const tradeskillLoaded = writable(false);

let loading: Promise<void> | null = null;

export function loadTradeskillModule(): Promise<void> {
  if (get(tradeskillLoaded)) return Promise.resolve();
  if (loading) return loading;
  // why: null-tolerant at the store boundary so every consumer's
  // .length/.map stays safe -- a null here (IPC hiccup, mock harness
  // without this fixture) crashed the whole Tradeskill module, caught
  // by tests/render/responsive.spec.ts
  loading = Promise.all([
    api.getTradeskillCatalog(),
    api.getCraftLog(),
    api.getTradeskillLevels(),
    api.getRecentCrafts(),
  ]).then(([catalog, log, levels, recent]) => {
    tradeskillCatalog.set(catalog ?? []);
    craftLog.set(log ?? []);
    tradeskillLevels.set(levels ?? []);
    recentCrafts.set(recent ?? []);
    tradeskillLoaded.set(true);
  });
  return loading;
}

/** why: Overview's own "restart"-adjacent refresh -- everything but the
 * static catalog is real session data */
export async function refreshCraftLog() {
  const [log, levels, recent] = await Promise.all([
    api.getCraftLog(),
    api.getTradeskillLevels(),
    api.getRecentCrafts(),
  ]);
  craftLog.set(log ?? []);
  tradeskillLevels.set(levels ?? []);
  recentCrafts.set(recent ?? []);
}

let byOutputName: Map<string, string> | null = null;

/** why: first skill wins on a name collision across skills (rare) --
 * either answer is a real "yes, craftable somewhere", not worth picking
 * a "correct" one. Rebuilt lazily, invalidated whenever the catalog changes. */
export function craftedVia(item: string): string | null {
  const catalog = get(tradeskillCatalog);
  if (!byOutputName) {
    byOutputName = new Map();
    for (const s of catalog) {
      for (const r of s.recipes) {
        const key = r.item.toLowerCase();
        if (!byOutputName.has(key)) byOutputName.set(key, s.skill);
      }
    }
  }
  return byOutputName.get(item.toLowerCase()) ?? null;
}

tradeskillCatalog.subscribe(() => {
  byOutputName = null;
});

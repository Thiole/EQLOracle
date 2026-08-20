// why: single source of truth for the Settings module's own preferences --
// notification volume (not yet wired to real sound playback, see
// preferences.rs's own doc) and the wiki era Game Data/Gear should filter
// to. Both are read from other modules (GameData, Character's GearPanel),
// not just Settings itself, which is why this lives here rather than as
// local component state in Settings.svelte.
import { writable, derived, get } from 'svelte/store';
import { api, type PreferencesDto } from '../tauri/api';

export const volume = writable(100);
/** why: the raw saved preference -- null means "no explicit choice yet",
 * see effectiveEra below for what that resolves to. */
export const era = writable<string | null>(null);
export const eraOptions = writable<string[]>([]);
export const currentEra = writable('Sky Era');
export const settingsLoaded = writable(false);

/** why: what every era-aware API call should actually send -- resolves
 * the "no preference saved" null to the live server's own current era,
 * once, here, instead of every caller duplicating that fallback. */
export const effectiveEra = derived([era, currentEra], ([$era, $currentEra]) => $era ?? $currentEra);

let loading: Promise<void> | null = null;

export function loadPreferences(): Promise<void> {
  if (loading) return loading;
  loading = (async () => {
    const [opts, prefs] = await Promise.all([api.getEraOptions(), api.getPreferences()]);
    eraOptions.set(opts.eras);
    currentEra.set(opts.current);
    volume.set(prefs.volume);
    era.set(prefs.era);
    settingsLoaded.set(true);
  })();
  return loading;
}

function currentPrefs(): PreferencesDto {
  return { volume: get(volume), era: get(era) };
}

export async function setVolume(v: number) {
  volume.set(v);
  await api.setPreferences({ ...currentPrefs(), volume: v }).catch(() => {});
}

export async function setEra(e: string) {
  era.set(e);
  await api.setPreferences({ ...currentPrefs(), era: e }).catch(() => {});
}

/** why: shared by every era-tagged Game Data category that carries a
 * flat `era` field (zones/NPCs/spells) -- items go through the backend's
 * own `gearplanner::in_era` via `maxEra` instead, since an item's era
 * resolution is a multi-field chain (`available_from`/`eras`/`era`), not
 * one field this could compare the same simple way. AAs carry no era
 * field at all (the scrape never tagged them) -- never filtered. */
export function passesEra(entryEra: string | null | undefined, ceiling: string, order: string[]): boolean {
  if (ceiling === 'All') return true;
  const ceilIx = order.indexOf(ceiling);
  if (ceilIx === -1) return true; // an unrecognized ceiling -- don't hide anything over it
  if (!entryEra) return true; // unresolved era -- always shown, matches gearplanner::in_era's own stance
  const ix = order.indexOf(entryEra);
  if (ix === -1) return true; // this entry's era isn't one `order` knows -- don't guess
  return ix <= ceilIx;
}

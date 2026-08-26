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
/** why: false (default) = infer everything fresh every launch, same as
 * always. true = also keep a saved per-character class profile across
 * restarts as a fallback for zone routing -- see `PreferencesDto.
 * save_profile`'s own doc. */
export const saveProfile = writable(false);
/** why: which release channel this install checks for updates against --
 * see PreferencesDto.update_channel's own doc */
export const updateChannel = writable<'public' | 'beta'>('public');
/** why: a themes.css `data-theme` slug -- see PreferencesDto.theme's own doc */
export const theme = writable('eqlp');
/** why: each overlay widget owns its own on/off -- deliberately NOT
 * loaded from or saved to preferences (see preferences.rs's own doc):
 * whether a widget is currently showing is live session state, not a
 * style choice to remember. Always starts false on a fresh launch. */
export const dpsMeterEnabled = writable(false);
/** why: this widget's own background alpha, 0.0 (invisible) to 1.0
 * (fully opaque) -- IS persisted, a real style choice worth keeping */
export const dpsMeterOpacity = writable(0.85);
/** why: same on/off contract as dpsMeterEnabled -- see its own doc */
export const statusEffectsEnabled = writable(false);
export const statusEffectsOpacity = writable(0.85);
export const settingsLoaded = writable(false);

// why: applies on every change, not just after an explicit setTheme() --
// covers the initial value loadPreferences() sets too, so the real saved
// theme is live the moment it's known rather than waiting on Settings.
// svelte to mount. Guarded for SSR/test environments with no `document`.
theme.subscribe((t) => {
  if (typeof document !== 'undefined') {
    document.documentElement.dataset.theme = t;
  }
});

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
    saveProfile.set(prefs.save_profile);
    updateChannel.set(prefs.update_channel);
    theme.set(prefs.theme);
    dpsMeterOpacity.set(prefs.overlay_dps_meter_opacity);
    statusEffectsOpacity.set(prefs.overlay_status_effects_opacity);
    settingsLoaded.set(true);
  })();
  return loading;
}

function currentPrefs(): PreferencesDto {
  return {
    volume: get(volume),
    era: get(era),
    save_profile: get(saveProfile),
    update_channel: get(updateChannel),
    theme: get(theme),
    overlay_dps_meter_opacity: get(dpsMeterOpacity),
    overlay_status_effects_opacity: get(statusEffectsOpacity),
  };
}

export async function setVolume(v: number) {
  volume.set(v);
  await api.setPreferences({ ...currentPrefs(), volume: v }).catch(() => {});
}

export async function setEra(e: string) {
  era.set(e);
  await api.setPreferences({ ...currentPrefs(), era: e }).catch(() => {});
}

export async function setSaveProfile(on: boolean) {
  saveProfile.set(on);
  await api.setPreferences({ ...currentPrefs(), save_profile: on }).catch(() => {});
}

export async function setUpdateChannel(channel: 'public' | 'beta') {
  updateChannel.set(channel);
  await api.setPreferences({ ...currentPrefs(), update_channel: channel }).catch(() => {});
}

export async function setTheme(slug: string) {
  theme.set(slug);
  await api.setPreferences({ ...currentPrefs(), theme: slug }).catch(() => {});
}

/** why: NOT persisted (see dpsMeterEnabled's own doc) -- the real
 * overlay window is one shared window holding every enabled widget, and
 * the backend itself tracks which widgets are on (AppState::
 * overlay_widgets): it opens on the first widget to enable, closes only
 * once the last one disables. Throws the backend's own plain-language
 * capability reason on failure (see windowcap.rs); the store still
 * flips on optimistically but the caller should show that reason rather
 * than pretend the window opened. */
export async function setDpsMeterEnabled(on: boolean) {
  dpsMeterEnabled.set(on);
  await api.setOverlayEnabled('dps_meter', on);
}

/** why: persists, and live-pushes to the open overlay window (a no-op
 * there if it isn't open) -- two separate calls, not one round trip,
 * since a slider drag shouldn't wait on a disk write to feel live */
export async function setDpsMeterOpacity(v: number) {
  dpsMeterOpacity.set(v);
  void api.setOverlayOpacity('dps_meter', v);
  await api.setPreferences({ ...currentPrefs(), overlay_dps_meter_opacity: v }).catch(() => {});
}

/** why: same contract as setDpsMeterEnabled -- see its own doc */
export async function setStatusEffectsEnabled(on: boolean) {
  statusEffectsEnabled.set(on);
  await api.setOverlayEnabled('status_effects', on);
}

/** why: same contract as setDpsMeterOpacity -- see its own doc */
export async function setStatusEffectsOpacity(v: number) {
  statusEffectsOpacity.set(v);
  void api.setOverlayOpacity('status_effects', v);
  await api.setPreferences({ ...currentPrefs(), overlay_status_effects_opacity: v }).catch(() => {});
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

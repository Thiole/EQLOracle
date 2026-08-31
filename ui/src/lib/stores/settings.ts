// why: single source of truth for the Settings module's own preferences --
// notification volume (not yet wired to real sound playback, see
// preferences.rs's own doc) and the wiki era Game Data/Gear should filter
// to. Both are read from other modules (GameData, Character's GearPanel),
// not just Settings itself, which is why this lives here rather than as
// local component state in Settings.svelte.
import { writable, derived, get } from 'svelte/store';
import { api, type PreferencesDto, type TrackedLootDto } from '../tauri/api';
import { asCcSize, DEFAULT_CC_SIZE, type CcSize } from '../overlay/ccSize';

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
/** why: the real master switch for the whole overlay system -- an
 * explicit, independent flag (NOT derived from "are all 4 widgets
 * currently on"), live-only same as every per-widget enabled flag below
 * (see their own doc). Off gates the per-widget toggles in
 * OverlayQuickMenu.svelte (the top-bar shortcut) so you can't
 * individually enable a widget before the system's even on; the
 * Settings page's own per-widget checkboxes stay independently
 * clickable regardless -- this flag doesn't gate those, only reflects/
 * drives the "enable everything at once" action there too (see
 * setOverlayEnabledAll's own doc). Toggling an individual widget off
 * after turning this on does NOT flip this back off -- it means "the
 * system is on," not "literally every widget is on right now". */
export const overlayEnabled = writable(false);
export const dpsMeterEnabled = writable(false);
/** why: this widget's own background alpha, 0.0 (invisible) to 1.0
 * (fully opaque) -- IS persisted, a real style choice worth keeping */
export const dpsMeterOpacity = writable(0.85);
/** why: the SEPARATE "everything" fade -- see PreferencesDto's own doc
 * on overlay_dps_meter_overall_opacity. 1.0 (fully opaque) by default. */
export const dpsMeterOverallOpacity = writable(1.0);
/** why: same on/off contract as dpsMeterEnabled -- see its own doc.
 * Covers all three of the Skill Tracker's own sections (status effects,
 * cooldowns, target effects) -- one widget, one window, one toggle. */
export const skillTrackerEnabled = writable(false);
export const skillTrackerOpacity = writable(0.85);
/** why: see dpsMeterOverallOpacity's own doc -- same "everything" fade,
 * this widget's own */
export const skillTrackerOverallOpacity = writable(1.0);
/** why: same on/off contract as dpsMeterEnabled -- see its own doc. CC
 * Tracker (Root/Stun/Fear squares) is its own peer widget, not a Skill
 * Tracker section -- see CCTrackerWidget.svelte's own doc. */
export const ccTrackerEnabled = writable(false);
export const ccTrackerOpacity = writable(0.85);
/** why: see dpsMeterOverallOpacity's own doc -- same "everything" fade,
 * this widget's own */
export const ccTrackerOverallOpacity = writable(1.0);
/** why: CC Tracker's own layout knob -- see ccSize.ts's own doc */
export const ccTrackerSize = writable<CcSize>(DEFAULT_CC_SIZE);
/** why: any ability/spell the player has "track"ed for the Skill
 * Tracker's own cooldowns section -- not a fixed list, populated by a
 * real "track" action wherever a spell/ability shows up (Combat's
 * ability rows, or the Skill Tracker's own settings card). IS
 * persisted, a real content choice, unlike the on/off above. Empty
 * until the user tracks something. Not per-target -- see
 * trackedTargetEffects below for that. */
export const trackedSkills = writable<string[]>([]);
/** why: separate from trackedSkills -- a spell added here (Spellbook's
 * "Overlay spell tracking" section) shows up ONLY against the current
 * target (landed? duration left?), never its own cooldown/READY row.
 * Empty by default -- nothing baked in. */
export const trackedTargetEffects = writable<string[]>([]);
/** why: same on/off contract as dpsMeterEnabled -- see its own doc */
export const dropWatchEnabled = writable(false);
export const dropWatchOpacity = writable(0.85);
/** why: see dpsMeterOverallOpacity's own doc -- same "everything" fade, this widget's own */
export const dropWatchOverallOpacity = writable(1.0);
/** why: item names to watch for -- see PreferencesDto.tracked_drop_items's
 * own doc. Entry points are Sky Quests' material chips and Primary Class
 * Unlocks' reward materials. */
export const trackedDropItems = writable<string[]>([]);
/** why: see PreferencesDto.tracked_drop_seen_counts's own doc -- the
 * "remove from Drop Watch?" prompt's own baseline, not a display value */
export const trackedDropSeenCounts = writable<Record<string, number>>({});
/** why: see PreferencesDto.drop_watch_checkpoint_ms's own doc --
 * dropWatchLoot.ts owns reading/writing this, this store just persists it */
export const dropWatchCheckpointMs = writable<number | null>(null);
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
    // why: list/map fields tolerate an absent value at this boundary --
    // a prefs blob older than a field (or a harness fixture predating
    // it) otherwise feeds undefined into components that .length/.map
    // it, crashing whole modules (caught by tests/render/responsive.
    // spec.ts on tracked_target_effects). Scalars keep their backend
    // #[serde(default)] guarantee; collections get the same here.
    eraOptions.set(opts?.eras ?? []);
    if (opts?.current) currentEra.set(opts.current);
    volume.set(prefs.volume);
    era.set(prefs.era);
    saveProfile.set(prefs.save_profile);
    updateChannel.set(prefs.update_channel);
    theme.set(prefs.theme);
    dpsMeterOpacity.set(prefs.overlay_dps_meter_opacity);
    dpsMeterOverallOpacity.set(prefs.overlay_dps_meter_overall_opacity);
    skillTrackerOpacity.set(prefs.overlay_skill_tracker_opacity);
    skillTrackerOverallOpacity.set(prefs.overlay_skill_tracker_overall_opacity);
    trackedSkills.set(prefs.tracked_skills ?? []);
    trackedTargetEffects.set(prefs.tracked_target_effects ?? []);
    dropWatchOpacity.set(prefs.overlay_drop_watch_opacity);
    dropWatchOverallOpacity.set(prefs.overlay_drop_watch_overall_opacity);
    ccTrackerOpacity.set(prefs.overlay_cc_tracker_opacity);
    ccTrackerOverallOpacity.set(prefs.overlay_cc_tracker_overall_opacity);
    ccTrackerSize.set(asCcSize(prefs.overlay_cc_tracker_size));
    trackedDropItems.set(prefs.tracked_drop_items ?? []);
    trackedDropSeenCounts.set(prefs.tracked_drop_seen_counts ?? {});
    dropWatchCheckpointMs.set(prefs.drop_watch_checkpoint_ms);
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
    overlay_dps_meter_overall_opacity: get(dpsMeterOverallOpacity),
    overlay_skill_tracker_opacity: get(skillTrackerOpacity),
    overlay_skill_tracker_overall_opacity: get(skillTrackerOverallOpacity),
    tracked_skills: get(trackedSkills),
    tracked_target_effects: get(trackedTargetEffects),
    overlay_drop_watch_opacity: get(dropWatchOpacity),
    overlay_drop_watch_overall_opacity: get(dropWatchOverallOpacity),
    overlay_cc_tracker_opacity: get(ccTrackerOpacity),
    overlay_cc_tracker_overall_opacity: get(ccTrackerOverallOpacity),
    overlay_cc_tracker_size: get(ccTrackerSize),
    tracked_drop_items: get(trackedDropItems),
    tracked_drop_seen_counts: get(trackedDropSeenCounts),
    drop_watch_checkpoint_ms: get(dropWatchCheckpointMs),
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

/** why: NOT persisted (see dpsMeterEnabled's own doc) -- each widget is
 * its own real OS window (see commands::overlay_label's own doc);
 * opening/closing this one doesn't touch any other widget's window.
 * Throws the backend's own plain-language capability reason on failure
 * (see windowcap.rs); the store still flips on optimistically but the
 * caller should show that reason rather than pretend the window opened. */
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

/** why: the SEPARATE "everything" fade -- same live-push/persist split as setDpsMeterOpacity above */
export async function setDpsMeterOverallOpacity(v: number) {
  dpsMeterOverallOpacity.set(v);
  void api.setOverlayOverallOpacity('dps_meter', v);
  await api.setPreferences({ ...currentPrefs(), overlay_dps_meter_overall_opacity: v }).catch(() => {});
}

/** why: same contract as setDpsMeterEnabled -- see its own doc */
export async function setSkillTrackerEnabled(on: boolean) {
  skillTrackerEnabled.set(on);
  await api.setOverlayEnabled('skill_tracker', on);
}

/** why: same contract as setDpsMeterOpacity -- see its own doc */
export async function setSkillTrackerOpacity(v: number) {
  skillTrackerOpacity.set(v);
  void api.setOverlayOpacity('skill_tracker', v);
  await api.setPreferences({ ...currentPrefs(), overlay_skill_tracker_opacity: v }).catch(() => {});
}

/** why: see setDpsMeterOverallOpacity's own doc -- same "everything" fade, this widget's own */
export async function setSkillTrackerOverallOpacity(v: number) {
  skillTrackerOverallOpacity.set(v);
  void api.setOverlayOverallOpacity('skill_tracker', v);
  await api
    .setPreferences({ ...currentPrefs(), overlay_skill_tracker_overall_opacity: v })
    .catch(() => {});
}

/** why: which cooldown skills show in the Skill Tracker's own section --
 * IS persisted (unlike enabled/opacity's live-push split, there's no
 * live overlay-window push here since the overlay window re-reads
 * preferences fresh on its own poll, no need to duplicate a push event
 * for a list that changes rarely) */
export async function setTrackedSkills(skills: string[]) {
  trackedSkills.set(skills);
  await api.setPreferences({ ...currentPrefs(), tracked_skills: skills }).catch(() => {});
}

/** why: the one call every real "track" button uses -- Spellbook's own
 * spell rows, Combat's ability rows, and the Skill Tracker's own
 * settings card (removal only, there) all just need "is this one
 * tracked, flip it", not the whole list */
export async function toggleTrackedSkill(name: string) {
  const current = get(trackedSkills);
  const next = current.includes(name) ? current.filter((s) => s !== name) : [...current, name];
  await setTrackedSkills(next);
}

/** why: which spells show against the current target -- see
 * trackedTargetEffects' own doc for why this is separate from
 * setTrackedSkills */
export async function setTrackedTargetEffects(spells: string[]) {
  trackedTargetEffects.set(spells);
  await api.setPreferences({ ...currentPrefs(), tracked_target_effects: spells }).catch(() => {});
}

/** why: the one call Spellbook's own "Overlay spell tracking" section
 * uses -- same "is this one tracked, flip it" shape as toggleTrackedSkill */
export async function toggleTrackedTargetEffect(name: string) {
  const current = get(trackedTargetEffects);
  const next = current.includes(name) ? current.filter((s) => s !== name) : [...current, name];
  await setTrackedTargetEffects(next);
}

/** why: same contract as setDpsMeterEnabled -- see its own doc */
export async function setDropWatchEnabled(on: boolean) {
  dropWatchEnabled.set(on);
  await api.setOverlayEnabled('drop_watch', on);
}

/** why: same contract as setDpsMeterOpacity -- see its own doc */
export async function setDropWatchOpacity(v: number) {
  dropWatchOpacity.set(v);
  void api.setOverlayOpacity('drop_watch', v);
  await api.setPreferences({ ...currentPrefs(), overlay_drop_watch_opacity: v }).catch(() => {});
}

/** why: see setDpsMeterOverallOpacity's own doc -- same "everything" fade, this widget's own */
export async function setDropWatchOverallOpacity(v: number) {
  dropWatchOverallOpacity.set(v);
  void api.setOverlayOverallOpacity('drop_watch', v);
  await api
    .setPreferences({ ...currentPrefs(), overlay_drop_watch_overall_opacity: v })
    .catch(() => {});
}

/** why: same contract as setDpsMeterEnabled -- see its own doc */
export async function setCcTrackerEnabled(on: boolean) {
  ccTrackerEnabled.set(on);
  await api.setOverlayEnabled('cc_tracker', on);
}

/** why: the one "turn everything on/off together" action, shared by
 * both the Settings page's own "enable ui" checkbox and
 * OverlayQuickMenu's own master toggle -- one real implementation
 * instead of two copies that could drift. Sets overlayEnabled explicitly
 * (see its own doc) rather than leaving it derived, since turning
 * everything off this way is exactly the "system off" case that flag
 * means. Errors from an individual widget (e.g. this session is
 * capability-capped) are swallowed here -- callers that need to surface
 * a reason per widget (Settings page) call the individual setters
 * directly instead of this one. */
export async function setOverlayEnabledAll(on: boolean) {
  overlayEnabled.set(on);
  await Promise.all([
    setDpsMeterEnabled(on).catch(() => {}),
    setSkillTrackerEnabled(on).catch(() => {}),
    setDropWatchEnabled(on).catch(() => {}),
    setCcTrackerEnabled(on).catch(() => {}),
  ]);
}

/** why: same contract as setDpsMeterOpacity -- see its own doc */
export async function setCcTrackerOpacity(v: number) {
  ccTrackerOpacity.set(v);
  void api.setOverlayOpacity('cc_tracker', v);
  await api.setPreferences({ ...currentPrefs(), overlay_cc_tracker_opacity: v }).catch(() => {});
}

/** why: see setDpsMeterOverallOpacity's own doc -- same "everything" fade, this widget's own */
export async function setCcTrackerOverallOpacity(v: number) {
  ccTrackerOverallOpacity.set(v);
  void api.setOverlayOverallOpacity('cc_tracker', v);
  await api
    .setPreferences({ ...currentPrefs(), overlay_cc_tracker_overall_opacity: v })
    .catch(() => {});
}

/** why: resizes the real OS window (if open), not just a CSS value --
 * same live-push/persist split as setCcTrackerOpacity above, see
 * ccSize.ts's own doc */
export async function setCcTrackerSize(v: CcSize) {
  ccTrackerSize.set(v);
  void api.setOverlaySize('cc_tracker', v);
  await api.setPreferences({ ...currentPrefs(), overlay_cc_tracker_size: v }).catch(() => {});
}

/** why: which items show a heads-up in the Drop Watch overlay -- IS
 * persisted, same as setTrackedSkills */
export async function setTrackedDropItems(items: string[]) {
  trackedDropItems.set(items);
  await api.setPreferences({ ...currentPrefs(), tracked_drop_items: items }).catch(() => {});
}

/** why: the "remove from Drop Watch?" prompt's own baseline -- see
 * PreferencesDto.tracked_drop_seen_counts's own doc */
export async function setTrackedDropSeenCounts(counts: Record<string, number>) {
  trackedDropSeenCounts.set(counts);
  await api.setPreferences({ ...currentPrefs(), tracked_drop_seen_counts: counts }).catch(() => {});
}

/** why: dropWatchLoot.ts's own periodic checkpoint save -- see
 * PreferencesDto.drop_watch_checkpoint_ms's own doc */
export async function setDropWatchCheckpointMs(ms: number) {
  dropWatchCheckpointMs.set(ms);
  await api.setPreferences({ ...currentPrefs(), drop_watch_checkpoint_ms: ms }).catch(() => {});
}

/** why: the one call every "track this drop" button uses -- Sky Quests'
 * material chips, Primary Class Unlocks' reward materials, and Gear
 * Planner's own unowned items -- same "is this one tracked, flip it"
 * shape as toggleTrackedSkill. Newly tracking something seeds its
 * prompt baseline to whatever's already been looted so far -- tracking
 * an item you already have shouldn't immediately prompt to remove it. */
export async function toggleTrackedDropItem(name: string) {
  const current = get(trackedDropItems);
  const adding = !current.includes(name);
  const next = adding ? [...current, name] : current.filter((s) => s !== name);
  await setTrackedDropItems(next);
  if (adding) {
    const [existing] = await api.getTrackedLootStatus([name]).catch(() => [] as TrackedLootDto[]);
    await setTrackedDropSeenCounts({ ...get(trackedDropSeenCounts), [name]: existing?.count ?? 0 });
  }
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

// why: single source of truth for Combat's zone/encounter selection
import { writable, get } from 'svelte/store';
import {
  api,
  type ZoneVisitDto,
  type EncounterDto,
  type CombatSummaryDto,
  type AllyDto,
  type FightTimelineDto,
  type EntityStateDto,
  type ParseRecordDto,
  type LoadoutSummaryDto,
} from '../tauri/api';

export const zoneVisits = writable<ZoneVisitDto[]>([]);
export const encounters = writable<EncounterDto[]>([]);
/** `null` = no filter (every zone visit); `-1` = the "Unknown" bucket. */
export const selectedZoneVisit = writable<number | null>(null);
/** `null` = aggregate across whatever the zone filter allows (when
 * `followCurrentFight` is false), or "no fight yet" (when it's true). */
export const selectedEncounterId = writable<number | null>(null);
/** why: the "current fight" mode -- true by default (see
 * `loadZoneVisitsThenJumpOrReset`), keeps `selectedEncounterId` pointed at
 * whichever fight is most recent as new ones start (`onCombatTick`).
 * Turned off by any explicit pick (Aggregate or a specific fight) and back
 * on only by `followCurrent`. */
export const followCurrentFight = writable(true);
export const summary = writable<CombatSummaryDto | null>(null);
export const allies = writable<AllyDto[]>([]);
export const timeline = writable<FightTimelineDto | null>(null);
export const stateAt = writable<{ tsMs: number; entities: EntityStateDto[] } | null>(null);
/** Which ally row (if any) is expanded to its own ability/cast breakdown. */
export const expandedAlly = writable<string | null>(null);
export const allySummary = writable<CombatSummaryDto | null>(null);

// ---------------------------------------------------------------- history pane

/** why: History pane's target; null for the aggregate view. A plain
 * writable, set explicitly wherever a fight gets selected (see
 * selectEncounter/jumpToEncounter) -- not derived from `encounters`,
 * since jumpToEncounter's own target can arrive before that fetch
 * resolves (see its own doc). */
export const historyTarget = writable<string | null>(null);
/** why: default true, avoids mixing resets with kills */
export const historyConfirmedOnly = writable<boolean>(true);
export const historyRecords = writable<ParseRecordDto[]>([]);
export const loadoutSummaries = writable<LoadoutSummaryDto[]>([]);

export async function loadZoneVisits() {
  zoneVisits.set((await api.listZoneVisits()) ?? []); // defensive -- invoke<T>()'s type is an assertion, not a guarantee
}

/** why: Game Data's "open in Combat →" sets this, then switches modules
 * to mount Combat.svelte -- that component's own mount effect consumes
 * this (instead of its usual "reset to All zones / Aggregate" default)
 * and clears it. A store, not a direct call to jumpToEncounter, because
 * Combat.svelte's mount effect always runs loadZoneVisits() first
 * (needed either way) and a jump landing before that resolves would just
 * get overwritten by the default reset that used to always follow it.
 * Carries `target` too -- see jumpToEncounter's own doc for why. */
export const pendingJump = writable<{ zoneVisit: number | null; encounterId: number; target: string } | null>(null);

export function requestJumpToEncounter(zoneVisit: number | null, encounterId: number, target: string) {
  pendingJump.set({ zoneVisit, encounterId, target });
}

/** why: Combat.svelte's own mount effect -- loads the zone list either
 * way, then either honors a pending Game Data jump or falls back to the
 * plain "All zones" default it always reset to before this existed. */
export async function loadZoneVisitsThenJumpOrReset() {
  const jump = get(pendingJump);
  pendingJump.set(null);
  await loadZoneVisits();
  if (jump) {
    await jumpToEncounter(jump.zoneVisit, jump.encounterId, jump.target);
  } else {
    await selectZoneVisit(null);
  }
}

// ---------------------------------------------------------------- encounters

let encounterLoadToken = 0;

/** why: the whole matching list, always -- fetching it turned out to be
 * cheap even for a long-lived character's thousands of fights (a single
 * sort + slice + per-row compute, see combat::list_encounters' own doc).
 * The actual cost was *rendering* that many <Select.Item>s at once, which
 * Combat.svelte now handles by virtualizing what it mounts -- not by
 * withholding data the user might scroll to. */
async function loadEncounters(zv: number | null) {
  const token = ++encounterLoadToken;
  const list = (await api.listEncounters(zv)) ?? [];
  if (token !== encounterLoadToken) return;
  encounters.set(list);
}

/** why: `list_encounters` sorts newest-first (see its own doc) -- the
 * first entry is always the most recently *started* fight, whether it's
 * still open or already closed, which is exactly "current fight" means
 * once nothing is actively in progress. `null` when there's no fight yet
 * at all (a fresh zone, or before the log has any). */
function mostRecentEncounterId(list: EncounterDto[]): number | null {
  return list[0]?.id ?? null;
}

export async function selectZoneVisit(zv: number | null) {
  selectedZoneVisit.set(zv);
  expandedAlly.set(null);
  await loadEncounters(zv);
  if (get(followCurrentFight)) {
    const id = mostRecentEncounterId(get(encounters));
    selectedEncounterId.set(id);
    historyTarget.set(id !== null ? (get(encounters).find((e) => e.id === id)?.target ?? null) : null);
  } else {
    selectedEncounterId.set(null);
    historyTarget.set(null);
  }
  await refreshSelection();
  await refreshHistory();
}

/** why: `targetOverride` is set by jumpToEncounter, whose caller already
 * knows the real target name (Game Data's own ZoneEncounterDto) -- faster
 * than waiting on `encounters` to (re)load after selectZoneVisit before
 * looking the id up there. The normal dropdown-pick path (no override)
 * still looks it up from `encounters`, safe there since `id` always comes
 * from an entry the dropdown is currently showing. */
/** why: the dropdown's "Aggregate" and specific-fight rows both go through
 * here -- either one is an explicit, fixed pick, so `followCurrentFight`
 * turns off; `followCurrent` is the only way back on. */
export async function selectEncounter(id: number | null, targetOverride?: string) {
  followCurrentFight.set(false);
  selectedEncounterId.set(id);
  expandedAlly.set(null);
  if (id === null) {
    historyTarget.set(null);
  } else {
    historyTarget.set(targetOverride ?? get(encounters).find((e) => e.id === id)?.target ?? null);
  }
  await refreshSelection();
  await refreshHistory();
}

/** why: the dropdown's "Current fight" row -- (re)enables follow mode and
 * snaps to whatever's most recent right now; `onCombatTick` keeps it
 * pointed at the newest fight from here on, same as the default on mount. */
export async function followCurrent() {
  followCurrentFight.set(true);
  const id = mostRecentEncounterId(get(encounters));
  selectedEncounterId.set(id);
  expandedAlly.set(null);
  historyTarget.set(id !== null ? (get(encounters).find((e) => e.id === id)?.target ?? null) : null);
  await refreshSelection();
  await refreshHistory();
}

/** why: Game Data's "open in Combat →" -- `zoneVisit` is a
 * `ZoneEncounterDto`'s own field, where `null` means the pre-first-zone-
 * line "Unknown" bucket; `selectZoneVisit`'s own convention for that same
 * bucket is the sentinel `-1`, not `null` (its `null` means "no filter,
 * every visit" instead) -- translated here so callers never have to know
 * the two stores disagree on what `null` means. `target` is passed
 * through to selectEncounter -- see its own doc for why that beats
 * looking it up locally. */
export async function jumpToEncounter(zoneVisit: number | null, encounterId: number, target: string) {
  await selectZoneVisit(zoneVisit ?? -1);
  await selectEncounter(encounterId, target);
}

export async function setHistoryConfirmedOnly(v: boolean) {
  historyConfirmedOnly.set(v);
  await refreshHistory();
}

let historyToken = 0;

/** why: refetch History pane for current target; output: void, clears if none */
async function refreshHistory() {
  const target = get(historyTarget);
  if (target == null) {
    historyRecords.set([]);
    loadoutSummaries.set([]);
    return;
  }
  const token = ++historyToken;
  const confirmedOnly = get(historyConfirmedOnly);
  const [records, loadouts] = await Promise.all([api.getMobHistory(target, confirmedOnly), api.getLoadoutSummary(target, confirmedOnly)]);
  if (token !== historyToken) return;
  // defensive -- invoke<T>()'s type is an assertion, not a guarantee
  historyRecords.set(records ?? []);
  loadoutSummaries.set(loadouts ?? []);
}

export async function toggleAlly(name: string) {
  const current = get(expandedAlly);
  if (current === name) {
    expandedAlly.set(null);
    allySummary.set(null);
    return;
  }
  expandedAlly.set(name);
  const zv = get(selectedZoneVisit);
  const enc = get(selectedEncounterId);
  allySummary.set(await api.getCombatSummary(zv, enc, name));
}

export async function scrubTo(tsMs: number) {
  const enc = get(selectedEncounterId);
  if (enc == null) return;
  stateAt.set({ tsMs, entities: await api.getFightStateAt(enc, tsMs) });
}

/** why: `preserveScrub` -- a user-picked timeline scrub point (`stateAt`,
 * see scrubTo's own doc) names a fixed, already-past `tsMs`; the data at
 * that instant never changes, so there's nothing to refetch, only a
 * reason NOT to blow it away. Real bug, caught live: onCombatTick's own
 * periodic re-poll while still watching the SAME open fight called this
 * unconditionally, clearing a just-picked scrub point a few seconds
 * after clicking it -- "selecting a point on the fight timeline loads
 * the data below it, but after a few seconds it goes away". Defaults to
 * clearing (false) for every call site that's an actual selection
 * change (a different zone/encounter, or Current Fight re-armed) --
 * that scrub point really is stale then, it belongs to whatever fight
 * was showing before. Only onCombatTick's own "still the same fight,
 * just a live refresh" paths pass true. */
async function refreshSelection(preserveScrub = false) {
  const zv = get(selectedZoneVisit);
  const enc = get(selectedEncounterId);
  const [s, a] = await Promise.all([api.getCombatSummary(zv, enc, null), api.listAllies(zv, enc)]);
  summary.set(s);
  allies.set(a ?? []); // defensive -- invoke<T>()'s type is an assertion, not a guarantee
  timeline.set(enc != null ? await api.getFightTimeline(enc) : null);
  if (!preserveScrub) stateAt.set(null);
  const expanded = get(expandedAlly);
  if (expanded) allySummary.set(await api.getCombatSummary(zv, enc, expanded));
}

// why: refetch open selection only; closed fights untouched, no teardown
export async function onCombatTick() {
  const zv = get(selectedZoneVisit);
  encounters.set((await api.listEncounters(zv)) ?? []); // defensive -- invoke<T>() is an assertion, not a guarantee

  // why: cheap, so refresh History unconditionally when a target is selected
  if (get(historyTarget) != null) void refreshHistory();

  if (get(followCurrentFight)) {
    // why: re-point to whatever's most recent *now* -- a new fight
    // starting is exactly the case this mode exists to follow, not just
    // "keep refreshing the one already selected".
    const id = mostRecentEncounterId(get(encounters));
    const changedFight = id !== get(selectedEncounterId);
    if (changedFight) {
      selectedEncounterId.set(id);
      historyTarget.set(id !== null ? (get(encounters).find((e) => e.id === id)?.target ?? null) : null);
    }
    // why: preserve a scrub point unless this tick actually moved to a
    // different fight -- see refreshSelection's own doc
    await refreshSelection(!changedFight);
    return;
  }

  const enc = get(selectedEncounterId);
  if (enc != null) {
    const current = get(encounters).find((e) => e.id === enc);
    if (!current?.open) return;
  }
  // why: same fight throughout this branch -- selectedEncounterId never
  // changes here, only a live poll of its still-open data
  await refreshSelection(true);
}

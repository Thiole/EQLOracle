// why: single source of truth for Combat's scope -- what the fight tree
// shows, what is selected in it, and what every number describes
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
  type SelectionDto,
  type MobRowDto,
} from '../tauri/api';

// ---------------------------------------------------------------- tree
/** `index` null is the pre-first-zone-line "Unknown" bucket; the tree
 * keys it as 'u', the backend as -1 */
export const zoneVisits = writable<ZoneVisitDto[]>([]);
export const visitKey = (visit: number | null) => (visit === null ? 'u' : String(visit));
export const visitArg = (visit: number | null) => (visit === null ? -1 : visit);
/** fights per visit, loaded when a visit is expanded (or followed) */
export const visitFights = writable<Record<string, EncounterDto[]>>({});
export const expandedVisits = writable<Set<string>>(new Set());
/** why: visits bundle by the log-time day their first line fell on; a
 * visit never splits across two days */
export const dayOf = (startMs: number) => {
  const d = new Date(startMs);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())}`;
};
export const expandedDays = writable<Set<string>>(new Set());
export function toggleDayExpanded(day: string) {
  const next = new Set(get(expandedDays));
  if (next.has(day)) next.delete(day);
  else next.add(day);
  expandedDays.set(next);
}
/** mobs per fight, loaded when a fight is expanded */
export const encounterMobs = writable<Record<number, MobRowDto[]>>({});
export const expandedEncounters = writable<Set<number>>(new Set());

// ---------------------------------------------------------------- selection
/** why: the persistent selection -- survives browsing other visits, so
 * fights from anywhere combine. Whole visits and log-time ranges are
 * members like fights are. */
export type Member =
  | { kind: 'day'; day: string }
  | { kind: 'visit'; visit: number | null }
  | { kind: 'encounter'; id: number; visit: number | null; target: string }
  | { kind: 'mob'; id: number; visit: number | null; name: string }
  | { kind: 'range'; since: number; until: number };
export const selection = writable<Member[]>([]);
/** ranges the user has defined; listed in the tree, selectable like fights */
export const ranges = writable<{ since: number; until: number }[]>([]);
/** why: on by default -- the selection tracks the newest fight until a
 * hand pick turns it off; `followCurrent` is the only way back on */
export const followCurrentFight = writable(true);

export const memberKey = (m: Member) =>
  m.kind === 'day'
    ? `d:${m.day}`
    : m.kind === 'visit'
    ? `v:${visitKey(m.visit)}`
    : m.kind === 'encounter'
      ? `e:${m.id}`
      : m.kind === 'mob'
        ? `m:${m.id}:${m.name}`
        : `r:${m.since}-${m.until}`;
export function isSelected(m: Member, sel: Member[] = get(selection)): boolean {
  const k = memberKey(m);
  return sel.some((x) => memberKey(x) === k);
}

export const summary = writable<CombatSummaryDto | null>(null);
export const allies = writable<AllyDto[]>([]);
/** why: the other side of the same fights, off by default -- one Hate
 * pull is 28 distinct mobs against 3-6 allies, so it stays collapsed and
 * is not even fetched until asked for. */
export const enemies = writable<AllyDto[]>([]);
export const showEnemies = writable(false);
export const timeline = writable<FightTimelineDto | null>(null);
export const stateAt = writable<{ tsMs: number; entities: EntityStateDto[] } | null>(null);
/** Which ally row (if any) is expanded to its own ability/cast breakdown. */
export const expandedAlly = writable<string | null>(null);
export const allySummary = writable<CombatSummaryDto | null>(null);
// why: an expanded owner is a folder -- each charmed/summoned pet under
// it gets its own summary, keyed by the pet's store name
export const petSummaries = writable<Record<string, CombatSummaryDto>>({});

// ---------------------------------------------------------------- history pane
/** why: the past-parses target -- the one fight selected, else nothing */
export const historyTarget = writable<string | null>(null);
/** why: default true, avoids mixing resets with kills */
export const historyConfirmedOnly = writable<boolean>(true);
export const historyRecords = writable<ParseRecordDto[]>([]);
export const loadoutSummaries = writable<LoadoutSummaryDto[]>([]);

/** why: exactly one fight selected and nothing else -- the timeline,
 * the history pane and the copy report's title all key on this */
export function singleEncounter(sel: Member[] = get(selection)): Extract<Member, { kind: 'encounter' }> | null {
  return sel.length === 1 && sel[0].kind === 'encounter' ? sel[0] : null;
}

/** why: one mob and nothing else -- the fight's timeline still applies,
 * and the past-parses pane keys on the mob's name */
export function singleMob(sel: Member[] = get(selection)): Extract<Member, { kind: 'mob' }> | null {
  return sel.length === 1 && sel[0].kind === 'mob' ? sel[0] : null;
}

export function selectionOf(sel: Member[]): SelectionDto {
  // why: a day is every visit filed under it -- resolved here, the backend knows visits
  const days = new Set(sel.flatMap((m) => (m.kind === 'day' ? [m.day] : [])));
  const dayVisits = days.size ? get(zoneVisits).filter((v) => days.has(dayOf(v.start_ms))).map((v) => visitArg(v.index)) : [];
  return {
    encounters: sel.flatMap((m) => (m.kind === 'encounter' ? [m.id] : [])),
    ranges: sel.flatMap((m) => (m.kind === 'range' ? [[m.since, m.until] as [number, number]] : [])),
    visits: [...new Set([...sel.flatMap((m) => (m.kind === 'visit' ? [visitArg(m.visit)] : [])), ...dayVisits])],
    mobs: sel.flatMap((m) => (m.kind === 'mob' ? [[m.id, m.name] as [number, string]] : [])),
  };
}

/** why: one fight or one visit goes through the plain zone/encounter
 * arguments (the mock fixture keys on those); anything else is a
 * selection the backend resolves */
export function scope(): { zv: number | null; enc: number | null; sel: SelectionDto | null } {
  const sel = get(selection);
  const one = singleEncounter(sel);
  if (one) return { zv: null, enc: one.id, sel: null };
  if (sel.length === 1 && sel[0].kind === 'visit') return { zv: visitArg(sel[0].visit), enc: null, sel: null };
  return { zv: null, enc: null, sel: selectionOf(sel) };
}

export async function loadZoneVisits() {
  zoneVisits.set((await api.listZoneVisits()) ?? []); // defensive -- invoke<T>()'s type is an assertion, not a guarantee
}

export async function loadVisitFights(visit: number | null) {
  const list = (await api.listEncounters(visitArg(visit))) ?? [];
  visitFights.update((m) => ({ ...m, [visitKey(visit)]: list }));
  return list;
}

export async function toggleEncounterExpanded(id: number) {
  const next = new Set(get(expandedEncounters));
  if (next.has(id)) next.delete(id);
  else {
    next.add(id);
    if (!get(encounterMobs)[id]) {
      const mobs = (await api.listEncounterMobs(id)) ?? [];
      encounterMobs.update((m) => ({ ...m, [id]: mobs }));
    }
  }
  expandedEncounters.set(next);
}

export async function toggleVisitExpanded(visit: number | null) {
  const k = visitKey(visit);
  const next = new Set(get(expandedVisits));
  if (next.has(k)) next.delete(k);
  else {
    next.add(k);
    if (!get(visitFights)[k]) await loadVisitFights(visit);
  }
  expandedVisits.set(next);
}

/** why: Game Data's "open in Combat →" sets this, then switches modules
 * to mount Combat.svelte -- that component's own mount effect consumes
 * it instead of the plain default. */
export const pendingJump = writable<{ zoneVisit: number | null; encounterId: number; target: string } | null>(null);
export function requestJumpToEncounter(zoneVisit: number | null, encounterId: number, target: string) {
  pendingJump.set({ zoneVisit, encounterId, target });
}

export async function loadZoneVisitsThenJumpOrReset() {
  const jump = get(pendingJump);
  pendingJump.set(null);
  await loadZoneVisits();
  if (jump) {
    await jumpToEncounter(jump.zoneVisit, jump.encounterId, jump.target);
  } else if (get(followCurrentFight)) {
    await followCurrent();
  } else {
    await refreshSelection();
  }
}

function currentVisit(): ZoneVisitDto | undefined {
  const visits = get(zoneVisits);
  return visits.find((v) => v.current) ?? visits[0];
}

/** why: `list_encounters` sorts newest-first -- an open fight beats a
 * newer-started closed one; "current" means the one you're in */
function newestFight(list: EncounterDto[]): EncounterDto | undefined {
  return list.find((e) => e.open) ?? list[0];
}

function toMember(e: EncounterDto, visit: number | null): Member {
  return { kind: 'encounter', id: e.id, visit, target: e.target };
}

async function applySelection(next: Member[], preserveScrub = false) {
  selection.set(next);
  expandedAlly.set(null);
  historyTarget.set(singleEncounter(next)?.target ?? singleMob(next)?.name ?? null);
  await refreshSelection(preserveScrub);
  await refreshHistory();
}

/** why: a hand pick -- replaces the selection and turns follow mode off */
export async function setSelection(next: Member[]) {
  followCurrentFight.set(false);
  await applySelection(next);
}

export async function clearSelection() {
  followCurrentFight.set(false);
  await applySelection([]);
}

export async function addRange(since: number, until: number) {
  ranges.update((r) => (r.some((x) => x.since === since && x.until === until) ? r : [...r, { since, until }]));
  followCurrentFight.set(false);
  const m: Member = { kind: 'range', since, until };
  await applySelection(isSelected(m) ? get(selection) : [...get(selection), m]);
}

export async function removeRange(since: number, until: number) {
  ranges.update((r) => r.filter((x) => !(x.since === since && x.until === until)));
  const k = memberKey({ kind: 'range', since, until });
  const next = get(selection).filter((m) => memberKey(m) !== k);
  if (next.length !== get(selection).length) await applySelection(next);
}

/** why: (re)arms follow mode and snaps to the newest fight of the
 * current visit; `onCombatTick` keeps it there from here on */
export async function followCurrent() {
  followCurrentFight.set(true);
  const v = currentVisit();
  if (!v) {
    await applySelection([]);
    return;
  }
  const k = visitKey(v.index);
  expandedDays.update((s) => new Set(s).add(dayOf(v.start_ms)));
  expandedVisits.update((s) => new Set(s).add(k));
  const list = get(visitFights)[k] ?? (await loadVisitFights(v.index));
  const e = newestFight(list);
  await applySelection(e ? [toMember(e, v.index)] : []);
}

/** why: Game Data's "open in Combat →" and a past-parse row -- a
 * `ZoneEncounterDto`'s own `zone_visit`, where null means the "Unknown" bucket */
export async function jumpToEncounter(zoneVisit: number | null, encounterId: number, target: string) {
  followCurrentFight.set(false);
  const k = visitKey(zoneVisit);
  const v = get(zoneVisits).find((x) => x.index === zoneVisit);
  if (v) expandedDays.update((s) => new Set(s).add(dayOf(v.start_ms)));
  expandedVisits.update((s) => new Set(s).add(k));
  if (!get(visitFights)[k]) await loadVisitFights(zoneVisit);
  await applySelection([{ kind: 'encounter', id: encounterId, visit: zoneVisit, target }]);
}

/** why: a past-parse row IS an encounter -- open exactly that fight */
export async function jumpToParse(r: ParseRecordDto) {
  if (r.encounter_id == null) return;
  await jumpToEncounter(r.zone_visit, r.encounter_id, r.target);
}

export async function setHistoryConfirmedOnly(v: boolean) {
  historyConfirmedOnly.set(v);
  await refreshHistory();
}

let historyToken = 0;
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
  historyRecords.set(records ?? []);
  loadoutSummaries.set(loadouts ?? []);
}

export async function toggleEnemies() {
  const on = !get(showEnemies);
  showEnemies.set(on);
  if (!on) {
    enemies.set([]);
    return;
  }
  const { zv, enc, sel } = scope();
  enemies.set((await api.listEnemies(zv, enc, false, sel)) ?? []);
}

export async function toggleAlly(name: string) {
  const current = get(expandedAlly);
  if (current === name) {
    expandedAlly.set(null);
    allySummary.set(null);
    petSummaries.set({});
    return;
  }
  expandedAlly.set(name);
  await loadExpanded(name);
}

// why: the owner's own rows and one block per pet, fetched together
async function loadExpanded(name: string) {
  const { zv, enc, sel } = scope();
  const pets = get(allies).find((a) => a.name === name)?.pets ?? [];
  const [own, ...petSums] = await Promise.all([
    api.getCombatSummary(zv, enc, name, false, sel),
    ...pets.map((p) => api.getCombatSummary(zv, enc, p.name, false, sel)),
  ]);
  if (get(expandedAlly) !== name) return;
  allySummary.set(own);
  petSummaries.set(Object.fromEntries(pets.map((p, i) => [p.name, petSums[i]])));
}

function timelineEncounter(): number | null {
  return singleEncounter()?.id ?? singleMob()?.id ?? null;
}

/** why: the "recent" window behind a chart click, in seconds; 6 is the
 * backend's own default and the fixture's key */
export const inspectWindowS = writable(6);

export async function scrubTo(tsMs: number) {
  const enc = timelineEncounter();
  if (enc == null) return;
  const w = get(inspectWindowS);
  stateAt.set({ tsMs, entities: await api.getFightStateAt(enc, tsMs, w === 6 ? undefined : w * 1000) });
}

export async function setInspectWindow(seconds: number) {
  inspectWindowS.set(seconds);
  const at = get(stateAt);
  if (at) await scrubTo(at.tsMs);
}

/** why: `preserveScrub` -- a picked timeline instant never changes, so a
 * live re-poll of the same fight must not clear it; a real selection
 * change does (see the incident in git history: the scrub point vanished
 * seconds after clicking it) */
let selectionToken = 0;
export async function refreshSelection(preserveScrub = false) {
  if (!get(selection).length) {
    summary.set(null);
    allies.set([]);
    enemies.set([]);
    timeline.set(null);
    stateAt.set(null);
    allySummary.set(null);
    petSummaries.set({});
    return;
  }
  const token = ++selectionToken;
  const { zv, enc, sel } = scope();
  const [s, a] = await Promise.all([api.getCombatSummary(zv, enc, null, false, sel), api.listAllies(zv, enc, false, sel)]);
  if (token !== selectionToken) return;
  summary.set(s);
  allies.set(a ?? []); // defensive -- invoke<T>()'s type is an assertion, not a guarantee
  if (get(showEnemies)) enemies.set((await api.listEnemies(zv, enc, false, sel)) ?? []);
  const tl = timelineEncounter();
  timeline.set(tl != null ? await api.getFightTimeline(tl) : null);
  if (!preserveScrub) stateAt.set(null);
  const expanded = get(expandedAlly);
  if (expanded) await loadExpanded(expanded);
}

// why: refetch what is on screen; closed fights untouched, no teardown
export async function onCombatTick() {
  // why: the zone list refreshes every tick too -- a fresh instance used
  // to snapshot it mid-backfill and read "All zones (0)" forever
  void loadZoneVisits();
  const v = currentVisit();
  const keys = new Set(get(expandedVisits));
  if (v) keys.add(visitKey(v.index));
  await Promise.all(
    [...keys].map((k) => loadVisitFights(k === 'u' ? null : Number(k))),
  );
  if (get(historyTarget) != null) void refreshHistory();

  if (get(followCurrentFight)) {
    // why: re-point to whatever's newest *now* -- a new fight starting
    // is exactly the case this mode exists to follow
    if (!v) return;
    const e = newestFight(get(visitFights)[visitKey(v.index)] ?? []);
    const next = e ? [toMember(e, v.index)] : [];
    const changed = next.map(memberKey).join() !== get(selection).map(memberKey).join();
    if (changed) await applySelection(next);
    else await refreshSelection(true);
    return;
  }
  // why: a hand-picked selection is live-polled only while something in it is still open
  const fights = get(visitFights);
  const open = get(selection).some((m) => {
    if (m.kind === 'range') return false;
    if (m.kind === 'day') return Object.values(fights).some((list) => list.some((e) => e.open));
    if (m.kind === 'visit') return (fights[visitKey(m.visit)] ?? []).some((e) => e.open);
    return (fights[visitKey(m.visit)] ?? []).find((e) => e.id === m.id)?.open ?? false;
  });
  // why: an open fight's mob list grows too
  if (open) {
    const ids = [...get(expandedEncounters)];
    await Promise.all(ids.map(async (id) => {
      const mobs = (await api.listEncounterMobs(id)) ?? [];
      encounterMobs.update((m) => ({ ...m, [id]: mobs }));
    }));
  }
  if (open) await refreshSelection(true);
}

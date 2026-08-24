// why: Spencer's own manual ranking of "which member of a spell line is
// actually best" -- the suggestion engine's default (-level, see
// spellSuggest.ts's priorityRank) gets this wrong for lines where a
// later, higher-level member isn't the better pick for every purpose
// (Mesmerization, an AE mez, beats the single-target mez spells that
// come after it in level). No way to infer that generically from the
// scraped wiki data, so this is a manual override instead of a guess.
import { writable, get } from 'svelte/store';
import { activeModule } from './shell';
import { allSpellLines, type SpellLine } from '../character/spellSuggest';
import type { SpellDto } from '../tauri/api';

const STORAGE_KEY = 'eqlp-spell-line-priority-v1';

function loadOverrides(): Record<string, string[]> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as Record<string, string[]>;
  } catch {
    // why: a private window, cleared site data, or a blocked storage
    // access all read as "nothing saved yet" -- never a reason to fail the page.
  }
  return {};
}

export const spellLineOverrides = writable<Record<string, string[]>>(loadOverrides());

spellLineOverrides.subscribe((v) => {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(v));
  } catch {
    // why: same as loadOverrides -- a save that can't land shouldn't throw; stays session-only this time.
  }
});

/** why: which line the settings page should auto-select on open -- a
 * one-shot deep-link value, not a permanent binding (SpellLinePriority.svelte
 * reads it once via an $effect, same "seed then let go" shape armed/
 * classesSeeded already use in SpellbookBuilder.svelte). */
export const openSpellLineKey = writable<string | null>(null);

/** why: switches to the Settings module too, not just openSpellLineKey --
 * mirrors gdOpenPage (stores/gamedata.ts) exactly: a cross-module link
 * that lands behind whatever module happens to already be visible would
 * look like nothing happened at all. */
export function openSpellLinePriority(key: string | null) {
  openSpellLineKey.set(key);
  activeModule.set('settings');
}

/** why: the order actually in effect right now for a line -- the saved
 * override if one exists, else the line's own default (-level) member
 * order, so the first reorder click on an untouched line moves a real,
 * already-displayed list instead of an empty override array. */
function effectiveOrder(line: SpellLine): string[] {
  return get(spellLineOverrides)[line.key] ?? line.members.map((s) => s.name);
}

function findLine(spells: SpellDto[], lineKey: string): SpellLine | null {
  return allSpellLines(spells).find((l) => l.key === lineKey) ?? null;
}

function swap(order: string[], i: number, j: number): string[] {
  const next = [...order];
  [next[i], next[j]] = [next[j], next[i]];
  return next;
}

export function moveUp(spells: SpellDto[], lineKey: string, name: string) {
  const line = findLine(spells, lineKey);
  if (!line) return;
  const order = effectiveOrder(line);
  const i = order.indexOf(name);
  if (i <= 0) return;
  spellLineOverrides.update((v) => ({ ...v, [lineKey]: swap(order, i, i - 1) }));
}

export function moveDown(spells: SpellDto[], lineKey: string, name: string) {
  const line = findLine(spells, lineKey);
  if (!line) return;
  const order = effectiveOrder(line);
  const i = order.indexOf(name);
  if (i < 0 || i >= order.length - 1) return;
  spellLineOverrides.update((v) => ({ ...v, [lineKey]: swap(order, i, i + 1) }));
}

export function resetLine(lineKey: string) {
  spellLineOverrides.update((v) => {
    const next = { ...v };
    delete next[lineKey];
    return next;
  });
}

export function resetAllSpellLinePriorities() {
  spellLineOverrides.set({});
}

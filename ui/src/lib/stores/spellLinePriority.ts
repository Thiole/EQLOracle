// why: Spencer's own manual ranking of "which member of a spell line is
// actually best" -- the suggestion engine's default (-level, see
// spellSuggest.ts's priorityRank) gets this wrong for lines where a
// later, higher-level member isn't actually the better pick for every
// purpose. No way to infer that generically from the scraped wiki data,
// so this is a manual override instead of a guess.
import { writable, get } from 'svelte/store';
import { activeModule } from './shell';
import { membersOfLine } from '../character/spellSuggest';
import type { SpellDto } from '../tauri/api';

const OVERRIDES_KEY = 'eqlp-spell-line-priority-v1';
const MEMBERSHIP_KEY = 'eqlp-spell-line-membership-v1';

function loadJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw) return JSON.parse(raw) as T;
  } catch {
    // why: a private window, cleared site data, or a blocked storage
    // access all read as "nothing saved yet" -- never a reason to fail the page.
  }
  return fallback;
}

function trySave(key: string, v: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(v));
  } catch {
    // why: same as loadJson -- a save that can't land shouldn't throw; stays session-only this time.
  }
}

export const spellLineOverrides = writable<Record<string, string[]>>(loadJson(OVERRIDES_KEY, {}));
spellLineOverrides.subscribe((v) => trySave(OVERRIDES_KEY, v));

/** why: real, manually-asserted "these overwrite each other" links --
 * spell name -> the target line's key. Never auto-detected (see
 * effectiveLineKey's own doc for why: the real stacking-group data
 * doesn't cover most lines, and guessing by similar effect risks a
 * false merge like Tash/Malosi, two genuinely separate resist-decrease
 * debuffs). Spencer's own real game knowledge (a class's Slow overwrites
 * another class's, say) is the only trustworthy source for this. */
export const spellLineCustomMembership = writable<Record<string, string>>(loadJson(MEMBERSHIP_KEY, {}));
spellLineCustomMembership.subscribe((v) => trySave(MEMBERSHIP_KEY, v));

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
function effectiveOrder(members: SpellDto[], lineKey: string): string[] {
  return get(spellLineOverrides)[lineKey] ?? members.map((s) => s.name);
}

function swap(order: string[], i: number, j: number): string[] {
  const next = [...order];
  [next[i], next[j]] = [next[j], next[i]];
  return next;
}

export function moveUp(spells: SpellDto[], lineKey: string, name: string) {
  const members = membersOfLine(spells, lineKey, get(spellLineCustomMembership));
  const order = effectiveOrder(members, lineKey);
  const i = order.indexOf(name);
  if (i <= 0) return;
  spellLineOverrides.update((v) => ({ ...v, [lineKey]: swap(order, i, i - 1) }));
}

export function moveDown(spells: SpellDto[], lineKey: string, name: string) {
  const members = membersOfLine(spells, lineKey, get(spellLineCustomMembership));
  const order = effectiveOrder(members, lineKey);
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
  spellLineCustomMembership.set({});
}

/** why: pulls `spellName` out of whatever line it's currently in (natural
 * or already-merged) and asserts it belongs to `lineKey` instead --
 * purely manual, see spellLineCustomMembership's own doc. */
export function addSpellToLine(lineKey: string, spellName: string) {
  spellLineCustomMembership.update((v) => ({ ...v, [spellName]: lineKey }));
}

/** why: undoes a manual merge -- the spell falls back to its own natural
 * (wiki-description-derived) line, same as before it was ever added anywhere. */
export function removeSpellFromLine(spellName: string) {
  spellLineCustomMembership.update((v) => {
    const next = { ...v };
    delete next[spellName];
    return next;
  });
}

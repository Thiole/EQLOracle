// why: the Death Recap's own navigation + toast state -- the recap is a
// dedicated page (activeModule 'deathrecap'), NOT a Combat-page panel
// (player's own call: the inline panel overcrowded Combat) and NOT a
// sidebar tab of its own either. The way in is a timed "Death recap?"
// toast that fires when a new death lands; clicking it opens the page.
// Polled off the same parse-tick every other live panel uses.
import { get, writable } from 'svelte/store';
import { api } from '../tauri/api';
import { activeModule } from './shell';
import { status } from './status';

/** why: same duration the Drop Watch loot prompt settled on (30s,
 * doubled from 15 by real experience) -- long enough to notice while
 * dealing with having just died, short enough it doesn't go stale */
export const DEATH_TOAST_TIMEOUT_MS = 30_000;

export interface DeathToast {
  deathTs: number;
  expiresAtMs: number;
}

export const deathToast = writable<DeathToast | null>(null);

/** why: which death the page opens on -- set by the toast click so the
 * page lands on the death that triggered it; null = follow latest */
export const recapPinned = writable<number | null>(null);

/** why: session death timestamps, shared with the page so it doesn't
 * re-fetch the list separately from the poll below */
export const deathList = writable<number[]>([]);

// why: -1 = no baseline yet. The first completed poll AFTER backfill
// sets the baseline without toasting -- deaths replayed from an old log
// on launch are history, not news.
let lastSeenCount = -1;

/** why: called every parse-tick (tauri/events.ts) */
export async function pollDeaths(): Promise<void> {
  const st = get(status);
  if (!st || !st.status || st.status.backfilling) return;
  const res = await api.getDeathRecap(null).catch(() => null);
  if (!res) return;
  const deaths = res[1];
  deathList.set(deaths);
  if (lastSeenCount === -1) {
    lastSeenCount = deaths.length;
    return;
  }
  if (deaths.length > lastSeenCount) {
    lastSeenCount = deaths.length;
    deathToast.set({
      deathTs: deaths[deaths.length - 1],
      expiresAtMs: Date.now() + DEATH_TOAST_TIMEOUT_MS,
    });
  }
}

/** why: the toast's click-through -- pin the triggering death, open the page */
export function openDeathRecap(deathTs: number | null = null): void {
  recapPinned.set(deathTs);
  deathToast.set(null);
  activeModule.set('deathrecap');
}

export function dismissDeathToast(): void {
  deathToast.set(null);
}

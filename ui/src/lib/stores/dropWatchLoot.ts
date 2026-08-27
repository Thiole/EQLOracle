// why: Drop Watch's "you just got one" prompt -- the player's own real
// ask: once a tracked item is actually looted (or auto-routed to
// storage, same real signal, see dropwatch.rs's own doc), offer to
// remove it from the watchlist with a timer; no answer at all means no
// change, it stays tracked. Polled off the same parse-tick every other
// live panel already uses (see tauri/events.ts), not its own interval.
import { get, writable } from 'svelte/store';
import { api } from '../tauri/api';
import { trackedDropItems, trackedDropSeenCounts, setTrackedDropSeenCounts, toggleTrackedDropItem } from './settings';

/** why: long enough to actually notice and read, short enough it
 * doesn't linger as a stale question after you've moved on */
export const LOOT_PROMPT_TIMEOUT_MS = 15_000;

export interface PendingLootPrompt {
  item: string;
  count: number;
  expiresAtMs: number;
}

export const pendingLootPrompts = writable<PendingLootPrompt[]>([]);

/** why: called every parse-tick -- one store pass on the backend for
 * every tracked name at once (see dropwatch::loot_status's own doc),
 * diffed here against each item's own persisted baseline. A count still
 * at or below baseline is old news, already accounted for. */
export async function pollTrackedLoot() {
  const items = get(trackedDropItems);
  if (!items.length) return;
  const rows = await api.getTrackedLootStatus(items).catch(() => []);
  if (!rows.length) return;
  const seen = get(trackedDropSeenCounts);
  const alreadyPending = new Set(get(pendingLootPrompts).map((p) => p.item));
  const fresh = rows.filter((r) => r.count > (seen[r.item] ?? 0) && !alreadyPending.has(r.item));
  if (!fresh.length) return;
  const expiresAtMs = Date.now() + LOOT_PROMPT_TIMEOUT_MS;
  pendingLootPrompts.update((pending) => [
    ...pending,
    ...fresh.map((r) => ({ item: r.item, count: r.count, expiresAtMs })),
  ]);
}

/** why: the only two ways a prompt ever ends -- explicit removal, or
 * anything else (declined, or the timer ran out with no answer at all).
 * Either way this exact count is now accounted for, so the same loot
 * doesn't prompt again; only a further pickup past it will. */
export function resolveLootPrompt(item: string, remove: boolean) {
  const entry = get(pendingLootPrompts).find((p) => p.item === item);
  pendingLootPrompts.update((pending) => pending.filter((p) => p.item !== item));
  if (entry) {
    void setTrackedDropSeenCounts({ ...get(trackedDropSeenCounts), [item]: entry.count });
  }
  if (remove) void toggleTrackedDropItem(item);
}

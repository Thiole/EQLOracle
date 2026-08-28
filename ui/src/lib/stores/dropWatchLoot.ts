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
 * doesn't linger as a stale question after you've moved on. Doubled
 * from 15s -- Spencer's own real experience, 15s wasn't enough */
export const LOOT_PROMPT_TIMEOUT_MS = 30_000;

/** why: real bug -- a fresh app launch backfills the whole log, and a
 * newly-tracked item's `seen` baseline starts at 0, so its real,
 * possibly-days-old total count reads as "fresh" and re-prompts every
 * single relaunch. Gated on the loot's own real timestamp (`last_looted_ms`,
 * a real epoch ms like `Date.now()`, not log-relative) actually being
 * recent -- only a pickup that just happened should ever prompt. */
export const RECENT_LOOT_WINDOW_MS = 30_000;

export interface PendingLootPrompt {
  item: string;
  count: number;
  expiresAtMs: number;
}

export const pendingLootPrompts = writable<PendingLootPrompt[]>([]);

/** why: called every parse-tick -- one store pass on the backend for
 * every tracked name at once (see dropwatch::loot_status's own doc),
 * diffed here against each item's own persisted baseline. A count still
 * at or below baseline is old news, already accounted for. A count
 * *above* baseline still isn't enough on its own though -- a fresh app
 * launch backfills the whole log, and a newly-tracked item's baseline
 * starts at 0, so its real (possibly days-old) total would otherwise
 * read as "fresh" and re-prompt on every single relaunch. Also gated on
 * the loot's own real timestamp being recent -- only a pickup that just
 * happened should ever prompt. */
export async function pollTrackedLoot() {
  const items = get(trackedDropItems);
  if (!items.length) return;
  const rows = await api.getTrackedLootStatus(items).catch(() => []);
  if (!rows.length) return;
  const seen = get(trackedDropSeenCounts);
  const alreadyPending = new Set(get(pendingLootPrompts).map((p) => p.item));
  const fresh = rows.filter(
    (r) =>
      r.count > (seen[r.item] ?? 0) &&
      !alreadyPending.has(r.item) &&
      Date.now() - r.last_looted_ms <= RECENT_LOOT_WINDOW_MS,
  );
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

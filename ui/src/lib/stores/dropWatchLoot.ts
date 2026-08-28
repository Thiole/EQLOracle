// why: Drop Watch's "you just got one" prompt -- the player's own real
// ask: once a tracked item is actually looted (or auto-routed to
// storage, same real signal, see dropwatch.rs's own doc), offer to
// remove it from the watchlist with a timer; no answer at all means no
// change, it stays tracked. Polled off the same parse-tick every other
// live panel already uses (see tauri/events.ts), not its own interval.
import { get, writable } from 'svelte/store';
import { api } from '../tauri/api';
import { status } from './status';
import {
  trackedDropItems,
  trackedDropSeenCounts,
  setTrackedDropSeenCounts,
  toggleTrackedDropItem,
  dropWatchCheckpointMs,
  setDropWatchCheckpointMs,
} from './settings';

/** why: long enough to actually notice and read, short enough it
 * doesn't linger as a stale question after you've moved on. Doubled
 * from 15s -- Spencer's own real experience, 15s wasn't enough */
export const LOOT_PROMPT_TIMEOUT_MS = 30_000;

/** why: real bug in an earlier version -- comparing a loot's own
 * timestamp against a fixed window back from *whenever the app happens
 * to next check* (a flat "within the last 30s of right now") wrongly
 * misses a genuine pickup that happened while the app itself (not the
 * game -- they're separate processes) was briefly closed: reopen even a
 * minute later and a pickup from 45 seconds ago already reads as stale.
 * Fixed with a real checkpoint instead -- "new" means *after the last
 * time this ever checked in*, not "within a fixed window of now",
 * however long that gap actually was. Refreshed roughly every 5
 * minutes while anything's tracked (not on every poll -- Spencer's own
 * call, a few minutes of blind spot on an ungraceful close is a real
 * accepted trade-off, not worth persisting on every single tick), and
 * used directly as the comparison threshold too -- letting it advance
 * mid-session is fine, any genuinely live future pickup is always after
 * whatever the most recent checkpoint is, real time only moves forward --
 * PROVIDED it only ever advances once backfill has actually finished
 * (`status.status.backfilling`, real bug caught before shipping: saving
 * on the very first tick, mid-backfill, would snap the checkpoint to
 * "now" before the backend has finished reporting the true current
 * count/timestamp, silently swallowing a genuine pickup from the "app
 * was closed" gap that a later chunk of backfill hasn't surfaced yet). */
const CHECKPOINT_SAVE_INTERVAL_MS = 5 * 60_000;
let lastCheckpointSaveAt = 0;

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
 * the loot's own real timestamp being past the checkpoint -- see its
 * own doc above -- computed BEFORE this tick's own checkpoint refresh
 * (below), never after: a genuinely live pickup landing in the same
 * tick as a periodic save must never lose that race against its own
 * bookkeeping. `null` (nothing saved yet, a fresh install or an
 * upgrade from before this existed) defaults to "now" -- so first
 * contact with this feature starts clean rather than replaying
 * whatever's already sitting in the log, same spirit as the original bug fix. */
export async function pollTrackedLoot() {
  const items = get(trackedDropItems);
  if (!items.length) return;
  const rows = await api.getTrackedLootStatus(items).catch(() => []);
  if (!rows.length) return;
  const seen = get(trackedDropSeenCounts);
  const alreadyPending = new Set(get(pendingLootPrompts).map((p) => p.item));
  const checkpointMs = get(dropWatchCheckpointMs) ?? Date.now();
  const fresh = rows.filter(
    (r) => r.count > (seen[r.item] ?? 0) && !alreadyPending.has(r.item) && r.last_looted_ms > checkpointMs,
  );
  if (fresh.length) {
    const expiresAtMs = Date.now() + LOOT_PROMPT_TIMEOUT_MS;
    pendingLootPrompts.update((pending) => [
      ...pending,
      ...fresh.map((r) => ({ item: r.item, count: r.count, expiresAtMs })),
    ]);
  }

  const now = Date.now();
  const backfilling = get(status)?.status.backfilling ?? true;
  if (!backfilling && now - lastCheckpointSaveAt >= CHECKPOINT_SAVE_INTERVAL_MS) {
    lastCheckpointSaveAt = now;
    void setDropWatchCheckpointMs(now);
  }
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

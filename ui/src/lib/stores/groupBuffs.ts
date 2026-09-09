// why: the Group Buffs card is a view of this store -- the parse-tick
// refreshes it (tauri/events.ts), no component fetches on its own
import { get, writable } from 'svelte/store';
import { api, type GroupBuffsDto } from '../tauri/api';

export const groupBuffs = writable<GroupBuffsDto | null>(null);

// why: ticks arrive every 100ms -- one call in flight, never a pile-up
let inflight: Promise<void> | null = null;
export function refreshGroupBuffs(): Promise<void> {
  if (inflight) return inflight;
  inflight = api
    .getGroupBuffs()
    .then((d) => groupBuffs.set(d))
    .catch(() => {})
    .finally(() => (inflight = null));
  return inflight;
}

/** why: mount path -- the cached view renders at once, fetch only when empty */
export function ensureGroupBuffs() {
  if (!get(groupBuffs)) void refreshGroupBuffs();
}

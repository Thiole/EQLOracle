// Single source of truth for the toolbar's own connection state -- the
// tail status + line counts. Nothing else in the app holds its own copy
// of this; components read the store, never call `api.getStatus`
// themselves.
import { writable } from 'svelte/store';
import { api, type StatusDto, type TailStatus, type LineCounts } from '../tauri/api';

export const status = writable<StatusDto | null>(null);

export async function refreshStatus() {
  status.set(await api.getStatus());
}

// `parse-tick`'s own payload shape -- applied in place rather than a
// full `refreshStatus()` round trip, since the tick already carries
// everything this store needs.
export function applyStatusTick(tick: { status: TailStatus; counts: LineCounts }) {
  status.update((s) => (s ? { ...s, status: tick.status, counts: tick.counts } : { configured: true, status: tick.status, counts: tick.counts }));
}

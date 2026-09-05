// Single source of truth for the toolbar's own connection state -- the
// tail status + line counts. Nothing else in the app holds its own copy
// of this; components read the store, never call `api.getStatus`
// themselves.
import { writable } from 'svelte/store';
import { api, type StatusDto, type TailStatus, type LineCounts } from '../tauri/api';
import { reloadAfterHistory } from './character';

export const status = writable<StatusDto | null>(null);

export async function refreshStatus() {
  status.set(await api.getStatus());
}

// why: App.svelte's mount call, hardened -- get_status is infallible
// backend-side, so a rejection here is IPC/webview startup flake, and
// one unlucky first call must not leave $status null (a permanently
// blank window with no error and no retry). Keeps trying quietly; the
// first success renders the app.
export async function refreshStatusUntilUp() {
  for (;;) {
    try {
      await refreshStatus();
      return;
    } catch {
      await new Promise((r) => setTimeout(r, 1000));
    }
  }
}

// `parse-tick`'s own payload shape -- applied in place rather than a
// full `refreshStatus()` round trip, since the tick already carries
// everything this store needs.
// why: the warm pass serves the log's tail immediately and the full fold
// swaps in behind it; `"history"` is the tail status while that runs, so
// the edge out of it is the one moment the character view is looking at a
// state that just got replaced under it.
let historyWasFolding = false;

export function applyStatusTick(tick: { status: TailStatus; counts: LineCounts }) {
  const wasFolding = historyWasFolding;
  historyWasFolding = tick.status.tail_status === 'history';
  status.update((s) => (s ? { ...s, status: tick.status, counts: tick.counts } : { configured: true, status: tick.status, counts: tick.counts }));
  if (wasFolding && !historyWasFolding) void reloadAfterHistory();
}

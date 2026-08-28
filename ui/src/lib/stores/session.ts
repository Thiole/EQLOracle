// why: overview.rs's own session-rate stats (plat/hour, xp/hour, motes,
// levels gained, AA spent) -- lived as Overview.svelte's own local state,
// fetched once on mount and never refreshed, unlike every other live
// panel (combat/raiding/chat/dropWatchLoot all refresh off parse-tick).
// A session TRACKER that never updates while you play defeats its own
// purpose, so this is now a real store wired the same way those are.
import { writable } from 'svelte/store';
import { api, type SessionDto } from '../tauri/api';

export const session = writable<SessionDto | null>(null);

export async function refreshSession() {
  session.set(await api.getSession());
}

/** why: Overview Session card's own "restart" button -- backend already
 * returns the freshly-reset DTO, so this is a single round trip, not a
 * reset-then-separate-refetch */
export async function resetSession() {
  session.set(await api.resetSession());
}

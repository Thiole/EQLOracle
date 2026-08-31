// why: single source of truth for the Raiding tab, refreshed on every
// parse-tick so a live boss kill shows up without a manual reload
import { writable } from 'svelte/store';
import { api, type RaidRowDto } from '../tauri/api';

export const raidRows = writable<RaidRowDto[] | null>(null);
export const raidRowsError = writable<string | null>(null);

export async function refreshRaidRows() {
  try {
    // why: ?? [] -- a resolved-but-null payload otherwise leaves the
    // Raiding tab on "Loading…" forever (same stuck-loading shape as
    // the 2026-08-21 field report; null-tolerance at the store boundary,
    // same stance as tradeskill/settings)
    raidRows.set((await api.getRaids()) ?? []);
    raidRowsError.set(null);
  } catch (e) {
    raidRowsError.set(e instanceof Error ? e.message : String(e));
  }
}

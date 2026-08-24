// why: single source of truth for the Raiding tab, refreshed on every
// parse-tick so a live boss kill shows up without a manual reload
import { writable } from 'svelte/store';
import { api, type RaidRowDto } from '../tauri/api';

export const raidRows = writable<RaidRowDto[] | null>(null);
export const raidRowsError = writable<string | null>(null);

export async function refreshRaidRows() {
  try {
    raidRows.set(await api.getRaids());
    raidRowsError.set(null);
  } catch (e) {
    raidRowsError.set(e instanceof Error ? e.message : String(e));
  }
}

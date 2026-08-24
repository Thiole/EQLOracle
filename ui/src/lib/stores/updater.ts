// why: single source of truth for the "an update is available" prompt --
// checked once on launch (App.svelte's onMount), read by UpdateBanner.
import { writable } from 'svelte/store';
import { api, type UpdateInfoDto } from '../tauri/api';

export const availableUpdate = writable<UpdateInfoDto | null>(null);
export const updateCheckError = writable<string | null>(null);
export const installing = writable(false);
export const installError = writable<string | null>(null);

export async function checkForUpdates() {
  try {
    availableUpdate.set(await api.checkForUpdate());
    updateCheckError.set(null);
  } catch (e) {
    // why: a failed check (offline, GitHub unreachable) is not user-facing
    // noise -- silent by default, surfaced only if Settings adds a manual
    // "check now" button later
    updateCheckError.set(e instanceof Error ? e.message : String(e));
  }
}

export function dismissUpdate() {
  availableUpdate.set(null);
}

export async function installUpdate() {
  installing.set(true);
  installError.set(null);
  try {
    await api.installPendingUpdate();
    // why: does not resolve on success -- the process exits first
  } catch (e) {
    installing.set(false);
    installError.set(e instanceof Error ? e.message : String(e));
  }
}

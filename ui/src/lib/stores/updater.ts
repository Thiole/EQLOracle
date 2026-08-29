// why: single source of truth for the "an update is available" prompt --
// checked once on launch (App.svelte's onMount), read by UpdateBanner.
import { writable } from 'svelte/store';
import { api, type UpdateInfoDto } from '../tauri/api';
import { listen } from '../tauri/invoke';

export const availableUpdate = writable<UpdateInfoDto | null>(null);
export const updateCheckError = writable<string | null>(null);
export const installing = writable(false);
export const installError = writable<string | null>(null);
// why: two-step flow -- install swaps the file on disk with the app
// still running, then this flips the banner to "restart when ready".
// Only ever reached on Linux: the Windows installer path exits the
// process inside the plugin itself, so install never resolves there.
export const installed = writable(false);
// why: [received, total] bytes from the backend's update-progress
// events; total null when the server sent no content-length
export const installProgress = writable<[number, number | null] | null>(null);

export async function checkForUpdates() {
  try {
    availableUpdate.set(await api.checkForUpdate());
    updateCheckError.set(null);
    // why: a fresh check is a fresh flow -- a stale "installed" from a
    // previous update must not open the next banner on the restart step
    installed.set(false);
    installProgress.set(null);
  } catch (e) {
    // why: a failed check (offline, GitHub unreachable) is not user-facing
    // noise on the silent launch-time check that also calls this --
    // Settings' own "check for updates" button is what actually surfaces
    // this store's value, gated behind its own justCheckedUpdate so a
    // stale launch-time failure doesn't leak in before the user clicks it
    updateCheckError.set(e instanceof Error ? e.message : String(e));
  }
}

export function dismissUpdate() {
  availableUpdate.set(null);
}

export async function installUpdate() {
  installing.set(true);
  installError.set(null);
  installProgress.set(null);
  // why: subscribed only for the install's own duration -- progress is
  // meaningless outside it, and the banner is the only consumer
  const unlisten = await listen<[number, number | null]>('update-progress', (e) => {
    installProgress.set(e.payload);
  });
  try {
    await api.installPendingUpdate();
    // why: resolving means the file on disk is already the new version
    // (Linux -- the Windows installer exits the process before this).
    // No auto-restart, player's own spec: prompt, restart when ready.
    installed.set(true);
  } catch (e) {
    installError.set(e instanceof Error ? e.message : String(e));
  } finally {
    installing.set(false);
    unlisten();
  }
}

export function restartNow() {
  void api.restartApp();
}

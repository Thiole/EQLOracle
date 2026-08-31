// why: each overlay widget is its own real OS window now (see
// commands::overlay_label's own doc) -- one shared overlay.html bundle,
// differentiated at runtime by which window it actually is. Real mode
// reads the window's own label (set by set_overlay_enabled as
// `overlay-<widget>`); the mock harness has no real Tauri window to
// read a label from, so it uses a `?widget=` query param on overlay.html
// instead -- same contract, different source, so OverlayApp.svelte
// itself never needs to know which mode it's running in. Importing
// @tauri-apps/api/window is safe even in mock mode -- it only touches
// the real Tauri bridge once something on it is actually called, never
// just from being imported.
import { getCurrentWindow } from '@tauri-apps/api/window';
import { isMock } from './invoke';

const OVERLAY_LABEL_PREFIX = 'overlay-';

/** why: null (not "") when neither source names one -- lets callers
 * treat "no widget" as a real, distinct case rather than an empty string
 * silently matching nothing */
export function currentOverlayWidget(): string | null {
  if (isMock) {
    return new URLSearchParams(window.location.search).get('widget');
  }
  const label = getCurrentWindow().label;
  return label.startsWith(OVERLAY_LABEL_PREFIX) ? label.slice(OVERLAY_LABEL_PREFIX.length) : null;
}

/* why: the custom title bar's three window controls (Toolbar.svelte,
 * Windows-frameless only -- see api.getUiShell). Mock mode has no real
 * window to control; guarded no-ops keep the harness clickable without a
 * mock/real branch in the component itself. */
export function minimizeWindow(): void {
  if (isMock) return;
  void getCurrentWindow().minimize();
}

export function toggleMaximizeWindow(): void {
  if (isMock) return;
  void getCurrentWindow().toggleMaximize();
}

export function closeWindow(): void {
  if (isMock) return;
  void getCurrentWindow().close();
}

// why: the Overlay tab's own runtime capability -- fetched once, not
// persisted (it's a fact about this session's display server, not a
// preference). See windowcap.rs's own doc: floating/click-through are
// X11-only, Wayland caps at Docked with a plain-language reason.
import { writable } from 'svelte/store';
import { api, type WindowCapabilityDto } from '../tauri/api';

export const windowCapability = writable<WindowCapabilityDto | null>(null);

export async function loadWindowCapability() {
  windowCapability.set(await api.getWindowCapability());
}

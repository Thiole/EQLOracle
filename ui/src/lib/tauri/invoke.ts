// The one place real-vs-mock is decided -- `api.ts` (and `events.ts`)
// import only from here, never `@tauri-apps/api` directly, so nothing
// downstream needs to know or care which mode it's running in.
import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen as tauriListen, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event';
import { mockInvoke, mockListen } from './mock';

export const isMock = import.meta.env.MODE === 'mock';

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return isMock ? mockInvoke<T>(cmd, args) : tauriInvoke<T>(cmd, args);
}

export async function listen<T>(event: string, callback: EventCallback<T>): Promise<UnlistenFn> {
  if (isMock) {
    return mockListen(event, callback as unknown as Parameters<typeof mockListen>[1]);
  }
  return tauriListen<T>(event, callback);
}

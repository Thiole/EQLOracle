// why: the "what's new" page's state -- opened by App.svelte on the first
// launch after an update (unread sections only), or by the Info panel
// with the whole changelog; "got it" acknowledges the running version.
import { writable } from 'svelte/store';
import { api, type ChangelogSection } from '../tauri/api';

export const whatsNew = writable<{ mode: 'update' | 'all'; sections: ChangelogSection[]; lastSeen: string | null } | null>(null);

export async function checkWhatsNew() {
  try {
    const w = await api.getWhatsNew();
    if (w.sections.length) whatsNew.set({ mode: 'update', sections: w.sections, lastSeen: w.last_seen });
  } catch {
    // why: nothing to show is the normal case; a failed read is not a banner
  }
}

export async function openChangelog() {
  try {
    whatsNew.set({ mode: 'all', sections: await api.getChangelog(), lastSeen: null });
  } catch {
    /* same */
  }
}

export function closeWhatsNew() {
  whatsNew.set(null);
}

export async function ackWhatsNew() {
  try {
    await api.ackWhatsNew();
  } finally {
    whatsNew.set(null);
  }
}

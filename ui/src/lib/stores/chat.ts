// why: single source of truth for the Social tab -- refreshed on every
// parse-tick (see events.ts's own onChatTick) so a new guild/party/raid
// line or PM shows up live, same pattern raiding.ts already uses. Only
// the channel/thread actually open gets refetched, not every one that
// exists -- a PM history can get long, no reason to re-pull threads
// nobody's looking at right now.
import { writable, get } from 'svelte/store';
import { api, type ChatMessageDto, type PmThreadDto } from '../tauri/api';

export type ChatChannelKind = 'guild' | 'party' | 'raid';

const CHANNEL_FETCH: Record<ChatChannelKind, () => Promise<ChatMessageDto[]>> = {
  guild: api.getGuildChat,
  party: api.getPartyChat,
  raid: api.getRaidChat,
};

export const activeChannel = writable<ChatChannelKind>('guild');
export const channelMessages = writable<ChatMessageDto[] | null>(null);
export const channelError = writable<string | null>(null);

export async function refreshActiveChannel() {
  const kind = get(activeChannel);
  try {
    channelMessages.set(await CHANNEL_FETCH[kind]());
    channelError.set(null);
  } catch (e) {
    channelError.set(e instanceof Error ? e.message : String(e));
  }
}

export function setActiveChannel(kind: ChatChannelKind) {
  if (get(activeChannel) === kind) return;
  activeChannel.set(kind);
  channelMessages.set(null);
  void refreshActiveChannel();
}

export const pmThreads = writable<PmThreadDto[] | null>(null);
export const pmThreadsError = writable<string | null>(null);

export async function refreshPmThreads() {
  try {
    pmThreads.set(await api.listPmThreads());
    pmThreadsError.set(null);
  } catch (e) {
    pmThreadsError.set(e instanceof Error ? e.message : String(e));
  }
}

export const activePmPlayer = writable<string | null>(null);
export const pmHistory = writable<ChatMessageDto[] | null>(null);

export async function refreshActivePmHistory() {
  const player = get(activePmPlayer);
  if (!player) return;
  pmHistory.set(await api.getPmHistory(player));
}

export function openPmThread(player: string) {
  if (get(activePmPlayer) === player) return;
  activePmPlayer.set(player);
  pmHistory.set(null);
  void refreshActivePmHistory();
}

/** why: called from events.ts's own parse-tick handler, unconditionally
 * (same as refreshRaidRows) -- keeps data warm across tab switches. */
export function onChatTick() {
  void refreshPmThreads();
  void refreshActiveChannel();
  void refreshActivePmHistory();
}

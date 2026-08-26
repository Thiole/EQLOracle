// why: single source of truth for the Social tab -- refreshed on every
// parse-tick (see events.ts's own onChatTick) so a new guild/party/raid
// line or PM shows up live, same pattern raiding.ts already uses. All 3
// public channels are fetched every tick (cheap, small logs) so the
// Channels list can show a real last-message preview for each one, not
// just whichever's currently selected -- same shape PM's own thread
// list already needed. Only the PM thread actually open gets its full
// history refetched -- a long thread nobody's looking at right now
// doesn't need to be re-pulled every tick.
import { writable, get } from 'svelte/store';
import { api, type ChatMessageDto, type PmThreadDto } from '../tauri/api';

export type ChatChannelKind = 'guild' | 'party' | 'raid';

export const guildMessages = writable<ChatMessageDto[] | null>(null);
export const partyMessages = writable<ChatMessageDto[] | null>(null);
export const raidMessages = writable<ChatMessageDto[] | null>(null);
export const channelError = writable<string | null>(null);

export const activeChannel = writable<ChatChannelKind>('guild');

export function setActiveChannel(kind: ChatChannelKind) {
  activeChannel.set(kind);
}

export async function refreshChannels() {
  try {
    const [guild, party, raid] = await Promise.all([api.getGuildChat(), api.getPartyChat(), api.getRaidChat()]);
    guildMessages.set(guild);
    partyMessages.set(party);
    raidMessages.set(raid);
    channelError.set(null);
  } catch (e) {
    channelError.set(e instanceof Error ? e.message : String(e));
  }
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
  void refreshChannels();
  void refreshPmThreads();
  void refreshActivePmHistory();
}

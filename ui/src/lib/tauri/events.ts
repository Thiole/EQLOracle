// why: only place listen() is called; idempotent against double-mount
import { listen } from './invoke';
import type { TailStatus, LineCounts } from './api';
import { applyStatusTick } from '../stores/status';
import { onCombatTick } from '../stores/combat';
import { refreshLastLocation, refreshZoneContext } from '../stores/maps';
import { refreshRaidRows } from '../stores/raiding';
import { onChatTick } from '../stores/chat';
import { pollTrackedLoot } from '../stores/dropWatchLoot';
import { pollDeaths } from '../stores/deathRecap';
import { refreshSession } from '../stores/session';
import { get } from 'svelte/store';
import { activeModule } from '../stores/shell';
import { loadCharacterModule } from '../stores/character';
import { refreshGroupBuffs } from '../stores/groupBuffs';

interface RecentLine {
  kind: string;
  rule_id: string;
  text: string;
}

interface ParseTick {
  status: TailStatus;
  counts: LineCounts;
  recent: RecentLine[];
}

let initialized = false;
// why: the Character card and the Overview's own queries load on mount and
// never again, so a long backfill (or EQLP_REPLAY_UNTIL, which mounts the
// window while replaying) left them showing a mid-replay moment. Refresh
// them once, when the parse stops backfilling.
let wasBackfilling = false;

// why: a store refreshes when a line it depends on was parsed, not on
// every tick -- '*' is any parsed line; a quiet tick costs nothing
type Trigger = { kinds: Set<string> | null; fn: () => void };
const triggers: Trigger[] = [{ kinds: null, fn: () => void refreshGroupBuffs() }];
export function refreshOn(kinds: string[] | '*', fn: () => void): () => void {
  const t: Trigger = { kinds: kinds === '*' ? null : new Set(kinds), fn };
  triggers.push(t);
  return () => {
    const i = triggers.indexOf(t);
    if (i >= 0) triggers.splice(i, 1);
  };
}
function fireTriggers(recent: RecentLine[]) {
  if (!recent.length) return;
  const seen = new Set(recent.map((r) => r.kind));
  for (const t of triggers) if (!t.kinds || [...t.kinds].some((k) => seen.has(k))) t.fn();
}

/// why: one coalesced pass instead of one per tick. `all` is the settle
/// case, which refreshes modules that are not on screen too.
const FAN_OUT_MS = 350;
let fanTimer: ReturnType<typeof setTimeout> | undefined;
let pendingRecent: RecentLine[] = [];

function fanOut(all: boolean) {
  const recent = pendingRecent;
  pendingRecent = [];
  const mod = get(activeModule);
  void onCombatTick();
  void refreshLastLocation();
  void refreshZoneContext();
  void pollTrackedLoot();
  void pollDeaths();
  void refreshSession();
  // why: these two feed one module each and both re-load on mount, so
  // fetching them while the user is elsewhere buys nothing -- chat alone
  // is four calls that serialize the whole session's messages
  if (all || mod === 'endgame') void refreshRaidRows();
  if (all || mod === 'social') onChatTick();
  fireTriggers(recent);
}

export async function initTauriEvents() {
  if (initialized) return;
  initialized = true;

  await listen<ParseTick>('parse-tick', (e) => {
    // why: always -- this is the progress badge itself, and it invokes nothing
    applyStatusTick(e.payload);
    const backfilling = e.payload.status.backfilling;
    // why: every one of these is an IPC round trip that takes the ingest
    // lock, and a backfill tick fires one per parsed chunk while the
    // worker is holding that same lock. Reported real on Windows, where
    // the IPC handler shares the window's own thread: the window could
    // not be dragged until the replay finished. Nothing below is worth
    // showing mid-replay anyway -- the settle path re-runs all of it
    // once, against the finished state.
    if (!backfilling) {
      // why: a tick fires up to ten times a second while the log grows,
      // and this fan-out is ~18 IPC round trips, each taking the same
      // ingest lock the parser holds. Coalesced to one pass per window:
      // the numbers on screen are a running fight, not an animation, and
      // the overlay widgets keep their own tick listener.
      for (const r of e.payload.recent) pendingRecent.push(r);
      if (fanTimer === undefined) {
        fanTimer = setTimeout(() => {
          fanTimer = undefined;
          fanOut(false);
        }, FAN_OUT_MS);
      }
    }
    if (wasBackfilling && !backfilling) {
      void loadCharacterModule();
      // why: the settle refresh -- everything, ungated, so a replay that
      // ends with a fight open still lands on the real state
      if (fanTimer !== undefined) {
        clearTimeout(fanTimer);
        fanTimer = undefined;
      }
      fanOut(true);
      for (const t of triggers) t.fn();
      window.dispatchEvent(new CustomEvent('eqlp:parse-settled'));
    }
    wasBackfilling = backfilling;
  });

  await listen<string>('parse-error', (e) => {
    console.error('parse-error', e.payload);
  });
}

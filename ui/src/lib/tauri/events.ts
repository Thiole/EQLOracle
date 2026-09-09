// why: only place listen() is called; idempotent against double-mount
import { listen } from './invoke';
import type { TailStatus, LineCounts } from './api';
import { applyStatusTick } from '../stores/status';
import { onCombatTick } from '../stores/combat';
import { onInventoryDumpDetected } from '../stores/character';
import { refreshLastLocation, refreshZoneContext } from '../stores/maps';
import { refreshRaidRows } from '../stores/raiding';
import { onChatTick } from '../stores/chat';
import { pollTrackedLoot } from '../stores/dropWatchLoot';
import { pollDeaths } from '../stores/deathRecap';
import { refreshSession } from '../stores/session';
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
export function refreshOn(kinds: string[] | '*', fn: () => void) {
  triggers.push({ kinds: kinds === '*' ? null : new Set(kinds), fn });
}
function fireTriggers(recent: RecentLine[]) {
  if (!recent.length) return;
  const seen = new Set(recent.map((r) => r.kind));
  for (const t of triggers) if (!t.kinds || [...t.kinds].some((k) => seen.has(k))) t.fn();
}

export async function initTauriEvents() {
  if (initialized) return;
  initialized = true;

  await listen<ParseTick>('parse-tick', (e) => {
    applyStatusTick(e.payload);
    void onCombatTick();
    void refreshLastLocation();
    void refreshZoneContext();
    void refreshRaidRows();
    onChatTick();
    void pollTrackedLoot();
    void pollDeaths();
    void refreshSession();
    // why: a backfill chunk overflows the recent window -- one full
    // refresh when it settles, kind-triggers only while live
    if (!e.payload.status.backfilling) fireTriggers(e.payload.recent);
    if (wasBackfilling && !e.payload.status.backfilling) {
      void loadCharacterModule();
      for (const t of triggers) t.fn();
      window.dispatchEvent(new CustomEvent('eqlp:parse-settled'));
    }
    wasBackfilling = e.payload.status.backfilling;
  });

  await listen<string>('parse-error', (e) => {
    console.error('parse-error', e.payload);
  });

  await listen<{ file: string; character: string | null }>('inventory-dump', (e) => {
    onInventoryDumpDetected(e.payload.file, e.payload.character);
  });
}

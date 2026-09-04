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
    if (wasBackfilling && !e.payload.status.backfilling) {
      void loadCharacterModule();
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

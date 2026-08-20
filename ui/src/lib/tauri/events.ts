// why: only place listen() is called; idempotent against double-mount
import { listen } from './invoke';
import type { TailStatus, LineCounts } from './api';
import { applyStatusTick } from '../stores/status';
import { onCombatTick } from '../stores/combat';
import { onInventoryDumpDetected } from '../stores/character';
import { refreshLastLocation, refreshZoneContext } from '../stores/maps';

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

export async function initTauriEvents() {
  if (initialized) return;
  initialized = true;

  await listen<ParseTick>('parse-tick', (e) => {
    applyStatusTick(e.payload);
    void onCombatTick();
    void refreshLastLocation();
    void refreshZoneContext();
  });

  await listen<string>('parse-error', (e) => {
    console.error('parse-error', e.payload);
  });

  await listen<{ file: string; character: string | null }>('inventory-dump', (e) => {
    onInventoryDumpDetected(e.payload.file, e.payload.character);
  });
}

// The mock IPC harness (docs/ci.md, ui/tests/README.md): `invoke()` in
// mock mode returns real, deterministic backend output -- computed by
// the actual Ingest/combat pipeline against fixtures/reference-slice.log
// via `cargo run -p eqlp-app --example dump_fixtures`, not hand-typed --
// looked up here by (command, args) rather than recomputed in JS. See
// that example's own doc for exactly what's covered and why hand-typed
// fixtures would be the wrong call (every backend formula fix would need
// a parallel, driftable JS reimplementation).
//
// Not exhaustive over every command/argument combination -- covers the
// Combat module's own real usage so far. A command or argument
// combination with no fixture logs a warning and returns `null`, loudly,
// rather than silently pretending to have data.

import fixtures from '../../../tests/fixtures/reference-slice.json';

type FixtureTable = Record<string, Record<string, unknown>>;
const data = fixtures as unknown as FixtureTable;

// `undefined`/`null` both mean "no filter" on the Rust side (an absent
// `Option`) and both serialize the same way through the dump tool's own
// key format -- see `crates/app/examples/dump_fixtures.rs`.
function norm(v: unknown): string {
  if (v === undefined || v === null) return 'null';
  if (Array.isArray(v)) return v.join(',');
  return String(v);
}

// why: a Record<string,string> arg (an exaltation assignment map) has no
// stable String() form -- sorted "k:v,k:v" gives one, matched exactly by
// dump_fixtures.rs's own key-building for the same commands.
function normMap(v: unknown): string {
  if (!v || typeof v !== 'object') return '';
  return Object.entries(v as Record<string, string>)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([k, val]) => `${k}:${val}`)
    .join(',');
}

// Mirrors `dump_fixtures.rs`'s own per-command key shape exactly -- these
// two files must be changed together. Not a generic serialization of
// `args` on purpose: a generic "every key, sorted" scheme would silently
// break the moment a command's real call sites start passing an argument
// inconsistently (present here, omitted there) that this file was never
// updated to match.
function keyFor(cmd: string, args: Record<string, unknown> | undefined): string {
  const a = args ?? {};
  switch (cmd) {
    case 'get_status':
    case 'list_zone_visits':
      return '';
    case 'list_encounters':
      return `zoneVisit=${norm(a.zoneVisit)}&offset=${norm(a.offset)}&limit=${norm(a.limit)}`;
    case 'get_combat_summary':
    case 'list_allies':
      return `zoneVisit=${norm(a.zoneVisit)}&encounterId=${norm(a.encounterId)}`;
    case 'get_fight_timeline':
      return `encounterId=${norm(a.encounterId)}`;
    case 'get_fight_state_at':
      return `encounterId=${norm(a.encounterId)}&tsMs=${norm(a.tsMs)}`;
    case 'get_class_configurations':
    case 'get_default_gear_classes':
      return `name=${norm(a.name)}`;
    case 'get_current_level':
    case 'get_aa_log':
    case 'get_spellbook':
    case 'list_aa':
      return '';
    case 'get_character_estimate':
      return `race=${norm(a.race)}&classes=${norm(a.classes)}&classLevels=${norm(a.classLevels)}`;
    case 'get_gear_recommendations':
      return `classes=${norm(a.classes)}&race=${norm(a.race)}&level=${norm(a.level)}`;
    case 'get_gear_weights':
      return `classes=${norm(a.classes)}&level=${norm(a.level)}`;
    case 'get_mob_history':
    case 'get_loadout_summary':
      return `target=${norm(a.target)}&confirmedOnly=${norm(a.confirmedOnly)}`;
    case 'get_inventory_dump':
      return `file=${norm(a.file)}`;
    case 'get_item_at_tier':
      return `id=${norm(a.id)}&tier=${norm(a.tier)}`;
    case 'get_item_with_exalts':
      return `id=${norm(a.id)}&tier=${norm(a.tier)}&exalts=${normMap(a.exalts)}`;
    case 'get_exalt_candidates':
      return `id=${norm(a.id)}&socketKey=${norm(a.socketKey)}&other=${normMap(a.otherAssignments)}&classes=${norm(a.classes)}&maxEra=${norm(a.maxEra)}`;
    case 'find_existing_inventory_dump':
    case 'list_map_packs':
    case 'list_all_map_zones':
    case 'get_last_location':
    case 'get_zone_context':
      return '';
    case 'list_map_zones':
      return `pack=${norm(a.pack)}`;
    case 'list_zone_versions':
      return `zone=${norm(a.zone)}`;
    case 'get_map_file':
      return `pack=${norm(a.pack)}&zone=${norm(a.zone)}`;
    case 'list_npc_zone_candidates':
      return `mapZoneName=${norm(a.mapZoneName)}`;
    case 'get_npc_markers_for_zone':
      return `zone=${norm(a.zone)}`;
    case 'list_debug_encounters':
      return `limit=${norm(a.limit)}`;
    case 'get_unmatched_coverage':
      return `top=${norm(a.top)}`;
    case 'get_configuration_zone_visits':
      return `name=${norm(a.name)}&classes=${norm(a.classes)}&levelRange=${norm(a.levelRange)}`;
    case 'list_zones':
    case 'list_npcs':
    case 'list_spells':
    case 'list_spell_effects':
    case 'get_mob_aliases':
      return '';
    case 'list_gear_items':
      return `classes=${norm(a.classes)}&slot=${norm(a.slot)}&maxEra=${norm(a.maxEra)}`;
    case 'get_item_loot_history':
      return `item=${norm(a.item)}`;
    case 'get_mob_stats':
      return `mobName=${norm(a.mobName)}`;
    case 'get_pm_history':
      return `player=${norm(a.player)}`;
    case 'list_zone_encounters':
      return `zoneId=${norm(a.zoneId)}&limit=${norm(a.limit)}`;
    case 'list_mob_encounters':
      return `mobName=${norm(a.mobName)}&limit=${norm(a.limit)}`;
    case 'get_encounter_detail':
      return `encounterId=${norm(a.encounterId)}`;
    case 'get_era_options':
    case 'get_preferences':
      return '';
    default:
      return '';
  }
}

export async function mockInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  // why: not fixture data -- a platform fact. True here so the harness
  // exercises the custom title bar (drag region, window controls); the
  // control clicks themselves no-op via window.ts's mock guards.
  if (cmd === 'get_ui_shell') {
    return { custom_titlebar: true } as T;
  }
  const table = data[cmd];
  if (!table) {
    console.warn(`[mock] no fixture table for command "${cmd}" -- returning null`);
    return null as T;
  }
  const key = keyFor(cmd, args);
  if (!(key in table)) {
    console.warn(`[mock] no fixture for "${cmd}" key "${key}" -- have: [${Object.keys(table).join(', ')}]`);
    return null as T;
  }
  return table[key] as T;
}

// No live event stream in mock mode -- fixtures are a static snapshot,
// "no wall clock, no live log, no waiting" (docs/design/sources.md's own
// replay-determinism stance). A test that needs to exercise a live
// `parse-tick` update calls `mockEmit` itself rather than this file
// simulating one on a timer.
//
// Wraps payloads in the same `{ event, id, payload }` shape
// `@tauri-apps/api/event`'s real `Event<T>` uses, so a callback written
// against the real API (`(e) => e.payload...`) behaves identically here
// -- a callback that only worked in one mode would be exactly the kind
// of mock/real drift this harness exists to prevent.
type MockListener = (event: { event: string; id: number; payload: unknown }) => void;
const listeners = new Map<string, Set<MockListener>>();
let nextId = 1;

export function mockListen(event: string, callback: MockListener): () => void {
  if (!listeners.has(event)) listeners.set(event, new Set());
  listeners.get(event)!.add(callback);
  return () => listeners.get(event)?.delete(callback);
}

export function mockEmit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((cb) => cb({ event, id: nextId++, payload }));
}

// why: lets Playwright fire a real event without importing this module
if (import.meta.env.MODE === 'mock' && typeof window !== 'undefined') {
  (window as unknown as { __mockEmit: typeof mockEmit }).__mockEmit = mockEmit;
}

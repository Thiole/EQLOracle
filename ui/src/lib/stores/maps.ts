// why: single source of truth for the Maps module's zone selection and
// the currently-loaded 3D geometry -- a plain load-on-select store, not
// eagerly loaded like Game Data's static catalogs, since a single zone's
// map file can be tens of thousands of line segments and there's no
// reason to fetch every zone's geometry before the user has picked one.
//
// Zone-first, not pack-first: the picker used to make the user choose a
// map pack before they could even see whether their zone had a map at
// all. Now every zone with a map anywhere (base game or any pack) is one
// flat list; picking a zone that's covered by more than one source (e.g.
// Befallen: base game + Brewall) surfaces an "available versions" toggle
// instead.
import { writable, get } from 'svelte/store';
import { api, type MapFileDto, type LastLocationDto, type NpcMarkerDto, type ZoneContextDto, type ZoneDto, type ZoneRouteDto } from '../tauri/api';
import { mapShortnames } from '../maps/zoneMatch';
import { activeModule } from './shell';

/** why: Settings' "N packs known" display only -- the zone picker itself
 * no longer asks the user to choose a pack up front, see `mapZones`. */
export const mapPacks = writable<string[]>([]);
export const mapZones = writable<string[]>([]);
export const selectedZone = writable<string | null>(null);
/** why: which source(s) have a map for the selected zone -- `null` = base
 * game, else a pack name. Populated by `selectZone`. */
export const zoneVersions = writable<(string | null)[]>([]);
/** why: which version is currently loaded -- `null` = base game. */
export const selectedVersion = writable<string | null>(null);
export const currentMap = writable<MapFileDto | null>(null);
export const mapLoading = writable(false);
export const mapError = writable<string | null>(null);

/** why: the "you are here" marker -- a snapshot, not live tracking, see
 * LastLocationDto's own doc. Polled on module mount and on each real
 * parse-tick while the module is open, same as the Combat module's own
 * live-refresh pattern -- not a new mechanism. */
export const lastLocation = writable<LastLocationDto | null>(null);

/** why: current + previous zone labels -- feeds MapViewer's entrance
 * guess (a `to_<previous>` marker) for the stretch after zoning in but
 * before the player has typed a real `/loc`. Polled the same way as
 * `lastLocation`, same reasoning. */
export const zoneContext = writable<ZoneContextDto | null>(null);

/** why: the "live: follow me" checkbox in Maps.svelte -- when on, a real
 * zone change automatically switches the viewer to that zone's map, via
 * `resolveMapZone` (real wiki-sourced resolution first, `learnedZoneMap`
 * fallback). On by default -- asked for: the map is mostly opened to see
 * where you are, and the checkbox was easy to miss off. Session-only. */
export const liveFollow = writable(true);

/** why: fallback only now -- `resolveMapZone`'s real resolution (the
 * backend's wiki-sourced `who_name` shortname per zone) covers most real
 * zones; this only still matters for the residual few with no wiki match
 * or no `who_name` recorded at all. Learns from the strongest signal
 * available for those: the user's own explicit zone selection.
 * Session-only (module-level, not a store, not persisted) -- resets on
 * restart same as everything else zone-visit-scoped in this app. */
const learnedZoneMap = new Map<string, string>();

/** why: real resolution first -- `ctx.current_map_zones` (the backend's
 * wiki-sourced shortname(s) for the current raw zone, see ZoneContextDto's
 * own doc), intersected with `mapZones` (zones this app actually has a
 * map file for, since a shortname can be real but for an expansion this
 * game's data doesn't cover) -- falling back to `learnedZoneMap` only
 * when that comes up empty (no wiki match, or no covered shortname). */
function resolveMapZone(ctx: ZoneContextDto | null): string | undefined {
  if (!ctx?.current) return undefined;
  const known = get(mapZones);
  const real = ctx.current_map_zones.find((z) => known.includes(z));
  if (real) return real;
  return learnedZoneMap.get(ctx.current);
}

export function setLiveFollow(on: boolean) {
  liveFollow.set(on);
  if (!on) return;
  const stem = resolveMapZone(get(zoneContext));
  if (stem && stem !== get(selectedZone)) void selectZone(stem);
}

// ------------------------------------------------------------- navigation

/** why: the GPS-style destination -- a real `ZoneDto.name`, not a map
 * shortname. Persists across zone switches (unlike a one-off route query
 * would) so the route recomputes automatically as the player actually
 * progresses, rather than going stale the moment they take the first
 * step. `null` means no active navigation. */
export const navigationTarget = writable<string | null>(null);
/** why: the route as of the player's *current* zone -- recomputed fresh
 * every real zone change (see `recomputeActiveRoute`), so `hops[0]` is
 * always "the next step from here," not a stale leg from wherever the
 * route was first requested. `'loading'`/`'error'` are real, distinct
 * states shown to the user, not folded into `null`. */
export const activeRoute = writable<ZoneRouteDto | 'loading' | 'error' | null>(null);

/** why: `find_zone_route` needs real `ZoneDto.name` strings, but this
 * module's own zone list (`mapZones`/`selectedZone`) is keyed by map-file
 * shortname -- fetched once and cached at module scope (not a store: no
 * view needs to react to it changing, it doesn't change after first
 * load). */
let zoneCatalog: ZoneDto[] | null = null;
async function ensureZoneCatalog(): Promise<ZoneDto[]> {
  if (!zoneCatalog) zoneCatalog = (await api.listZones()) ?? [];
  return zoneCatalog;
}

/** why: resolves a map-file shortname to the real `ZoneDto.name`
 * `find_zone_route` needs, by matching against each zone's own
 * `mapShortnames(who_name)` -- shared by the auto-recompute below and
 * Maps.svelte's own "current zone" display, so they can never disagree. */
export async function zoneNameForShortname(shortname: string): Promise<string | null> {
  const zones = await ensureZoneCatalog();
  const z = zones.find((z) => z.who_name && mapShortnames(z.who_name).some((s) => s.toLowerCase() === shortname.toLowerCase()));
  return z?.name ?? null;
}

/** why: guards `recomputeActiveRoute` against re-querying on every
 * `parse-tick` when nothing about the route actually changed -- only a
 * real "current zone" change (or a fresh `setNavigationTarget` call)
 * should trigger a new `find_zone_route` call. */
let lastRoutedFrom: string | null = null;

/** why: an in-zone endpoint for the navigation -- a real entity ("go to
 * Gorgalosk"), not just a zone. Rides along with `navigationTarget`:
 * cross-zone hops come from the route as usual, and once the loaded map
 * is the poi's own zone, Maps.svelte draws the final walk leg to these
 * coordinates instead of stopping at "you're already here". `zone` is
 * the raw wiki zone -- the same value the npc-overlay candidate bridge
 * matches maps against. Coordinates are `/loc`-space, transformed at
 * draw time like every other position. */
export interface NavigationPoi {
  name: string;
  zone: string;
  x: number;
  y: number;
  z: number | null;
}
export const navigationPoi = writable<NavigationPoi | null>(null);

/** why: called by `Maps.svelte`'s destination field -- sets (or clears,
 * passing `null`) the persistent target and immediately computes the
 * first route from wherever the player currently is. A plain zone pick
 * clears any entity poi -- the poi only ever arrives together with its
 * own zone via `navigateToNpc`. */
export async function setNavigationTarget(zoneName: string | null, poi: NavigationPoi | null = null) {
  navigationPoi.set(poi);
  navigationTarget.set(zoneName);
  activeRoute.set(null);
  lastRoutedFrom = null;
  await recomputeActiveRoute();
}

/** why: the "click the information, set a path" entry point -- called
 * from an NPC's info page. Same mechanism as picking a destination zone
 * (route recomputes on every real zone change), plus the entity's own
 * spawn point as the final walk leg; jumps to the Maps module so the
 * result is visible, same reasoning gdOpenPage gives for switching. */
export async function navigateToNpc(routeZone: string, poi: NavigationPoi) {
  activeModule.set('maps');
  await setNavigationTarget(routeZone, poi);
}

/** why: the actual GPS behavior -- called here on every real zone change
 * (see `refreshZoneContext`) and once from `setNavigationTarget`. Always
 * routes from the player's *current* resolved zone, so the returned
 * `hops[0]` is always the honest next step, not a leg from the original
 * query point that's now behind them. */
export async function recomputeActiveRoute() {
  const target = get(navigationTarget);
  if (!target) {
    activeRoute.set(null);
    lastRoutedFrom = null;
    return;
  }
  const shortname = resolveMapZone(get(zoneContext));
  if (!shortname) return; // no known current zone yet -- leave whatever's showing
  const fromName = await zoneNameForShortname(shortname);
  if (!fromName) {
    activeRoute.set('error');
    return;
  }
  if (fromName === lastRoutedFrom) return; // already routed from here, real change needed to re-query
  lastRoutedFrom = fromName;
  if (fromName === target) {
    activeRoute.set({ hops: [], total_distance: 0 });
    return;
  }
  activeRoute.set('loading');
  try {
    activeRoute.set(await api.findZoneRoute(fromName, target));
  } catch {
    activeRoute.set('error');
  }
}

/** why: fuzzy candidates for the *currently selected* map zone -- see
 * npcdata::candidate_zones' own doc for why this can't be exact. The user
 * toggles each one on/off and judges correctness visually (do the
 * plotted markers land inside the walls), not this app. */
export const npcZoneCandidates = writable<string[]>([]);
/** why: which candidates are currently toggled on -- a Set, since more
 * than one can be enabled at once (e.g. both halves of a zone that got
 * renamed mid-scrape). */
export const enabledNpcZones = writable<Set<string>>(new Set());
/** why: merged markers from every currently-enabled candidate zone --
 * recomputed whenever the enabled set changes, not per-zone state the
 * viewer would have to merge itself. */
export const npcMarkers = writable<NpcMarkerDto[]>([]);
/** why: the current zone's navmesh poly outlines (map-file coords), for
 * MapViewer's translucent floor overlay; null until cached or when none */
export const navMesh = writable<[number, number, number][][] | null>(null);

let zonesLoaded = false;

/** why: fetched once per module-mount rather than on every render, same
 * as Game Data's static catalogs -- but unlike those, the folder on disk
 * genuinely *can* change mid-session (a user drops in a new map pack
 * while the app is still running), so this is a cache to short-circuit
 * repeat mounts, not a "this can never change" assumption -- see
 * `rescanMapFolder` for the escape hatch Settings exposes. */
export async function loadMapModule() {
  if (zonesLoaded) return;
  zonesLoaded = true;
  mapPacks.set((await api.listMapPacks()) ?? []); // defensive -- invoke<T>()'s type is an assertion, not a guarantee
  mapZones.set((await api.listAllMapZones()) ?? []);
  void refreshLastLocation();
  void refreshZoneContext();
}

/** why: Settings' "rescan maps folder" button -- the backend itself never
 * caches (`mapsdata::load_zone_map` and friends always read straight off
 * disk), so the only stale thing to fix is this store's own one-time
 * zone-list fetch. Re-lists packs and zones, and -- if a zone is open in
 * the viewer right now -- re-selects it, in case the user updated an
 * existing zone's file rather than adding a whole new pack. */
export async function rescanMapFolder() {
  mapPacks.set((await api.listMapPacks()) ?? []);
  mapZones.set((await api.listAllMapZones()) ?? []);
  const zone = get(selectedZone);
  if (zone) await selectZone(zone);
}

/** why: real per-zone marker cache, keyed by which candidate zone they
 * came from -- not a store itself (nothing outside toggleNpcZone/
 * selectZone needs to know the *breakdown*, only the flat merged result
 * `npcMarkers` exposes), but keeping it keyed is what lets toggling one
 * zone off drop only its own markers without touching any other
 * currently-enabled zone's. */
const npcMarkersByZone = new Map<string, NpcMarkerDto[]>();

function recomputeNpcMarkers() {
  npcMarkers.set([...npcMarkersByZone.values()].flat());
}

/** why: which version's map is actually fetched for `zone` -- prefers
 * whatever the user already had open (so hopping between two zones that
 * both have a Brewall version keeps showing Brewall, not silently
 * dropping back to base each time), falling back to the first available
 * version (base game sorts first when present, see
 * `mapsdata::list_zone_versions`). */
function pickVersion(versions: (string | null)[]): string | null {
  const preferred = get(selectedVersion);
  return versions.includes(preferred) ? preferred : (versions[0] ?? null);
}

export async function selectZone(zone: string) {
  selectedZone.set(zone);
  // why: fire-and-forget nav/collision fetch for this zone -- see
  // api.ensureEmuZone. Walk paths route over the mesh once cached;
  // until then the line-grid fallback answers.
  // why: the navmesh overlay -- fetched once the zone's mesh is cached
  // (ensureEmuZone downloads it on first open); null clears the overlay
  // while a different zone loads
  navMesh.set(null);
  void api
    .ensureEmuZone(zone)
    .catch(() => ({ nav: false, geo: false }))
    .then(() => api.getNavMesh(zone))
    .then((m) => {
      if (get(selectedZone) === zone) navMesh.set(m ?? null);
    })
    .catch(() => {});
  // why: the user picking a zone map while a specific raw zone label is
  // current is the strongest possible training signal for `learnedZoneMap`
  // -- not a guess, their own explicit action -- see that map's own doc.
  const raw = get(zoneContext)?.current;
  if (raw) learnedZoneMap.set(raw, zone);
  mapLoading.set(true);
  mapError.set(null);
  // why: a fresh zone invalidates whatever NPC overlay was on for the
  // *previous* zone -- carrying it over would plot one zone's mobs on a
  // different zone's walls.
  enabledNpcZones.set(new Set());
  npcMarkersByZone.clear();
  recomputeNpcMarkers();
  try {
    const versions = (await api.listZoneVersions(zone)) ?? [];
    const version = pickVersion(versions);
    zoneVersions.set(versions);
    selectedVersion.set(version);
    const [map, candidates] = await Promise.all([
      api.getMapFile(version, zone),
      api.listNpcZoneCandidates(zone),
    ]);
    currentMap.set(map);
    npcZoneCandidates.set(candidates ?? []);
  } catch (e) {
    mapError.set(String(e));
    currentMap.set(null);
  } finally {
    mapLoading.set(false);
  }
}

/** why: the version toggle in Maps.svelte -- switches which source's
 * rendition of the *same already-selected* zone is loaded. Doesn't touch
 * NPC overlay state (zone-scoped, not version-scoped) or re-fetch
 * candidates -- only the geometry itself differs between versions. */
export async function selectVersion(version: string | null) {
  const zone = get(selectedZone);
  if (!zone) return;
  selectedVersion.set(version);
  mapLoading.set(true);
  mapError.set(null);
  try {
    currentMap.set(await api.getMapFile(version, zone));
  } catch (e) {
    mapError.set(String(e));
    currentMap.set(null);
  } finally {
    mapLoading.set(false);
  }
}

/** why: the toggle chips in Maps.svelte -- turning a candidate zone on
 * fetches its real NPC markers and merges them in; turning it off drops
 * just that zone's markers, leaving any other enabled candidate alone. */
export async function toggleNpcZone(zone: string) {
  const enabled = new Set(get(enabledNpcZones));
  if (enabled.has(zone)) {
    enabled.delete(zone);
    enabledNpcZones.set(enabled);
    npcMarkersByZone.delete(zone);
    recomputeNpcMarkers();
    return;
  }
  enabled.add(zone);
  enabledNpcZones.set(enabled);
  npcMarkersByZone.set(zone, (await api.getNpcMarkersForZone(zone)) ?? []);
  recomputeNpcMarkers();
}

export async function refreshLastLocation() {
  lastLocation.set(await api.getLastLocation());
}

export async function refreshZoneContext() {
  const prevRaw = get(zoneContext)?.current ?? null;
  const ctx = await api.getZoneContext();
  zoneContext.set(ctx);
  // why: GPS-style navigation recomputes on every real zone change,
  // independent of "live: follow me" (that toggle only controls whether
  // the *viewed map* auto-switches, not whether an active route stays
  // current) -- see `recomputeActiveRoute`'s own doc.
  if (get(navigationTarget) && ctx?.current !== prevRaw) void recomputeActiveRoute();
  // why: only act on a genuine zone *change* -- not every tick's re-fetch
  // of the same current zone.
  if (!get(liveFollow) || !ctx?.current || ctx.current === prevRaw) return;
  const stem = resolveMapZone(ctx);
  if (stem && stem !== get(selectedZone)) void selectZone(stem);
}

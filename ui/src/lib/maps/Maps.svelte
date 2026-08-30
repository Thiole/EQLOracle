<script lang="ts">
  // why: a standalone zone picker over the game folder's own maps/ files,
  // not cross-referenced against zonedata::zones() -- see the plan's own
  // "scope decision" note: there's no clean existing shortname table to
  // reuse, and building one well is separate work from the viewer itself.
  import { Input } from '$lib/components/ui/input';
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Checkbox } from '$lib/components/ui/checkbox';
  import SearchIcon from '@lucide/svelte/icons/search';
  import NavigationIcon from '@lucide/svelte/icons/navigation';
  import XIcon from '@lucide/svelte/icons/x';
  import MapViewer from './MapViewer.svelte';
  import { zoneMatches, looksLikeEntranceFor } from './zoneMatch';
  import { api, type MapLineDto, type MapMarkerDto, type PathDto, type ZoneDto } from '$lib/tauri/api';
  import { displayZoneName } from '$lib/utils';
  import {
    mapZones,
    selectedZone,
    zoneVersions,
    selectedVersion,
    currentMap,
    mapLoading,
    mapError,
    loadMapModule,
    selectZone,
    selectVersion,
    zoneContext,
    lastLocation,
    npcZoneCandidates,
    enabledNpcZones,
    npcMarkers,
    toggleNpcZone,
    liveFollow,
    setLiveFollow,
    navigationTarget,
    navigationPoi,
    activeRoute,
    setNavigationTarget,
  } from '$lib/stores/maps';

  $effect(() => {
    void loadMapModule();
  });

  let search = $state('');
  const q = $derived(search.trim().toLowerCase());
  const filteredZones = $derived($mapZones.filter((z) => !q || z.toLowerCase().includes(q)));

  /** why: real files are internal shortnames ("befallen") -- title-cased
   * for display, the underlying value stays the real filename stem so
   * selectZone/getMapFile keep working unchanged. */
  function displayName(zone: string): string {
    return zone.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase());
  }

  function versionLabel(v: string | null): string {
    return v ?? 'Base game';
  }

  // why: closes the loop on the "is it a data bug or a display bug"
  // question -- computed from the *actual* loaded `$currentMap.lines`
  // (the exact geometry the 3D view is drawing right now), not a separate
  // offline recomputation that could quietly diverge from what the app
  // itself is using.
  function distPointSeg(p: [number, number, number], a: [number, number, number], b: [number, number, number]): number {
    const ab: [number, number, number] = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    const ap: [number, number, number] = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    const ab2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    const t = ab2 === 0 ? 0 : Math.max(0, Math.min(1, (ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / ab2));
    const c: [number, number, number] = [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]];
    return Math.hypot(p[0] - c[0], p[1] - c[1], p[2] - c[2]);
  }

  function nearestWallDistance(lines: MapLineDto[], p: [number, number, number]): number | null {
    let best: number | null = null;
    for (const l of lines) {
      const d = distPointSeg(p, l.a, l.b);
      if (best === null || d < best) best = d;
    }
    return best;
  }

  // why: `/loc`'s (x, y) map to this format's own (-y, -x), z unchanged --
  // see MapViewer.svelte's own doc for how that was found (brute-forced
  // against real readings, not guessed). Applied here too so this debug
  // number means the same thing as what's actually rendered, not a stale
  // "no transform" comparison that would silently disagree with the map.
  const nearestWall = $derived(
    $lastLocation && $currentMap
      ? nearestWallDistance($currentMap.lines, [-$lastLocation.y, -$lastLocation.x, $lastLocation.z])
      : null,
  );

  // why: the confirmed-/loc debug line above only ever reports the *last
  // typed* /loc's own frozen context -- it correctly stays showing an old
  // zone until you type a fresh one there, which reads as "stuck" even
  // though it's working as designed. This is the other half of the
  // picture: the *live* entrance-guess pipeline (the yellow marker; a
  // teleport landing is shown red/confirmed instead, see
  // MapViewer.svelte's own doc), recomputed the same way
  // MapViewer.svelte's own `placeHereMesh` does
  // (same imported functions, not reimplemented) so it can't silently
  // disagree with what's actually on screen.
  const entranceGuess = $derived.by((): string => {
    const ctx = $zoneContext;
    if (!$selectedZone) return 'no zone selected';
    if (!ctx?.current) return 'no live zone context yet';
    if (!zoneMatches(ctx.current_map_zones, ctx.current, $selectedZone)) {
      return `not your current zone -- you're in "${ctx.current}", this map is "${$selectedZone}"`;
    }
    const markers = $currentMap?.markers ?? [];
    if (ctx.teleport_landing) {
      const { class: cls, x, y, z } = ctx.teleport_landing;
      const label = cls === 'any' ? 'Origin' : `${cls} teleport`;
      return `${label} landing -- confirmed destination (${x}, ${y}, ${z})`;
    }
    if (!ctx.previous) return 'no previous zone known yet (first zone this session?)';
    const candidates = markers.filter((m) => looksLikeEntranceFor(m.label, ctx.previous!));
    if (candidates.length === 1) return `entered from "${ctx.previous}" -- using "${candidates[0].label}"`;
    if (candidates.length === 0) return `entered from "${ctx.previous}" -- no matching "to_" marker found on this map`;
    return `entered from "${ctx.previous}" -- ambiguous, ${candidates.length} candidates: ${candidates.map((m) => m.label).join(', ')}`;
  });

  // ---------------------------------------------------------- pathfinding

  /** why: `find_zone_route` takes real `ZoneDto.name` strings ("Northern
   * Plains of Karana"), but this module's own zone list is keyed by real
   * map-file shortname ("northkarana") -- fetched once, not re-derived
   * per query. `null` while loading; the route UI below degrades to
   * "unavailable" rather than erroring if this never resolves (a real,
   * possible outcome -- not every install has the wiki zone-guide pack
   * lined up with every map shortname, see `zonedata.rs`'s own doc on the
   * 2 known real gaps). */
  let allZones = $state<ZoneDto[] | null>(null);
  $effect(() => {
    api.listZones().then((z) => (allZones = z));
  });

  /** why: is the loaded map actually the zone the player is standing in
   * right now -- the same "is the map I have open my real current zone"
   * check `entranceGuess` above already uses, reused here since GPS
   * next-step guidance is meaningless on a map the player is just
   * browsing, not physically in. */
  const viewingLiveZone = $derived(
    !!$zoneContext?.current && !!$selectedZone && zoneMatches($zoneContext.current_map_zones, $zoneContext.current, $selectedZone),
  );

  let destinationSearch = $state('');
  const destinationCandidates = $derived.by((): ZoneDto[] => {
    const q = destinationSearch.trim().toLowerCase();
    if (!q || !allZones) return [];
    return allZones.filter((z) => z.name.toLowerCase().includes(q)).slice(0, 8);
  });

  function pickDestination(toZone: string) {
    destinationSearch = '';
    void setNavigationTarget(toZone);
  }

  function hopLabel(kind: 'walk' | 'teleport' | 'succor', spell: string | null): string {
    if (kind === 'walk') return 'walk';
    if (kind === 'succor') return 'Succor/Difficulty Change';
    return `cast "${spell}"`;
  }

  /** why: the same "confirmed" tier the "you are here" marker and
   * `entranceGuess` above already use (see docs/design/maps.md's "You are
   * here" — the location-estimation ladder"), and the same freshest-wins
   * comparison `MapViewer.svelte`'s `placeHereMesh` applies -- this used
   * to check `/loc` first unconditionally and only fall to a teleport
   * landing when no `/loc` existed for the zone at all, which kept a walk
   * path starting from a stale pre-teleport `/loc` even after a fresher,
   * exact teleport landing was known (reported directly: "you are using
   * the prev location as a you are here"). Landing evidence fills in the
   * start position rather than being an alternate display of it. Falls to
   * `$currentMap.markers[0]` only when neither source matches the loaded
   * zone at all. */
  function walkStartPosition(): [number, number, number] | null {
    const loc = $lastLocation;
    const locMatches = loc && $selectedZone && zoneMatches(loc.map_zones, loc.zone, $selectedZone);
    const ctx = $zoneContext;
    const landingMatches = ctx?.teleport_landing && $selectedZone && zoneMatches(ctx.current_map_zones, ctx.current, $selectedZone);

    const preferLoc = !landingMatches || (locMatches && (ctx!.teleport_landing_ts == null || loc!.ts_ms >= ctx!.teleport_landing_ts));

    if (locMatches && preferLoc) {
      // `/loc`-space -> map-file space, same transform MapViewer.svelte
      // applies to a real /loc reading before plotting it.
      return [-loc.y, -loc.x, loc.z];
    }
    if (landingMatches) {
      // Same `/loc`-space -> map-file transform as above -- teleport_landing
      // is confirmed to live in the same coordinate space (see
      // MapViewer.svelte's own doc on the independent cross-check against
      // the Brewall map pack).
      const { x, y, z } = ctx!.teleport_landing!;
      return [-y, -x, z];
    }
    const first = $currentMap?.markers[0];
    return first ? [first.pos[0], first.pos[1], first.pos[2]] : null;
  }

  // ---- manual walk path: click a marker on the loaded map to draw a
  // real walking route to it -- always available, independent of GPS
  // navigation (a quick "how far is that thing" query doesn't need a
  // destination zone set at all).
  let walkMode = $state(false);
  let manualWalkPath = $state<PathDto | null>(null);
  let manualWalkError = $state<string | null>(null);

  async function onMarkerClicked(marker: MapMarkerDto) {
    if (!walkMode || !$selectedZone) return;
    const from = walkStartPosition();
    if (!from) {
      manualWalkError = 'no known starting position in this zone yet';
      return;
    }
    manualWalkPath = null;
    manualWalkError = null;
    try {
      manualWalkPath = await api.findWalkPath($selectedVersion, $selectedZone, from, marker.pos);
    } catch (e) {
      manualWalkError = e instanceof Error ? e.message : String(e);
    }
  }

  // ---- GPS next-step path: when navigation is active, the loaded map is
  // the zone the player is actually standing in, and `$activeRoute`'s own
  // first hop is a walk, auto-draw the route to that hop's exit marker --
  // "when the zone is up it'll show the path to next step", the user's
  // own ask. `$activeRoute` is always freshly recomputed from the
  // player's *current* zone (see stores/maps.ts's `recomputeActiveRoute`),
  // so `hops[0]` is always the honest next step, never a stale leg.
  let gpsWalkPath = $state<PathDto | null>(null);
  let gpsError = $state<string | null>(null);
  $effect(() => {
    const route = $activeRoute;
    const map = $currentMap;
    gpsWalkPath = null;
    gpsError = null;
    if (!viewingLiveZone || !map || !route || route === 'loading' || route === 'error' || route.hops.length === 0) return;
    const next = route.hops[0];
    if (next.kind !== 'walk') return; // a teleport/succor step has no in-zone path to draw -- the hop list itself says what to do
    const candidates = map.markers.filter((m) => looksLikeEntranceFor(m.label, next.zone));
    if (candidates.length !== 1) return;
    const from = walkStartPosition();
    if (!from || !$selectedZone) return;
    api
      .findWalkPath($selectedVersion, $selectedZone, from, candidates[0].pos)
      .then((p) => (gpsWalkPath = p))
      .catch((e) => (gpsError = e instanceof Error ? e.message : String(e)));
  });

  // ---- entity poi path: navigation set from an NPC's info page carries
  // the entity's own spawn point -- once the loaded map is the poi's
  // zone (same wiki-zone bridge the npc overlay uses), draw the final
  // walk leg to it instead of stopping at zone arrival. Takes precedence
  // over the GPS exit-hop path: on the destination map, the leg to the
  // entity IS the next step.
  let poiWalkPath = $state<PathDto | null>(null);
  let poiError = $state<string | null>(null);
  $effect(() => {
    const poi = $navigationPoi;
    const map = $currentMap;
    poiWalkPath = null;
    poiError = null;
    if (!poi || !map || !$selectedZone) return;
    if (!$npcZoneCandidates.includes(poi.zone)) return;
    const from = walkStartPosition();
    if (!from) return;
    // Same `/loc`-space -> map-file transform every other position gets.
    api
      .findWalkPath($selectedVersion, $selectedZone, from, [-poi.y, -poi.x, poi.z ?? 0])
      .then((p) => (poiWalkPath = p))
      .catch((e) => (poiError = e instanceof Error ? e.message : String(e)));
  });

  const displayedPath = $derived(poiWalkPath ?? gpsWalkPath ?? manualWalkPath);

  // A walk-path drawn on one zone's map is meaningless on another -- clear
  // the moment the loaded zone changes, rather than leaving a stale line
  // lingering from whatever was open before.
  $effect(() => {
    void $selectedZone;
    manualWalkPath = null;
    manualWalkError = null;
  });
</script>

<div class="flex flex-col gap-3 p-3">
  <div class="flex items-center gap-3">
    <Input bind:value={search} placeholder="filter zones…" class="h-7 w-56 text-[12px]" />
    <label class="flex items-center gap-1.5 text-[11px] text-muted-foreground" title="Switches to your current zone's map automatically when you zone -- only once you've picked that zone's map manually at least once this session.">
      <Checkbox checked={$liveFollow} onCheckedChange={(v: boolean) => setLiveFollow(v)} />
      live: follow me
    </label>
  </div>

  <div class="flex gap-3">
    <Card class="h-[520px] w-56 shrink-0 overflow-y-auto rounded-sm">
      <CardContent class="px-2 py-2">
        {#if filteredZones.length === 0}
          <p class="p-2 text-[11px] text-muted-foreground">No zones match.</p>
        {:else}
          <ul class="flex flex-col">
            {#each filteredZones as z (z)}
              <li>
                <button
                  type="button"
                  class="w-full cursor-pointer rounded-sm px-2 py-1 text-left text-[12px] hover:bg-muted/40 {$selectedZone === z ? 'bg-primary/15 font-medium text-primary' : ''}"
                  onclick={() => selectZone(z)}
                >
                  {displayName(z)}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </CardContent>
    </Card>

    <Card class="flex h-[520px] flex-1 flex-col rounded-sm">
      {#if $selectedZone && ($zoneVersions.length > 1 || $npcZoneCandidates.length > 0)}
        <div class="flex flex-col gap-1 border-b border-border px-2 py-1.5">
          {#if $zoneVersions.length > 1}
            <!-- why: this zone has more than one rendering (e.g. base
                 game + a community pack) -- pick which one to load,
                 replacing the old up-front "pick a pack" step. -->
            <div class="flex flex-wrap items-center gap-1.5">
              <span class="text-[10px] uppercase tracking-wide text-muted-foreground">version</span>
              {#each $zoneVersions as v (v ?? '')}
                {@const on = $selectedVersion === v}
                <button
                  type="button"
                  class="cursor-pointer rounded-full border px-2 py-0.5 text-[11px] transition-colors {on
                    ? 'border-primary bg-primary/15 text-primary'
                    : 'border-border text-muted-foreground hover:bg-muted/40'}"
                  onclick={() => selectVersion(v)}
                >
                  {versionLabel(v)}
                </button>
              {/each}
            </div>
          {/if}
          {#if $npcZoneCandidates.length > 0}
            <!-- why: fuzzy matches only -- the user is the correctness
                 check, not this app. See stores/maps.ts's
                 npcZoneCandidates doc. -->
            <div class="flex flex-wrap items-center gap-1.5">
              <span class="text-[10px] uppercase tracking-wide text-muted-foreground">npc overlay</span>
              {#each $npcZoneCandidates as z (z)}
                {@const on = $enabledNpcZones.has(z)}
                <button
                  type="button"
                  class="cursor-pointer rounded-full border px-2 py-0.5 text-[11px] transition-colors {on
                    ? 'border-primary bg-primary/15 text-primary'
                    : 'border-border text-muted-foreground hover:bg-muted/40'}"
                  onclick={() => toggleNpcZone(z)}
                >
                  {z}
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
      {#if $selectedZone}
        <!-- why: two pathfinding entry points, kept visually and
             functionally separate (never blended into one "route" concept)
             -- in-zone walking (click a marker on the loaded map) and GPS
             navigation (a persistent destination, recomputed every real
             zone change) answer genuinely different questions, and a
             route can legitimately mix walk and teleport hops that a
             single control couldn't represent honestly. See
             docs/design/maps.md's "Pathfinding" section. -->
        <div class="flex flex-wrap items-center gap-3 border-b border-border px-2 py-1.5">
          <label class="flex items-center gap-1.5 text-[11px] text-muted-foreground" title="Click a marker on the map to draw a real walking route to it, starting from your last known position in this zone.">
            <Checkbox checked={walkMode} onCheckedChange={(v: boolean) => { walkMode = v; manualWalkPath = null; manualWalkError = null; }} />
            walk mode: click a marker
          </label>
          {#if manualWalkError}<span class="text-[11px] text-bad">{manualWalkError}</span>{/if}
          {#if manualWalkPath}<span class="text-[11px] text-muted-foreground">route drawn -- {manualWalkPath.waypoints.length} waypoints</span>{/if}
        </div>

        <!-- why: GPS destination gets its own visually distinct block --
             a tinted, bordered section rather than one more item crammed
             into the thin toolbar strip above, since it's the primary way
             a player actually drives this module (walk mode is a
             secondary, click-to-probe tool). Search input and its results
             are stacked, not wrapped inline together, so picking a result
             doesn't mean hunting for it next to whatever else happened to
             wrap onto that line. -->
        <div class="border-b border-border bg-muted/30 px-3 py-2.5">
          <div class="flex items-center gap-2">
            <NavigationIcon class="size-3.5 shrink-0 text-muted-foreground" />
            <span class="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">destination</span>
            {#if $navigationTarget}
              <span class="flex items-center gap-1.5 rounded-full border border-primary bg-primary/15 py-1 pl-2.5 pr-1.5 text-[12px] font-medium text-primary">
                {displayZoneName($navigationTarget)}{#if $navigationPoi}<span class="text-primary/80">&rarr; {$navigationPoi.name}</span>{/if}
                <button
                  type="button"
                  class="cursor-pointer rounded-full p-0.5 text-primary/70 hover:bg-primary/20 hover:text-primary"
                  onclick={() => setNavigationTarget(null)}
                  aria-label="clear destination"
                >
                  <XIcon class="size-3" />
                </button>
              </span>
            {:else}
              <div class="relative max-w-xs flex-1">
                <SearchIcon class="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
                <Input bind:value={destinationSearch} placeholder="where do you want to go?" class="h-8 pl-7 text-[12px]" />
              </div>
            {/if}
          </div>
          {#if destinationSearch && destinationCandidates.length > 0}
            <div class="mt-2 flex max-w-xs flex-col gap-0.5 rounded-md border border-border bg-background p-1 shadow-sm">
              {#each destinationCandidates as z (z.id)}
                <button
                  type="button"
                  class="cursor-pointer rounded px-2 py-1 text-left text-[12px] text-foreground transition-colors hover:bg-primary/10 hover:text-primary"
                  onclick={() => pickDestination(z.name)}
                >
                  {displayZoneName(z.name)}
                </button>
              {/each}
            </div>
          {/if}
        </div>
        {#if $navigationTarget}
          {#if $activeRoute === 'loading'}
            <p class="border-b border-border px-2 py-1.5 text-[11px] text-muted-foreground">finding a route…</p>
          {:else if $activeRoute === 'error'}
            <p class="border-b border-border px-2 py-1.5 text-[11px] text-bad">couldn't find a route to {displayZoneName($navigationTarget)}.</p>
          {:else if $activeRoute && $activeRoute.hops.length === 0}
            <p class="border-b border-border px-2 py-1.5 text-[11px] text-muted-foreground">
              {#if $navigationPoi}you're in the zone -- walking leg to {$navigationPoi.name} below.{:else}you're already here.{/if}
            </p>
          {:else if $activeRoute}
            <!-- why: each hop's kind is shown plainly, never blended into a
                 generic "shortcut" -- a teleport hop names its own spell so
                 the reader can judge whether they actually have access to
                 it, the same reasoning RouteHopDto's own doc gives for why
                 the backend doesn't (and can't) know that. The first hop
                 is bolded -- it's the *next step*, recomputed fresh from
                 wherever the player currently is (see stores/maps.ts's
                 `recomputeActiveRoute`), not just the first leg of
                 whatever route was originally requested. -->
            <ol class="flex flex-wrap items-center gap-1 border-b border-border px-2 py-1.5 text-[11px] text-muted-foreground">
              {#each $activeRoute.hops as hop, i (i)}
                <li>
                  <span class="{hop.kind === 'teleport' ? 'text-primary' : hop.kind === 'succor' ? 'text-brand-soft' : ''} {i === 0 ? 'font-medium text-foreground' : ''}">{hopLabel(hop.kind, hop.via_spell)}</span>
                  ({hop.distance.toFixed(0)})
                </li>
                <li>&rarr;</li>
                <li class={i === $activeRoute.hops.length - 1 ? 'font-medium text-foreground' : ''}>{displayZoneName(hop.zone)}</li>
                {#if i < $activeRoute.hops.length - 1}<li>&rarr;</li>{/if}
              {/each}
              <li class="ml-2">total: {$activeRoute.total_distance.toFixed(0)}</li>
            </ol>
          {/if}
          {#if poiError}
            <p class="border-b border-border px-2 py-1.5 text-[11px] text-bad">couldn't draw the path to {$navigationPoi?.name}: {poiError}</p>
          {:else if poiWalkPath}
            <p class="border-b border-border px-2 py-1.5 text-[11px] text-muted-foreground">
              path to {$navigationPoi?.name} drawn on the map (green) -- {poiWalkPath.waypoints.length} waypoints.
            </p>
          {:else if gpsError}
            <p class="border-b border-border px-2 py-1.5 text-[11px] text-bad">couldn't draw the next step: {gpsError}</p>
          {:else if gpsWalkPath}
            <p class="border-b border-border px-2 py-1.5 text-[11px] text-muted-foreground">next-step path drawn on the map (green).</p>
          {:else if $activeRoute && $activeRoute !== 'loading' && $activeRoute !== 'error' && $activeRoute.hops.length > 0 && !viewingLiveZone}
            <p class="border-b border-border px-2 py-1.5 text-[11px] text-muted-foreground">open your current zone's map to see the next-step path drawn.</p>
          {/if}
        {/if}
      {/if}
      <CardContent class="flex flex-1 items-center justify-center p-0">
        {#if !$selectedZone}
          <p class="text-[12px] text-muted-foreground">
            Pick a zone to view its map.
            {#if $liveFollow && $zoneContext?.current}
              <br />Live follow won't switch here on its own yet -- pick your current zone once
              and it'll remember it for the rest of the session.
            {/if}
          </p>
        {:else if $mapLoading}
          <p class="text-[12px] text-muted-foreground">Loading…</p>
        {:else if $mapError}
          <p class="text-[12px] text-bad">{$mapError}</p>
        {:else if $currentMap}
          <MapViewer
            map={$currentMap}
            zone={$selectedZone}
            npcMarkers={$npcMarkers}
            zoneContext={$zoneContext}
            path={displayedPath}
            onMarkerClick={walkMode ? onMarkerClicked : null}
          />
        {/if}
      </CardContent>
    </Card>
  </div>

  {#if $selectedZone}
    <!-- why: debugging aid -- why the "you are here" dot isn't showing is
         otherwise invisible from outside MapViewer.svelte's own internal
         state. Same `zoneMatches` call the marker itself uses (imported
         from the same file, not reimplemented), so this can never disagree
         with what actually decided the dot's fate. -->
    <p class="font-mono text-[10px] text-muted-foreground/70">
      your location:
      {#if $lastLocation}
        zone="{$lastLocation.zone ?? '(none)'}" ({$lastLocation.x.toFixed(1)}, {$lastLocation.y.toFixed(1)}, {$lastLocation.z.toFixed(1)}) ·
        resolved map_zones=[{$lastLocation.map_zones.join(', ') || 'none'}] · loaded map="{$selectedZone}" ({versionLabel($selectedVersion)}) ·
        match={zoneMatches($lastLocation.map_zones, $lastLocation.zone, $selectedZone)} ·
        nearest wall in loaded map data: {nearestWall === null ? 'n/a' : `${nearestWall.toFixed(1)} units`}
      {:else}
        no /loc reading yet this session (type /loc in-game)
      {/if}
    </p>
    <p class="font-mono text-[10px] text-muted-foreground/70">entrance guess: {entranceGuess}</p>
  {/if}
</div>

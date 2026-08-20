<script lang="ts">
  // why: a standalone zone picker over the game folder's own maps/ files,
  // not cross-referenced against zonedata::zones() -- see the plan's own
  // "scope decision" note: there's no clean existing shortname table to
  // reuse, and building one well is separate work from the viewer itself.
  import { Input } from '$lib/components/ui/input';
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Checkbox } from '$lib/components/ui/checkbox';
  import MapViewer from './MapViewer.svelte';
  import { zoneMatches, looksLikeEntranceFor } from './zoneMatch';
  import type { MapLineDto } from '$lib/tauri/api';
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
      return `${cls} teleport landing -- confirmed spell destination (${x}, ${y}, ${z})`;
    }
    if (!ctx.previous) return 'no previous zone known yet (first zone this session?)';
    const candidates = markers.filter((m) => looksLikeEntranceFor(m.label, ctx.previous!));
    if (candidates.length === 1) return `entered from "${ctx.previous}" -- using "${candidates[0].label}"`;
    if (candidates.length === 0) return `entered from "${ctx.previous}" -- no matching "to_" marker found on this map`;
    return `entered from "${ctx.previous}" -- ambiguous, ${candidates.length} candidates: ${candidates.map((m) => m.label).join(', ')}`;
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
          <MapViewer map={$currentMap} zone={$selectedZone} npcMarkers={$npcMarkers} zoneContext={$zoneContext} />
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

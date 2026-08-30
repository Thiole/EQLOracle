<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import NavigationIcon from '@lucide/svelte/icons/navigation';
  import { api, type NpcDto, type MobStatsDto, type NpcNavPointDto } from '$lib/tauri/api';
  import { navigateToNpc } from '$lib/stores/maps';
  import { wikiLinkText } from './wikitext';
  import GdField from './GdField.svelte';
  import GdLink from './GdLink.svelte';
  import EncounterHistory from './EncounterHistory.svelte';

  let { npc }: { npc: NpcDto } = $props();

  const raid = $derived(npc.categories.includes('Raid Encounters'));
  const raceLabel = $derived(npc.race ? wikiLinkText(npc.race) : null);
  const locationParts = $derived(
    (npc.location ?? '')
      .split(',')
      .map((p) => wikiLinkText(p.trim()))
      .filter(Boolean),
  );

  let stats = $state<MobStatsDto | null>(null);
  let token = 0;
  $effect(() => {
    const name = npc.name;
    stats = null;
    const my = ++token;
    api.getMobStats(name).then((s) => {
      if (my === token) stats = s;
    });
  });

  // why: "click the information, set a path" -- the player's own ask.
  // Spawn points come from the same wiki coordinates the map's npc
  // overlay plots; empty (multi-zone NPC, or no parseable coordinates)
  // means no button, nothing to route to.
  let navPoints = $state<NpcNavPointDto[]>([]);
  $effect(() => {
    const name = npc.name;
    navPoints = [];
    api.getNpcNavPoints(name).then((p) => {
      if (name === npc.name) navPoints = p ?? [];
    });
  });

  function setPathHere() {
    const p = navPoints[0];
    if (!p) return;
    void navigateToNpc(p.route_zone ?? p.zone, { name: npc.name, zone: p.zone, x: p.x, y: p.y, z: p.z });
  }
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h2 class="stat-figure text-[18px]">
      {npc.name}
      {#if raid}<span class="ml-1 rounded-full border border-caution/40 bg-caution/10 px-1.5 py-0.5 align-middle text-[10px] text-caution uppercase"
          >raid</span
        >{/if}
    </h2>

    <div class="mt-2 flex flex-col">
      <GdField label="Race" value={raceLabel} />
      <GdField label="Class" value={npc.class} />
      <GdField label="Level" value={npc.level} />
      {#if npc.zone}
        <GdField label="Zone">{#snippet value()}<GdLink kind="zone" name={npc.zone ?? ''} />{/snippet}</GdField>
      {/if}
      {#if locationParts.length}
        <GdField label="Location">
          {#snippet value()}{#each locationParts as p, i (p + i)}{#if i > 0}<span>, </span>{/if}<GdLink kind="zone" name={p} />{/each}{/snippet}
        </GdField>
      {/if}
      <GdField label="Respawn timer" value={npc.respawn_time} />
      <GdField label="AC" value={npc.AC} />
      <GdField label="HP" value={npc.HP?.toLocaleString()} />
      <GdField label="Special" value={npc.special} />
    </div>

    <div class="mt-3 flex items-center gap-3">
      {#if navPoints.length > 0}
        <!-- why: sets the Maps module's GPS destination to this NPC's own
             spawn point, exactly the way picking a destination zone does,
             plus the final in-zone walk leg -- see stores/maps.ts's
             navigateToNpc. First spawn point wins for a multi-spawn mob;
             the title says so rather than hiding it. -->
        <button
          type="button"
          class="inline-flex cursor-pointer items-center gap-1 rounded-full border border-primary bg-primary/10 px-2 py-0.5 text-[11px] font-medium text-primary hover:bg-primary/20"
          title={navPoints.length > 1
            ? `Set as navigation destination (${navPoints.length} known spawn points -- routing to the first)`
            : 'Set as navigation destination'}
          onclick={setPathHere}
        >
          <NavigationIcon class="size-3" />
          set path here
        </button>
      {/if}
      <a class="text-[11px] text-brand-soft hover:text-primary hover:underline" href={npc.url} target="_blank" rel="noopener">
        eqlwiki ↗ (backup / full page)
      </a>
    </div>
  </CardContent>
</Card>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h3 class="panel-title mb-1">known loot</h3>
    {#if !npc.known_loot.length}
      <p class="text-[11px] text-muted-foreground">The wiki records no known drop table for this NPC.</p>
    {:else}
      <table class="w-full text-[11px]">
        <thead>
          <tr class="text-left text-muted-foreground"><th class="pb-1 font-normal">item</th><th class="pb-1 font-normal">rarity</th></tr>
        </thead>
        <tbody>
          {#each npc.known_loot as l (l.item)}
            <tr><td class="py-0.5"><GdLink kind="item" name={l.item} /></td><td class="py-0.5 text-muted-foreground">{l.rarity ?? '—'}</td></tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </CardContent>
</Card>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h3 class="panel-title mb-1">your history with this mob</h3>
    {#if !stats}
      <p class="text-[11px] text-muted-foreground">Loading…</p>
    {:else if stats.pulls === 0}
      <p class="text-[11px] text-muted-foreground">Not fought yet this session.</p>
    {:else}
      <p class="mb-1.5 text-[11px]">
        {stats.kills.toLocaleString()} confirmed kill{stats.kills === 1 ? '' : 's'} across {stats.pulls.toLocaleString()} pull{stats.pulls === 1
          ? ''
          : 's'}.
      </p>
      <EncounterHistory kind="npc" id={npc.name} showZone={true} />
    {/if}
  </CardContent>
</Card>

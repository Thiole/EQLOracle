<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { npcs, type GdKind } from '$lib/stores/gamedata';
  import type { ZoneDto } from '$lib/tauri/api';
  import { displayZoneName } from '$lib/utils';
  import GdField from './GdField.svelte';
  import GdLinkList from './GdLinkList.svelte';
  import EncounterHistory from './EncounterHistory.svelte';

  let { zone }: { zone: ZoneDto } = $props();

  // why: the wiki's own curated "Notable NPCs" list on a zone page is
  // already a subset of the full bestiary (npcdata.rs only ever scrapes
  // "Named Mobs" pages) -- this cross-checks the *other* direction, every
  // NPC in the full catalog whose own `zone` field names this zone, so an
  // ordinary trash mob the zone page never called out can still show up.
  const mobsHere = $derived(
    [...new Set($npcs.filter((n) => n.zone?.toLowerCase() === zone.name.toLowerCase()).map((n) => n.name))].sort((a, b) =>
      a.localeCompare(b),
    ),
  );
  const kindNpc: GdKind = 'npc';
  const kindItem: GdKind = 'item';
  const kindZone: GdKind = 'zone';
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h2 class="stat-figure text-[18px]">{displayZoneName(zone.name)}</h2>

    <div class="mt-2 flex flex-col">
      <GdField label="Zone ID">
        {#snippet value()}<code class="text-[10px] text-muted-foreground">{zone.id}</code>{/snippet}
      </GdField>
      <GdField label="Level range" value={zone.level_range} />
      <GdField label="Monster types" value={zone.monster_types} />
      {#if zone.notable_npcs.length}
        <GdField label="Notable NPCs">
          {#snippet value()}<GdLinkList kind={kindNpc} names={zone.notable_npcs} />{/snippet}
        </GdField>
      {/if}
      {#if zone.unique_items.length}
        <GdField label="Unique items">
          {#snippet value()}<GdLinkList kind={kindItem} names={zone.unique_items} />{/snippet}
        </GdField>
      {/if}
      <GdField label="Related quests" value={zone.related_quests.join(', ')} />
      <GdField label="Guilds" value={zone.guilds.join(', ')} />
      <GdField label="Tradeskill facilities" value={zone.tradeskill_facilities.join(', ')} />
      <GdField label="City races" value={zone.city_races.join(', ')} />
      {#if zone.adjacent_zones.length}
        <GdField label="Adjacent zones">
          {#snippet value()}<GdLinkList kind={kindZone} names={zone.adjacent_zones} />{/snippet}
        </GdField>
      {/if}
      <GdField label="Spawn timer" value={zone.spawn_timer} />
      <GdField label="Succor/Difficulty Change" value={zone.succor_evacuate} />
      <GdField label="/who name" value={zone.who_name} />
    </div>

    <a class="mt-3 inline-block text-[11px] text-brand-soft hover:text-primary hover:underline" href={zone.url} target="_blank" rel="noopener">
      eqlwiki ↗ (backup / full page)
    </a>
  </CardContent>
</Card>

{#if mobsHere.length}
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h3 class="panel-title mb-1">mobs found here</h3>
      <p class="text-[11px]"><GdLinkList kind={kindNpc} names={mobsHere} /></p>
    </CardContent>
  </Card>
{/if}

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h3 class="panel-title mb-1">your parsed encounters here</h3>
    <EncounterHistory kind="zone" id={zone.id} showZone={false} />
  </CardContent>
</Card>

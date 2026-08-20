<script lang="ts">
  // why: an item's own `zones` list is really "drop-source strings" -- the
  // scrape's drop parser reads "* [[mob]]" bullets as mobs and any other
  // linked line as a new zone, which breaks for a "Dropped By" list that
  // names a raid boss directly with no bullet wrapping it (the boss ends
  // up in `zones` with an empty `mobs`, even though it's the actual
  // encounter). Try zone first, then NPC, before falling back to plain
  // text -- mirrors the legacy planner's own `gdZoneOrMobLink`.
  import { gdFind, gdOpenPage } from '$lib/stores/gamedata';

  let { name }: { name: string } = $props();
  const zone = $derived(gdFind('zone', name));
  const npc = $derived(!zone ? gdFind('npc', name) : undefined);
  const raid = $derived(!!npc?.categories.includes('Raid Encounters'));
</script>

{#if zone}
  <button type="button" class="text-brand-soft hover:text-primary hover:underline" onclick={() => gdOpenPage('zone', name)}>{name}</button>
{:else if npc}
  <button type="button" class="text-brand-soft hover:text-primary hover:underline" onclick={() => gdOpenPage('npc', name)}>{name}</button>
  {#if raid}<span class="ml-1 rounded-full border border-caution/40 bg-caution/10 px-1.5 py-0.5 text-[9px] text-caution uppercase">raid</span>{/if}
{:else}
  <span>{name}</span>
{/if}

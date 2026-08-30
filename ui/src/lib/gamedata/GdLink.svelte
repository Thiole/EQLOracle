<script lang="ts">
  // why: a cross-reference name becomes a real in-app link only when that
  // category's own catalog has a match -- otherwise it's just text, not a
  // dead link dressed up to look clickable.
  import { gdFind, gdOpenPage, type GdKind } from '$lib/stores/gamedata';
  import { displayZoneName } from '$lib/utils';
  import BellIcon from '@lucide/svelte/icons/bell';
  import { trackedDropItems, toggleTrackedDropItem } from '$lib/stores/settings';

  // why: `bell` -- every dynamically-placed item name is a Drop Watch
  // entry point (player's ask: "mouse over it and click notification
  // icon that shows up after hovering"). Default on for items; the Sky
  // tabs pass false because their chips carry their own always-visible
  // bell in a dedicated slot. Runes excluded same as everywhere else --
  // not mob drops, nothing Drop Watch could match.
  let { kind, name, bell = true }: { kind: GdKind; name: string; bell?: boolean } = $props();
  const found = $derived(!!gdFind(kind, name));
  // why: lookup/navigation always uses the real `name` -- only the
  // rendered text is display-cleaned, and only for zones (see
  // `displayZoneName`'s own doc for why this can't apply to every kind).
  const label = $derived(kind === 'zone' ? displayZoneName(name) : name);
  const bellable = $derived(bell && kind === 'item' && !name.startsWith('Wind Rune '));
  const tracked = $derived(bellable && $trackedDropItems.includes(name));
</script>

{#snippet nameEl()}
  {#if found}
    <button type="button" class="text-brand-soft hover:text-primary hover:underline" onclick={() => gdOpenPage(kind, name)}>{label}</button>
  {:else}
    <span>{label}</span>
  {/if}
{/snippet}

{#if bellable}
  <!-- why: two presentations of one toggle -- a TRACKED item shows a
       small persistent filled bell (state must be visible without
       hunting), an untracked one reveals the add-bell only on hover, as
       an absolute overlay so nothing in the surrounding text reflows
       (the anti-jumping rule from the Sky grid pass applies here too). -->
  <span class="group/bell relative inline-block">
    {@render nameEl()}
    {#if tracked}
      <button
        type="button"
        class="ml-0.5 inline-flex size-3.5 cursor-pointer items-center justify-center rounded-full border border-good bg-good align-text-top text-background"
        title="Stop tracking {name} in the Drop Watch overlay"
        onclick={() => void toggleTrackedDropItem(name)}
      >
        <BellIcon class="size-2.5" />
      </button>
    {:else}
      <button
        type="button"
        class="absolute -top-2.5 -right-2.5 z-10 hidden size-4 cursor-pointer items-center justify-center rounded-full border border-primary bg-primary text-background group-hover/bell:flex"
        title="Track {name} in the Drop Watch overlay"
        onclick={() => void toggleTrackedDropItem(name)}
      >
        <BellIcon class="size-3" />
      </button>
    {/if}
  </span>
{:else}
  {@render nameEl()}
{/if}

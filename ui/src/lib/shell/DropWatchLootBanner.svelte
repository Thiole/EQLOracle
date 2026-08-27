<script lang="ts">
  // why: global toast, same corner/pattern as InventoryDumpBanner -- fires
  // on any module, not buried in the Overlay settings tab. Stacks one
  // card per pending item, each with its own independent timer.
  import { pendingLootPrompts } from '$lib/stores/dropWatchLoot';
  import DropWatchLootPrompt from './DropWatchLootPrompt.svelte';
</script>

{#if $pendingLootPrompts.length}
  <div class="fixed right-4 bottom-4 z-50 flex w-[380px] flex-col gap-2">
    {#each $pendingLootPrompts as p (p.item)}
      <DropWatchLootPrompt item={p.item} count={p.count} expiresAtMs={p.expiresAtMs} />
    {/each}
  </div>
{/if}

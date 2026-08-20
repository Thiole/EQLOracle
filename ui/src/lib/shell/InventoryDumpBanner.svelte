<script lang="ts">
  // why: global toast, not buried in Gear tab -- fires on any module
  import { Button } from '$lib/components/ui/button';
  import { pendingInventoryDump, inventoryDumpError, loadInventoryDump, dismissInventoryDump } from '$lib/stores/character';
</script>

{#if $pendingInventoryDump || $inventoryDumpError}
  <div class="fixed right-4 bottom-4 z-50 flex w-[380px] flex-col gap-2">
    {#if $pendingInventoryDump}
      <div class="rounded-sm border border-primary/40 bg-card px-3 py-2.5 text-[12px] shadow-lg">
        <p class="mb-2">
          <span class="text-primary">{$pendingInventoryDump.character ?? 'A character'}</span>'s inventory dump just finished writing
          <span class="text-muted-foreground">({$pendingInventoryDump.file})</span>.
        </p>
        <div class="flex justify-end gap-2">
          <Button size="sm" variant="ghost" class="h-6 text-[11px]" onclick={dismissInventoryDump}>Dismiss</Button>
          <Button size="sm" class="h-6 text-[11px]" onclick={loadInventoryDump}>Load into Gear</Button>
        </div>
      </div>
    {/if}
    {#if $inventoryDumpError}
      <div class="rounded-sm border border-bad/40 bg-card px-3 py-2.5 text-[11px] text-bad shadow-lg">
        Couldn't load inventory dump: {$inventoryDumpError}
      </div>
    {/if}
  </div>
{/if}

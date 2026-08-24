<script lang="ts">
  // why: global toast, same corner/pattern as InventoryDumpBanner -- fires
  // on any module, not buried in Settings
  import { Button } from '$lib/components/ui/button';
  import { availableUpdate, dismissUpdate, installUpdate, installing, installError } from '$lib/stores/updater';
</script>

{#if $availableUpdate}
  <div class="fixed right-4 bottom-4 z-50 flex w-[380px] flex-col gap-2">
    <div class="rounded-sm border border-primary/40 bg-card px-3 py-2.5 text-[12px] shadow-lg">
      <p class="mb-1">
        <span class="text-primary">Update available</span> -- v{$availableUpdate.current_version} → v{$availableUpdate.version}.
      </p>
      {#if $availableUpdate.notes}
        <p class="mb-2 max-h-24 overflow-y-auto whitespace-pre-line text-[11px] text-muted-foreground">
          {$availableUpdate.notes}
        </p>
      {/if}
      {#if $installError}
        <p class="mb-2 text-[11px] text-bad">Install failed: {$installError}</p>
      {/if}
      <div class="flex justify-end gap-2">
        <Button size="sm" variant="ghost" class="h-6 text-[11px]" disabled={$installing} onclick={dismissUpdate}>
          Later
        </Button>
        <Button size="sm" class="h-6 text-[11px]" disabled={$installing} onclick={installUpdate}>
          {$installing ? 'Installing…' : 'Install & restart'}
        </Button>
      </div>
    </div>
  </div>
{/if}

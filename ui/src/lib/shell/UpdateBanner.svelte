<script lang="ts">
  // why: global toast, same corner/pattern as InventoryDumpBanner -- fires
  // on any module, not buried in Settings
  import { Button } from '$lib/components/ui/button';
  import {
    availableUpdate,
    dismissUpdate,
    installUpdate,
    installing,
    installed,
    installProgress,
    installError,
    restartNow,
  } from '$lib/stores/updater';

  // why: whole-percent only when the server sent a content-length;
  // otherwise MB received is the honest number
  const progressLabel = $derived.by(() => {
    const p = $installProgress;
    if (!p) return 'Installing…';
    const [received, total] = p;
    if (total) return `Installing… ${Math.min(100, Math.round((100 * received) / total))}%`;
    return `Installing… ${(received / 1024 / 1024).toFixed(0)} MB`;
  });
</script>

{#if $availableUpdate}
  <div class="fixed right-4 bottom-4 z-50 flex w-[380px] flex-col gap-2">
    <div class="rounded-sm border border-primary/40 bg-card px-3 py-2.5 text-[12px] shadow-lg">
      {#if $installed}
        <!-- why: two-step flow -- the file on disk is already the new
             version; nothing auto-restarts. A plain window close works
             too, the next launch is the update either way. -->
        <p class="mb-2">
          <span class="text-good">Update installed</span> -- v{$availableUpdate.version} is ready. Restart whenever you like;
          closing the app normally also brings it up on the new version.
        </p>
        <div class="flex justify-end gap-2">
          <Button size="sm" variant="ghost" class="h-6 text-[11px]" onclick={dismissUpdate}>Later</Button>
          <Button size="sm" class="h-6 text-[11px]" onclick={restartNow}>Restart now</Button>
        </div>
      {:else}
        <p class="mb-1">
          <span class="text-primary">Update available</span> -- v{$availableUpdate.current_version} → v{$availableUpdate.version}.
          <a
            class="text-brand-soft hover:text-primary hover:underline"
            href={$availableUpdate.release_url}
            target="_blank"
            rel="noopener"
          >
            View on GitHub ↗
          </a>
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
            {$installing ? progressLabel : 'Install update'}
          </Button>
        </div>
      {/if}
    </div>
  </div>
{/if}

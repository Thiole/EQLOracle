<script lang="ts">
  // why: the one way into the Death Recap page -- a timed global toast
  // that fires when a new death lands (see stores/deathRecap.ts), same
  // corner/pattern as every other banner. Clicking opens the recap page
  // pinned to that death; ignoring it (or dismissing) means nothing,
  // the recap stays reachable through the next death's own toast.
  import { deathToast, dismissDeathToast, openDeathRecap } from '$lib/stores/deathRecap';
  import XIcon from '@lucide/svelte/icons/x';

  let remaining = $state(0);

  $effect(() => {
    const t = $deathToast;
    if (!t) return;
    remaining = Math.max(0, Math.ceil((t.expiresAtMs - Date.now()) / 1000));
    const iv = setInterval(() => {
      remaining = Math.max(0, Math.ceil((t.expiresAtMs - Date.now()) / 1000));
      if (Date.now() >= t.expiresAtMs) dismissDeathToast();
    }, 1000);
    return () => clearInterval(iv);
  });
</script>

{#if $deathToast}
  <div class="fixed right-4 bottom-4 z-50 flex w-[380px] flex-col gap-2">
    <div class="flex items-center gap-2 rounded-md border border-border bg-card px-3 py-2 shadow-lg">
      <button
        type="button"
        class="flex-1 text-left text-[13px] font-medium text-foreground hover:text-primary"
        onclick={() => openDeathRecap($deathToast?.deathTs ?? null)}
      >
        You died — death recap?
        <span class="ml-1 text-[11px] font-normal text-muted-foreground">what hit you, who healed</span>
      </button>
      <span class="text-[11px] tabular-nums text-muted-foreground">{remaining}s</span>
      <button
        type="button"
        class="rounded-sm p-0.5 text-muted-foreground hover:text-foreground"
        title="Dismiss"
        onclick={dismissDeathToast}
      >
        <XIcon class="size-3.5" />
      </button>
    </div>
  </div>
{/if}

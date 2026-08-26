<script lang="ts">
  // why: FOUNDATION.md's own house rules for a combat meter -- flat
  // opaque rows, monospace numeric column, fixed layout, no continuous
  // CSS animation (a bar's width jumps to its new value each poll, it
  // doesn't ease there -- "a number that moves is a number you can't read").
  import type { LiveMeterDto } from '$lib/tauri/api';

  let { meter }: { meter: LiveMeterDto | null } = $props();

  const maxDps = $derived(Math.max(1, ...(meter?.rows.map((r) => r.dps) ?? [0])));
</script>

<div class="flex flex-col gap-0.5 text-[12px]">
  {#if !meter || !meter.rows.length}
    <p class="text-white/70">no active fight</p>
  {:else}
    <div class="mb-0.5 truncate font-medium text-white">
      {meter.target}{meter.open ? '' : ' (ended)'}
    </div>
    {#each meter.rows as r (r.name)}
      <div class="relative overflow-hidden rounded-sm bg-black/30">
        <div class="absolute inset-y-0 left-0 bg-primary/50" style:width="{(r.dps / maxDps) * 100}%"></div>
        <div class="relative flex items-center justify-between gap-2 px-1.5 py-0.5">
          <span class="truncate {r.is_pet ? 'text-white/70 italic' : 'text-white'}">{r.name}</span>
          <span class="shrink-0 font-mono tabular-nums text-white">{r.dps.toFixed(0)}</span>
        </div>
      </div>
    {/each}
  {/if}
</div>

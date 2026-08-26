<script lang="ts">
  // why: FOUNDATION.md's own house rules for a combat meter -- flat
  // opaque rows, monospace numeric column, fixed layout, no continuous
  // CSS animation (a bar's width jumps to its new value each poll, it
  // doesn't ease there -- "a number that moves is a number you can't read").
  // Same outgoing/incoming split the Combat tab's own summary card
  // shows, outgoing on top -- "damage BY this entity" is the same real
  // calc on both sides, just grouped by which side of the fight it's on.
  import type { EntityStateDto, LiveMeterDto } from '$lib/tauri/api';

  // why: this widget's own panel background alpha -- each overlay widget
  // owns its own opacity, not one shared window-wide value (see
  // OverlayApp.svelte's own doc)
  let { meter, opacity }: { meter: LiveMeterDto | null; opacity: number } = $props();

  function total(rows: EntityStateDto[]): number {
    return rows.reduce((n, r) => n + r.dps, 0);
  }

  function maxOf(rows: EntityStateDto[]): number {
    return Math.max(1, ...rows.map((r) => r.dps));
  }
</script>

{#snippet meterRows(rows: EntityStateDto[], barClass: string)}
  {@const max = maxOf(rows)}
  {#each rows as r (r.name)}
    <div class="relative overflow-hidden rounded-sm bg-black/30">
      <div class="absolute inset-y-0 left-0 {barClass}" style:width="{(r.dps / max) * 100}%"></div>
      <div class="relative flex items-center justify-between gap-2 px-1.5 py-0.5">
        <span class="truncate {r.is_pet ? 'text-white/70 italic' : 'text-white'}">{r.name}</span>
        <span class="shrink-0 font-mono tabular-nums text-white">{r.dps.toFixed(0)}</span>
      </div>
    </div>
  {/each}
{/snippet}

<div class="flex flex-col gap-1.5 rounded-md p-2 text-[12px]" style:background-color="rgba(10, 11, 13, {opacity})">
  {#if !meter || (!meter.outgoing.length && !meter.incoming.length)}
    <p class="text-white/70">no active fight</p>
  {:else}
    <div class="truncate font-medium text-white">
      {meter.target}{meter.open ? '' : ' (ended)'}
    </div>

    {#if meter.outgoing.length}
      <div class="flex flex-col gap-0.5">
        <div class="flex items-center justify-between text-[10px] tracking-wide text-white/60 uppercase">
          <span>outgoing</span>
          <span class="font-mono tabular-nums">{total(meter.outgoing).toFixed(0)}</span>
        </div>
        {@render meterRows(meter.outgoing, 'bg-primary/50')}
      </div>
    {/if}

    {#if meter.incoming.length}
      <div class="flex flex-col gap-0.5">
        <div class="flex items-center justify-between text-[10px] tracking-wide text-white/60 uppercase">
          <span>incoming</span>
          <span class="font-mono tabular-nums">{total(meter.incoming).toFixed(0)}</span>
        </div>
        {@render meterRows(meter.incoming, 'bg-bad/50')}
      </div>
    {/if}
  {/if}
</div>

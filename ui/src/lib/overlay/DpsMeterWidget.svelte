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
  // OverlayApp.svelte's own doc). overallOpacity is the SEPARATE
  // "everything" fade (Spencer's own ask: "2 options, total opacity
  // (non text), and everything") -- a real CSS opacity on the whole
  // widget below, so text/icons fade right along with the panel
  // instead of staying fully readable no matter how see-through the
  // background is.
  let {
    meter,
    opacity,
    overallOpacity,
  }: { meter: LiveMeterDto | null; opacity: number; overallOpacity: number } = $props();

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

<!-- why: Spencer's own ask -- "make text a bit darker/bolder by
     default, so if only background is removed, its more readable".
     Bolder base weight + a dark shadow (inherited by every span below,
     nothing per-row to touch) means text stays legible against
     whatever's behind it even at background opacity 0 -- the game
     itself, not this panel's own dark fill. Reduces or removes cleanly
     for anyone who'd rather have it thinner (each row's own text color
     still wins for anything that already sets font-bold/font-mono). -->
<div
  class="flex flex-col gap-1.5 rounded-md p-2 text-[12px] font-semibold"
  style:background-color="rgba(10, 11, 13, {opacity})"
  style:opacity={overallOpacity}
  style:text-shadow="0 1px 2px rgba(0, 0, 0, 0.9), 0 0px 4px rgba(0, 0, 0, 0.6)"
>
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

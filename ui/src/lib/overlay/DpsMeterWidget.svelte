<script lang="ts">
  // why: FOUNDATION.md's own house rules for a combat meter -- flat
  // opaque rows, monospace numeric column, fixed layout, no continuous
  // CSS animation (a bar's width jumps to its new value each poll, it
  // doesn't ease there -- "a number that moves is a number you can't read").
  //
  // Row spec, asked directly: name, time in encounter, damage, DPS, %
  // of that team's damage -- totals over the WHOLE encounter (they do
  // not reset when a target dies); the encounter's own timer sits in
  // the header. The bar is % share, not dps -- share is the stable
  // comparative read, dps breathes.
  import type { LiveMeterRowDto, LiveMeterDto } from '$lib/tauri/api';

  // why: this widget's panel background alpha -- each overlay widget
  // owns its own opacity, not one shared window-wide value (see
  // OverlayApp.svelte's doc). overallOpacity is the SEPARATE
  // "everything" fade -- a CSS opacity on the whole widget, so
  // text/icons fade with the panel instead of staying fully readable
  // no matter how see-through the background is.
  let {
    meter,
    opacity,
    overallOpacity,
  }: {
    meter: LiveMeterDto | null;
    opacity: number;
    overallOpacity: number;
  } = $props();

  function sideTotal(rows: LiveMeterRowDto[]): number {
    return rows.reduce((n, r) => n + r.total, 0);
  }

  function fmtCompact(n: number): string {
    if (n < 1000) return n.toFixed(0);
    return `${(n / 1000).toFixed(1)}k`;
  }

  function fmtActive(ms: number): string {
    const s = Math.max(0, Math.round(ms / 1000));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
  }
</script>

<!-- why: fixed column widths, shared by rows and the header labels --
     alignment between a label and its numbers only holds if both sides
     use the exact same width classes. gap-3 over the old gap-2: asked
     directly, more separation between columns. -->
{#snippet columnLabels()}
  <span class="flex shrink-0 items-center gap-3 font-mono text-[9px] tracking-wide text-foreground/50 uppercase">
    <span class="w-10 text-right" title="time in encounter -- from this entity's first action">time</span>
    <span class="w-12 text-right" title="total damage over the whole encounter">dmg</span>
    <span class="w-11 text-right" title="DPS over time in encounter">dps</span>
    <span class="w-8 text-right" title="share of this side's damage">%</span>
  </span>
{/snippet}

{#snippet meterRows(rows: LiveMeterRowDto[], barClass: string)}
  {#each rows as r (r.name)}
    <div class="relative overflow-hidden rounded-sm bg-foreground/10">
      <div class="absolute inset-y-0 left-0 {barClass}" style:width="{r.pct}%"></div>
      <div class="relative flex items-center gap-3 px-1.5 py-0.5">
        <span class="min-w-0 flex-1 truncate {r.is_pet ? 'text-foreground/70 italic' : 'text-foreground'}"
          >{r.name}<!--
          why: an AoE lands one line per target, so N of one name in a
               single instant is a census. Shown as "x5+" because it is a
               high-water mark, never a live count -- nothing in the log
               says how many are up right now.
        -->{#if r.instances}<span class="ml-1 font-mono text-[10px] text-foreground/60" title="{r.instances} of these were up at once, seen when an area effect landed on all of them">&times;{r.instances}+</span>{/if}</span>
        <span class="w-10 shrink-0 text-right font-mono text-[10px] text-foreground/70 tabular-nums" title="time in encounter -- from this entity's first action">{fmtActive(r.active_ms)}</span>
        <span class="w-12 shrink-0 text-right font-mono text-foreground/80 tabular-nums" title="total damage over the whole encounter">{fmtCompact(r.total)}</span>
        <span class="w-11 shrink-0 text-right font-mono text-foreground tabular-nums" title="DPS over time in encounter">{r.dps.toFixed(0)}</span>
        <span class="w-8 shrink-0 text-right font-mono text-[10px] text-foreground/70 tabular-nums" title="share of this side's damage">{r.pct.toFixed(0)}%</span>
      </div>
    </div>
  {/each}
{/snippet}

<!-- why: bolder base weight + a dark shadow (inherited by every span
     below) keeps text legible against whatever's behind it even at
     background opacity 0 -- the game itself, not this panel's fill.
     Each row's own text color still wins for font-bold/font-mono.

     Panel background is the theme's own --background now, not a fixed
     rgb triple -- color-mix against transparent lets a THEME color
     still take a variable alpha, same as rgba() did for the one fixed
     color this used to always be. -->
<div
  class="flex flex-col gap-1.5 rounded-md p-2 text-[12px] font-semibold"
  style:background-color="color-mix(in srgb, var(--background) {opacity * 100}%, transparent)"
  style:opacity={overallOpacity}
  style:text-shadow="0 1px 2px rgba(0, 0, 0, 0.9), 0 0px 4px rgba(0, 0, 0, 0.6)"
>
  {#if !meter || (!meter.outgoing.length && !meter.incoming.length)}
    <p class="text-muted-foreground">no active fight</p>
  {:else}
    <!-- why: the encounter is named as team v team, never after one
         mob -- a mob's death must not rename the header; the current
         target sits UNDER it, as asked -->
    <div class="truncate font-medium text-foreground">
      encounter
      <span class="ml-1 font-mono text-[10px] text-foreground/70 tabular-nums" title="encounter clock -- from your first involvement">{fmtActive(meter.duration_ms)}</span>
      <span class="ml-1 font-mono text-[10px] text-foreground/70" title="allies dealing damage v enemies involved">{meter.ally_count} v {meter.enemy_count}</span>{meter.open ? '' : ' (ended)'}
    </div>
    {#if meter.current_target}
      <div class="truncate text-[10px] text-foreground/70">current target: {meter.current_target}</div>
    {/if}

    {#if meter.outgoing.length}
      <div class="flex flex-col gap-0.5">
        <!-- why: side total moves next to the section name so the right
             edge can carry the column labels, aligned over the numbers
             (same px-1.5 the rows use inside their bar container) -->
        <div class="flex items-center justify-between px-1.5 text-[10px] tracking-wide text-muted-foreground uppercase">
          <span>outgoing · <span class="font-mono tabular-nums">{fmtCompact(sideTotal(meter.outgoing))}</span></span>
          {@render columnLabels()}
        </div>
        {@render meterRows(meter.outgoing, 'bg-primary/50')}
      </div>
    {/if}

    {#if meter.incoming.length}
      <div class="flex flex-col gap-0.5">
        <div class="flex items-center justify-between px-1.5 text-[10px] tracking-wide text-muted-foreground uppercase">
          <span>incoming · <span class="font-mono tabular-nums">{fmtCompact(sideTotal(meter.incoming))}</span></span>
          {@render columnLabels()}
        </div>
        {@render meterRows(meter.incoming, 'bg-bad/50')}
      </div>
    {/if}

  {/if}
</div>

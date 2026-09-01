<script lang="ts">
  // why: FOUNDATION.md's own house rules for a combat meter -- flat
  // opaque rows, monospace numeric column, fixed layout, no continuous
  // CSS animation (a bar's width jumps to its new value each poll, it
  // doesn't ease there -- "a number that moves is a number you can't read").
  //
  // Row spec, asked directly: % of the side's damage, total damage, DPS,
  // and time active -- where "active" is THAT entity's own engagement
  // window (their first action to the fight's live edge), so a late
  // joiner's DPS is honest instead of pull-diluted. The bar is % share,
  // not dps -- share is the stable comparative read, dps breathes.
  import type { LiveMeterRowDto, LiveMeterDto, SpellCheckDto } from '$lib/tauri/api';

  // why: this widget's panel background alpha -- each overlay widget
  // owns its own opacity, not one shared window-wide value (see
  // OverlayApp.svelte's doc). overallOpacity is the SEPARATE
  // "everything" fade -- a CSS opacity on the whole widget, so
  // text/icons fade with the panel instead of staying fully readable
  // no matter how see-through the background is.
  let {
    meter,
    spellCheck = null,
    opacity,
    overallOpacity,
  }: {
    meter: LiveMeterDto | null;
    spellCheck?: SpellCheckDto | null;
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

{#snippet meterRows(rows: LiveMeterRowDto[], barClass: string)}
  {#each rows as r (r.name)}
    <div class="relative overflow-hidden rounded-sm bg-foreground/10">
      <div class="absolute inset-y-0 left-0 {barClass}" style:width="{r.pct}%"></div>
      <div class="relative flex items-center gap-2 px-1.5 py-0.5">
        <span class="min-w-0 flex-1 truncate {r.is_pet ? 'text-foreground/70 italic' : 'text-foreground'}">{r.name}</span>
        <span class="shrink-0 font-mono text-[10px] text-foreground/70 tabular-nums" title="time active -- from this entity's own first action">{fmtActive(r.active_ms)}</span>
        <span class="shrink-0 font-mono text-foreground/80 tabular-nums" title="total damage">{fmtCompact(r.total)}</span>
        <span class="shrink-0 font-mono text-foreground tabular-nums" title="DPS over own active time">{r.dps.toFixed(0)}</span>
        <span class="w-9 shrink-0 text-right font-mono text-[10px] text-foreground/70 tabular-nums" title="share of this side's damage">{r.pct.toFixed(0)}%</span>
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
    <div class="truncate font-medium text-foreground">
      {meter.target}{meter.open ? '' : ' (ended)'}
    </div>

    {#if meter.outgoing.length}
      <div class="flex flex-col gap-0.5">
        <div class="flex items-center justify-between text-[10px] tracking-wide text-muted-foreground uppercase">
          <span>outgoing</span>
          <span class="font-mono tabular-nums">{fmtCompact(sideTotal(meter.outgoing))} dmg</span>
        </div>
        {@render meterRows(meter.outgoing, 'bg-primary/50')}
      </div>
    {/if}

    {#if meter.incoming.length}
      <div class="flex flex-col gap-0.5">
        <div class="flex items-center justify-between text-[10px] tracking-wide text-muted-foreground uppercase">
          <span>incoming</span>
          <span class="font-mono tabular-nums">{fmtCompact(sideTotal(meter.incoming))} dmg</span>
        </div>
        {@render meterRows(meter.incoming, 'bg-bad/50')}
      </div>
    {/if}

    <!-- why: rolling landing-average check, target-blind -- appears
         only while a well-sampled spell is landing well under its
         baseline (partial resists). Baseline prefers the last 5 zones
         under the CURRENT invocation so a stance switch isn't a false
         dip; session norm is the fallback. See ingest::SpellPerf. -->
    {#if spellCheck && spellCheck.struggling.length}
      <div class="flex flex-col gap-0.5 border-t border-foreground/15 pt-1">
        {#each spellCheck.struggling as s (s.name)}
          <div class="flex items-center gap-2 text-[10px]">
            <span class="min-w-0 flex-1 truncate text-bad">{s.name}</span>
            <span
              class="shrink-0 font-mono tabular-nums text-bad"
              title="recent avg hit {fmtCompact(s.recent_avg)} vs {s.matched
                ? `${spellCheck.invocation ?? 'same-invocation'} baseline`
                : 'session norm'} {fmtCompact(s.baseline)}"
              >{(s.ratio * 100).toFixed(0)}% of usual</span
            >
          </div>
        {/each}
        {#if spellCheck.alternatives.length}
          <div class="truncate text-[10px] text-muted-foreground">
            holding: {spellCheck.alternatives
              .map((a) => `${a.name} ~${fmtCompact(a.baseline)}`)
              .join(' · ')}
          </div>
        {/if}
      </div>
    {/if}
  {/if}
</div>

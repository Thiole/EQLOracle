<script lang="ts">
  import { fmtLogTime } from '$lib/utils';
  import { ICON_BASE } from '$lib/character/constants';
  // why: the bar's own clock -- 'left' seconds, minutes past a minute
  const fmtLeft = (ms: number) => (ms >= 60_000 ? `${Math.floor(ms / 60_000)}m${String(Math.floor((ms % 60_000) / 1000)).padStart(2, '0')}` : `${Math.ceil(ms / 1000)}s`);
  // Same multi-series overlapping-lines idea the legacy app used (one
  // polyline per entity, easier to compare shapes over time than a row
  // of bars per person), rebuilt reactively instead of the legacy
  // version's own `chart.innerHTML = ''` + fresh `<svg>` on every
  // redraw. Every point here is a plain `$derived` value bound directly
  // into the template, so a `parse-tick` update to the `timeline` store
  // (see stores/combat.ts's `onCombatTick`) patches only the polylines/
  // circles whose numbers actually changed -- Svelte's own fine-grained
  // reactivity, not a rebuild -- which is the actual "constant time"
  // property asked for, regardless of which charting approach draws the
  // SVG itself.
  import { timeline, scrubTo, stateAt, inspectWindowS, setInspectWindow } from '$lib/stores/combat';

  // why: one condensed line per action -- "hit a gnoll 120 · Burst of Flame"
  function actionText(a: { kind: string; target: string; ability: string; amount: number; crit: boolean }): string {
    const crit = a.crit ? ' (crit)' : '';
    switch (a.kind) {
      case 'hit':
        return `hit ${a.target} ${a.amount.toLocaleString()} · ${a.ability}${crit}`;
      case 'miss':
        return `missed ${a.target} · ${a.ability}`;
      case 'heal':
        return `healed ${a.target} ${a.amount.toLocaleString()} · ${a.ability}`;
      default:
        return `cast ${a.ability}${a.target && a.target !== a.ability ? ` on ${a.target}` : ''}`;
    }
  }

  const VIEW_W = 960;
  const VIEW_H = 180;
  const PAD_TOP = 8;

  const SERIES_COLORS = [
    'var(--color-primary)',
    'var(--color-good)',
    'var(--color-caution)',
    'var(--color-bad)',
    'var(--color-brand-soft)',
    '#c084fc',
    '#fb923c',
    '#34d399',
  ];

  let highlighted = $state<string | null>(null);

  const globalMax = $derived(Math.max(1, ...($timeline?.series.flatMap((s) => s.values) ?? [0])));
  const bucketCount = $derived($timeline?.buckets.length ?? 0);
  const xStep = $derived(bucketCount > 1 ? VIEW_W / (bucketCount - 1) : 0);

  function xFor(i: number) {
    return bucketCount > 1 ? i * xStep : VIEW_W / 2;
  }
  function yFor(v: number) {
    return PAD_TOP + (VIEW_H - PAD_TOP) * (1 - v / globalMax);
  }

  const points = $derived(
    ($timeline?.series ?? []).map((s, i) => ({
      name: s.name,
      color: SERIES_COLORS[i % SERIES_COLORS.length],
      side: s.is_player || s.is_pet ? 'ally' : s.is_enemy ? 'enemy' : '',
      pointsAttr: s.values.map((v, bi) => `${xFor(bi).toFixed(1)},${yFor(v).toFixed(1)}`).join(' '),
      total: s.total,
    })),
  );

  function onChartClick(event: MouseEvent, svg: SVGSVGElement) {
    if (!$timeline) return;
    const rect = svg.getBoundingClientRect();
    const relX = ((event.clientX - rect.left) / rect.width) * VIEW_W;
    const idx = bucketCount > 1 ? Math.round(relX / xStep) : 0;
    const clamped = Math.max(0, Math.min(bucketCount - 1, idx));
    void scrubTo($timeline.buckets[clamped] ?? $timeline.start_ms);
  }

  // Keyboard equivalent of clicking a point on the chart -- left/right
  // steps the scrub position one bucket at a time, so the "click to
  // inspect a moment" interaction has a non-pointer path too.
  let scrubIndex = $state(0);
  function onChartKeydown(event: KeyboardEvent) {
    if (!$timeline) return;
    if (event.key === 'ArrowLeft') scrubIndex = Math.max(0, scrubIndex - 1);
    else if (event.key === 'ArrowRight') scrubIndex = Math.min(bucketCount - 1, scrubIndex + 1);
    else return;
    event.preventDefault();
    void scrubTo($timeline.buckets[scrubIndex] ?? $timeline.start_ms);
  }
</script>

{#if $timeline}
  <div class="flex flex-wrap gap-2">
    {#each points as p (p.name)}
      <button
        type="button"
        class="flex items-center gap-1.5 rounded-full border border-border bg-muted/40 px-2 py-0.5 text-[11px] transition-opacity"
        style:opacity={highlighted && highlighted !== p.name ? 0.4 : 1}
        onclick={() => (highlighted = highlighted === p.name ? null : p.name)}
      >
        <span class="size-2 rounded-full" style:background={p.color}></span>
        <span class={p.side === 'ally' ? 'text-primary' : p.side === 'enemy' ? 'text-bad' : ''}>{p.name}</span>
        <span class="tabular-nums text-muted-foreground">({p.total.toLocaleString()})</span>
      </button>
    {/each}
  </div>

  <!-- The interactive element is this div, not the <svg> inside it --
       Svelte's a11y linter treats <svg> as permanently non-interactive
       regardless of role, so the click/keyboard handlers live on the
       wrapper instead. -->
  <div
    class="mt-2 cursor-crosshair rounded-md bg-muted/20 focus-visible:outline focus-visible:outline-ring"
    role="application"
    aria-label="Damage over time per entity. Click or use arrow keys to inspect a moment."
    tabindex="0"
    onclick={(e) => onChartClick(e, e.currentTarget.querySelector('svg')!)}
    onkeydown={onChartKeydown}
  >
    <svg viewBox="0 0 {VIEW_W} {VIEW_H}" preserveAspectRatio="none" class="h-40 w-full">
      {#each points as p (p.name)}
        <polyline
          points={p.pointsAttr}
          fill="none"
          stroke={p.color}
          stroke-width={highlighted === p.name ? 2.5 : 1.5}
          opacity={highlighted && highlighted !== p.name ? 0.25 : 1}
        />
      {/each}
    </svg>
  </div>
  <p class="mt-1 text-[11px] text-muted-foreground">click the chart to see who was doing what at that instant</p>

  {#if $stateAt}
    <div class="mt-2 rounded-md border border-border bg-muted/20 p-2">
      <div class="mb-1 flex items-center gap-2 text-[11px]">
        <span class="font-medium">{fmtLogTime($stateAt.tsMs)}</span>
        <span class="text-muted-foreground">recent</span>
        <!-- why: the window behind "recent" -- dps and the action list both run over it -->
        {#each [1, 2, 3, 4, 5, 6, 7, 8] as s (s)}
          <button
            type="button"
            class="rounded-sm border px-1 text-[10px] {$inspectWindowS === s ? 'border-primary text-primary' : 'border-border text-muted-foreground hover:text-foreground'}"
            onclick={() => void setInspectWindow(s)}>{s}s</button
          >
        {/each}
      </div>
      <table class="w-full text-[11px]">
        <tbody>
          {#each $stateAt.entities as e (e.name)}
            <tr class="border-b border-border/50">
              <td class="py-0.5 {e.is_player || e.is_pet ? 'text-primary' : e.is_enemy ? 'text-bad' : ''}">
                {e.name}
                <!-- why: Spencer -- "buffs/debuffs currently assumed on each
                     target ... spell icons, with the duration it thinks is
                     left ... a buff bar under the targets name" -->
                {#if (e.effects ?? []).length}
                  <div class="mt-0.5 flex flex-wrap gap-0.5">
                    {#each e.effects as f (f.spell)}
                      <span
                        class="inline-flex items-center gap-0.5 rounded-[3px] border border-border bg-muted/30 px-0.5 text-[9px] text-muted-foreground"
                        title="{f.spell}{f.source ? ` · from ${f.source}` : ''}{f.remaining_ms != null ? ` · ${fmtLeft(f.remaining_ms)} left` : ' · duration unknown'}"
                      >
                        {#if f.icon}
                          <img src={ICON_BASE + encodeURIComponent(f.icon)} alt="" class="size-3.5 rounded-[2px]" />
                        {:else}
                          <span class="inline-block size-3.5 rounded-[2px] bg-muted"></span>
                        {/if}
                        {#if f.remaining_ms != null}<span class="tabular-nums">{fmtLeft(f.remaining_ms)}</span>{/if}
                      </span>
                    {/each}
                  </div>
                {/if}
              </td>
              <td class="py-0.5 text-muted-foreground">{e.state}{e.observed ? '' : ' (inferred)'}</td>
              <td class="py-0.5 text-right tabular-nums">{e.dps.toFixed(1)} dps</td>
            </tr>
            {#if (e.window_abilities ?? []).length > 0}
              <!-- why: "show total but # of hits in that window" -- one
                   line per ability, combined, so a 3-cast burst reads
                   as three, not one -->
              <tr class="border-b border-border/50">
                <td colspan="3" class="py-0.5 pl-3">
                  <div class="flex flex-wrap gap-x-3 gap-y-0.5 text-[10px]">
                    {#each e.window_abilities as w (w.ability)}
                      <span><span class="text-foreground">{w.ability}</span> <span class="text-muted-foreground">×{w.count}{#if w.total} · {w.total.toLocaleString()}{/if}</span></span>
                    {/each}
                  </div>
                </td>
              </tr>
            {/if}
            {#if (e.recent_actions ?? []).length > 0}
              <tr class="border-b border-border/50">
                <td colspan="3" class="py-0.5 pl-3">
                  <div class="flex flex-col gap-0.5 text-muted-foreground">
                    {#each e.recent_actions as a, i (i)}
                      <div class="flex gap-2">
                        <span class="shrink-0 tabular-nums">{fmtLogTime(a.ts_ms)}</span>
                        <span class={a.kind === 'hit' ? 'text-foreground' : ''}>{actionText(a)}</span>
                      </div>
                    {/each}
                  </div>
                </td>
              </tr>
            {/if}
            {#if e.recent_effects.length > 0}
              <tr class="border-b border-border/50">
                <td colspan="3" class="py-0.5 pl-3">
                  <div class="text-[10px] uppercase tracking-wide text-muted-foreground">recent effects</div>
                  <div class="flex flex-col gap-0.5">
                    {#each e.recent_effects as eff, i (i)}
                      <div class="flex gap-1 text-muted-foreground">
                        {#if eff.source}
                          <span class="text-foreground">{eff.source}</span>
                          <span>›</span>
                        {/if}
                        {#if eff.skill}
                          <span class="text-primary">{eff.skill}</span>
                          <span>›</span>
                        {/if}
                        <span>{eff.text}</span>
                      </div>
                    {/each}
                  </div>
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
{/if}

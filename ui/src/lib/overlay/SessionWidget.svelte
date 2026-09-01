<script lang="ts">
  // why: session rates at a glance -- 3 stat columns (AA/levels/plat per
  // hour, value over label, asked directly) with a mote strip underneath
  // (tier circles + counts). Same numbers as the Overview tab's session
  // card, overlay form. "--" is an honest "rate not meaningful yet"
  // (session under a minute), never a fake 0.
  import type { SessionDto } from '$lib/tauri/api';

  let {
    session,
    opacity,
    overallOpacity,
  }: {
    session: SessionDto | null;
    opacity: number;
    overallOpacity: number;
  } = $props();

  function rate(v: number | null | undefined, digits: number): string {
    if (v == null) return '--';
    if (v >= 1000) return `${(v / 1000).toFixed(1)}k`;
    return v.toFixed(digits);
  }
</script>

<!-- why: same legibility treatment as every other overlay widget --
     bolder text + dark shadow, theme --background via color-mix. -->
<div
  class="flex flex-col gap-1.5 rounded-md p-2 text-[12px] font-semibold"
  style:background-color="color-mix(in srgb, var(--background) {opacity * 100}%, transparent)"
  style:opacity={overallOpacity}
  style:text-shadow="0 1px 2px rgba(0, 0, 0, 0.9), 0 0px 4px rgba(0, 0, 0, 0.6)"
>
  {#if !session}
    <p class="text-muted-foreground">waiting for session data…</p>
  {:else}
    <div class="grid grid-cols-3 gap-1.5 text-center">
      <div class="rounded-sm bg-foreground/10 px-1 py-1.5">
        <div class="font-mono text-[16px] text-foreground tabular-nums">{rate(session.aa_per_hour, 1)}</div>
        <div class="text-[9px] tracking-wide text-foreground/60 uppercase">AA/hr</div>
      </div>
      <div class="rounded-sm bg-foreground/10 px-1 py-1.5">
        <div class="font-mono text-[16px] text-foreground tabular-nums">{rate(session.levels_per_hour, 2)}</div>
        <div class="text-[9px] tracking-wide text-foreground/60 uppercase">levels/hr</div>
      </div>
      <div class="rounded-sm bg-foreground/10 px-1 py-1.5">
        <div class="font-mono text-[16px] text-foreground tabular-nums">{rate(session.platinum_per_hour, 1)}</div>
        <div class="text-[9px] tracking-wide text-foreground/60 uppercase">plat/hr</div>
      </div>
    </div>
    <div class="flex flex-wrap items-center gap-x-1.5 gap-y-1 rounded-sm bg-foreground/10 px-1.5 py-1">
      <span class="text-[9px] tracking-wide text-foreground/60 uppercase">motes</span>
      <span class="font-mono text-foreground tabular-nums">{session.motes_found}</span>
      <span class="font-mono text-[10px] text-foreground/60 tabular-nums">({rate(session.motes_per_hour, 1)}/hr)</span>
      {#if session.mote_tiers.length}
        <span class="flex flex-1 flex-wrap items-center justify-end gap-1.5">
          {#each session.mote_tiers as t (t.name)}
            <span title={t.name} class="flex items-center gap-0.5">
              <span
                class="flex size-4 shrink-0 items-center justify-center rounded-full border border-foreground/40 font-mono text-[9px] text-foreground/80"
              >
                {t.tier ?? '?'}
              </span>
              <span class="font-mono text-[10px] text-foreground tabular-nums">{t.count}</span>
            </span>
          {/each}
        </span>
      {/if}
    </div>
  {/if}
</div>

<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Badge } from '$lib/components/ui/badge';
  import { type RaidDto, type RaidTargetDto, type BestTimeDto } from '$lib/tauri/api';
  import { raidRows, raidRowsError, refreshRaidRows } from '$lib/stores/raiding';

  // why: same fixed 5-tier scale `zone::zone_tier` parses out of a zone
  // name everywhere else in the app -- index 0 is the base/untiered zone,
  // 1-4 the four named difficulty suffixes, in that order. Labeled "D0"-
  // "D4" here to match the Solo/Group grid's own compact column headers.
  const TIER_LABELS = ['D0 (Base)', 'D1 (Awakened)', 'D2 (Adaptive)', 'D3 (Fused)', 'D4 (Refined)'];

  // why: rows/refresh now live in a store (see `stores/raiding.ts`) so a
  // live boss kill updates this tab from `parse-tick` without a manual
  // reload -- this component just triggers the initial load.
  $effect(() => {
    void refreshRaidRows();
  });

  // why: per-zone open/closed state, not native <details> -- lets the
  // expanded panel render full-width below the header instead of nested
  // inside the right-aligned toggle's own flex slot
  let openFastest = $state<Set<string>>(new Set());
  function toggleFastest(zone: string) {
    const next = new Set(openFastest);
    if (next.has(zone)) next.delete(zone);
    else next.add(zone);
    openFastest = next;
  }

  function dropsLootedCount(t: RaidTargetDto): number {
    return t.drops.filter((d) => d.looted).length;
  }

  // why: H:MM:SS once an hour is crossed (Nagafen's Lair's own real
  // fastest run is well over an hour), MM:SS below that -- a speedrun
  // timer that silently rolled hours into a 90-minute "MM:SS" would read
  // wrong at a glance.
  function formatDuration(ms: number): string {
    const totalSeconds = Math.round(ms / 1000);
    const h = Math.floor(totalSeconds / 3600);
    const m = Math.floor((totalSeconds % 3600) / 60);
    const s = totalSeconds % 60;
    const mm = h > 0 ? String(m).padStart(2, '0') : String(m);
    return h > 0 ? `${h}:${mm}:${String(s).padStart(2, '0')}` : `${mm}:${String(s).padStart(2, '0')}`;
  }
</script>

{#snippet tierGrid(label: string, tiers: boolean[])}
  <div class="flex items-center gap-1">
    <span class="w-11 shrink-0 text-muted-foreground">{label}</span>
    <div class="flex gap-0.5">
      {#each tiers as cleared, i (i)}
        <span
          class="h-2.5 w-2.5 rounded-sm border {cleared ? 'border-primary bg-primary/70' : 'border-border bg-muted'}"
          title="{TIER_LABELS[i]}: {cleared ? 'cleared' : 'not yet cleared'}"
        ></span>
      {/each}
    </div>
  </div>
{/snippet}

{#snippet timeRow(label: string, times: (BestTimeDto | null)[])}
  <div class="flex items-center gap-1">
    <span class="w-11 shrink-0 text-muted-foreground">{label}</span>
    <div class="flex gap-0.5">
      {#each times as t, i (i)}
        <span
          class="flex h-4 w-11 items-center justify-center rounded-sm border font-mono text-[9px] tabular-nums {t ? 'border-primary/40 bg-primary/10 text-foreground' : 'border-border text-muted-foreground'}"
          title={t ? `${TIER_LABELS[i]}, ${label}: ${formatDuration(t.duration_ms)}, achieved ${new Date(t.achieved_ms).toLocaleString()}` : `${TIER_LABELS[i]}, ${label}: not cleared yet`}
        >
          {t ? formatDuration(t.duration_ms) : '--'}
        </span>
      {/each}
    </div>
  </div>
{/snippet}

{#snippet fastestTimesToggle(raid: RaidDto)}
  <!-- why: plain toggle button, not <details> -- the expanded panel
       renders separately below the whole header (see fastestTimesPanel)
       so it can left-align under the zone title instead of nesting
       under this right-aligned link. Colored apart from the plain
       muted-foreground toggles elsewhere on this page (drops/level etc.)
       on purpose -- it's the one thing on this card that isn't a
       completion metric, and reads as "its own thing" at a glance. -->
  <button
    type="button"
    class="shrink-0 text-[11px] font-medium text-primary hover:text-primary/80"
    onclick={() => toggleFastest(raid.zone)}
  >
    Fastest Times
  </button>
{/snippet}

{#snippet fastestTimesPanel(raid: RaidDto)}
  <!-- why: mt-2 drops it a little below the header row; full-width and
       left-aligned so it sits directly under the zone title, matching
       the card's own left edge instead of the toggle's right-aligned spot. -->
  <div class="mb-1.5 mt-2 flex flex-col gap-1.5 rounded-sm border border-primary/30 bg-primary/5 px-2 py-1.5 text-left text-[11px]">
    <p class="text-[10px] text-muted-foreground">first action → {raid.boss.name} kill, fastest per difficulty</p>
    <div class="flex flex-col gap-0.5">
      {@render timeRow('Solo', raid.times.solo)}
      {@render timeRow('Group', raid.times.group)}
    </div>
    <div class="mt-1 flex items-baseline justify-between gap-2 border-t border-primary/20 pt-1">
      <span class="text-muted-foreground">Full Clear <span class="italic">(coming soon)</span></span>
      <span class="text-muted-foreground">--</span>
    </div>
  </div>
{/snippet}

{#snippet target(t: RaidTargetDto, kind: 'boss' | 'miniboss')}
  {@const looted = dropsLootedCount(t)}
  <div class="flex flex-col gap-1.5 py-2 first:pt-0 last:pb-0">
    <div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
      <div class="flex items-baseline gap-2">
        {#if kind === 'boss'}
          <Badge class="h-4 px-1.5 text-[9px] uppercase tracking-wide">boss</Badge>
        {/if}
        <span class="text-[13px] font-medium text-foreground">{t.name}</span>
        {#if t.level}
          <span class="text-[11px] text-muted-foreground">lvl {t.level}</span>
        {/if}
      </div>
      <span class="text-[11px] text-muted-foreground">{t.kills} kill{t.kills === 1 ? '' : 's'}</span>
    </div>

    <div class="flex flex-wrap items-center gap-x-5 gap-y-1 text-[10px]">
      <div class="flex flex-col gap-0.5">
        {@render tierGrid('Solo', t.solo_tiers_cleared)}
        {@render tierGrid('Group', t.group_tiers_cleared)}
      </div>

      <div class="flex flex-1 items-center gap-1.5 text-[11px]">
        <span class="shrink-0 text-muted-foreground">drops {looted}/{t.drops.length}</span>
        <div class="h-1.5 max-w-32 flex-1 overflow-hidden rounded-full bg-muted">
          <div class="h-full rounded-full bg-primary" style="width: {t.drops.length ? (100 * looted) / t.drops.length : 0}%"></div>
        </div>
      </div>
    </div>

    {#if t.drops.length}
      <details class="text-[11px]">
        <summary class="cursor-pointer font-medium text-brand-soft hover:text-brand-soft/80">known drops</summary>
        <div class="mt-1 flex flex-wrap gap-1 rounded-sm border border-brand-soft/30 bg-brand-soft/5 p-1.5">
          {#each t.drops as drop (drop.item)}
            <Badge
              variant={drop.looted ? 'default' : 'outline'}
              class="h-5 text-[10px] {drop.looted ? '' : 'text-muted-foreground'}"
              title={drop.looted ? `looted x${drop.count}` : 'not yet looted'}
            >
              {drop.item}{drop.looted ? ` ×${drop.count}` : ''}
            </Badge>
          {/each}
        </div>
      </details>
    {:else}
      <p class="text-[11px] text-muted-foreground">no known drop table scraped for this one yet</p>
    {/if}
  </div>
{/snippet}

{#if $raidRowsError}
  <div class="flex items-center gap-2 text-[12px]">
    <p class="text-destructive">{$raidRowsError}</p>
    <button type="button" class="text-primary underline" onclick={refreshRaidRows}>retry</button>
  </div>
{:else if !$raidRows}
  <p class="text-[12px] text-muted-foreground">Loading…</p>
{:else if !$raidRows.length}
  <!-- why: an empty payload renders a sentence, not a blank page --
       "Loading…" forever and silent blankness are the two shapes the
       stuck-tab report could take; both now say something true -->
  <p class="text-[12px] text-muted-foreground">No raid data yet — it fills in as the log replays.</p>
{:else}
  <div class="flex flex-col gap-5">
    {#each $raidRows as row (row.row)}
      <div class="flex flex-col gap-2">
        <h1 class="text-[13px] font-semibold text-foreground">{row.row}</h1>
        <div class="grid grid-cols-2 gap-3">
          {#each row.raids as raid (raid.zone)}
            <Card class="rounded-sm">
              <CardContent class="px-3 py-2.5">
                <div class="mb-1.5 flex items-start justify-between gap-3">
                  <h2 class="panel-title">{raid.zone}</h2>
                  {@render fastestTimesToggle(raid)}
                </div>
                {#if openFastest.has(raid.zone)}
                  {@render fastestTimesPanel(raid)}
                {/if}
                <div class="flex flex-col divide-y divide-border">
                  {@render target(raid.boss, 'boss')}
                  {#each raid.minibosses as m (m.name)}
                    {@render target(m, 'miniboss')}
                  {/each}
                </div>
              </CardContent>
            </Card>
          {/each}
        </div>
      </div>
    {/each}
  </div>
{/if}

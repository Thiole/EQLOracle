<script lang="ts">
  import { fade } from 'svelte/transition';
  import { Card, CardContent } from '$lib/components/ui/card';
  import CopyIcon from '@lucide/svelte/icons/copy';
  import ChevronIcon from '@lucide/svelte/icons/chevron-right';
  import AllyTable from './AllyTable.svelte';
  import FightTree from './FightTree.svelte';
  import HistoryPane from './HistoryPane.svelte';
  import FightTimelineChart from './FightTimelineChart.svelte';
  import { buildCombatReport } from './report';
  import { copyText } from '$lib/clipboard';
  import {
    selection,
    visitFights,
    visitKey,
    singleEncounter,
    singleMob,
    scope,
    summary,
    allies,
    enemies,
    showEnemies,
    toggleEnemies,
    timeline,
    loadZoneVisitsThenJumpOrReset,
  } from '$lib/stores/combat';
  import { api } from '$lib/tauri/api';
  import { get } from 'svelte/store';
  import { fmtDuration } from '$lib/format';

  // why: also the entry point for Game Data's "open in Combat →" -- the
  // mount effect honors a pending jump, else follows the newest fight
  $effect(() => {
    void loadZoneVisitsThenJumpOrReset();
  });

  const stats = $derived(
    $summary
      ? [
          { label: 'fights', value: $summary.fight_count.toLocaleString(), tone: '' },
          { label: 'duration', value: fmtDuration($summary.duration_ms), tone: '' },
          { label: 'team damage', value: $summary.total_damage.toLocaleString(), tone: 'text-good' },
          { label: 'team dps', value: $summary.dps.toFixed(1), tone: 'text-good' },
          { label: 'incoming damage', value: $summary.enemy_damage.toLocaleString(), tone: 'text-bad' },
          { label: 'incoming dps', value: $summary.enemy_dps.toFixed(1), tone: 'text-bad' },
        ]
      : [],
  );

  // ---------------------------------------------------------------- copy report
  let copyNote = $state<{ x: number; y: number; text: string } | null>(null);
  let copyNoteTimer: ReturnType<typeof setTimeout> | undefined;

  async function copyReport(event: MouseEvent) {
    if (!$summary) return;
    // why: one fight copies exactly what is on screen and names it; any
    // other selection is an aggregate copy that drops reset fights
    // (abandoned/fled fragments dilute shared numbers) and says so
    const one = singleEncounter(get(selection));
    const mob = singleMob(get(selection));
    const e = one ? ($visitFights[visitKey(one.visit)] ?? []).find((x) => x.id === one.id) : undefined;
    const tag = e ? (e.open ? 'ongoing' : e.slain ? 'kill' : e.wiped ? 'wipe' : 'reset') : null;
    let sum = $summary;
    let allyRows = $allies;
    // why: one fight or one mob copies what is on screen and names it
    const aggregate = one === null && mob === null;
    if (aggregate) {
      const { zv, enc, sel } = scope();
      const [s, a] = await Promise.all([api.getCombatSummary(zv, enc, null, true, sel), api.listAllies(zv, enc, true, sel)]);
      if (s) sum = s;
      if (a) allyRows = a;
    }
    const report = buildCombatReport(
      { target: one?.target ?? mob?.name ?? null, tag, fightCount: sum.fight_count, resetsExcluded: aggregate },
      sum,
      allyRows,
    );
    // why: failure is SHOWN -- a silent return read as "copied" and left a stale clipboard
    const ok = await copyText(report);
    clearTimeout(copyNoteTimer);
    copyNote = {
      x: event.clientX,
      y: event.clientY,
      text: ok ? 'report copied to clipboard' : 'clipboard copy FAILED',
    };
    copyNoteTimer = setTimeout(() => (copyNote = null), 1400);
  }
</script>

<!-- why: two panes -- the list that is the scope on the left, the data
     it describes on the right; below ~1100px the list stacks on top,
     twelve rows tall, so nothing is hidden behind a toggle -->
<div class="grid grid-cols-1 items-start gap-4 p-4 lg:grid-cols-[340px_minmax(0,1fr)]">
  <FightTree />

  <div class="flex min-w-0 flex-col gap-4">
    {#if !$selection.length}
      <p class="rounded-sm border border-border bg-card px-4 py-6 text-center text-[12px] text-muted-foreground">
        Pick a fight or a zone on the left. Ctrl-click to add, shift-click to span, or drag a box.
      </p>
    {/if}

    {#if stats.length}
      <div class="flex items-stretch gap-2">
        <div class="flex flex-1 divide-x divide-border rounded-sm border border-border bg-card">
          {#each stats as s (s.label)}
            <div class="flex-1 px-4 py-2.5">
              <div class="stat-figure {s.tone}">{s.value}</div>
              <div class="stat-label mt-0.5">{s.label}</div>
            </div>
          {/each}
        </div>
        <button
          type="button"
          class="flex shrink-0 items-center gap-1.5 self-center rounded-md border border-border px-2.5 py-1.5 text-[11px] text-muted-foreground hover:border-foreground/30 hover:text-foreground"
          title="Copy a one-line report of this selection, ready to paste in-game -- fight, team total/dps, top allies"
          onclick={copyReport}
        >
          <CopyIcon class="size-3" />
          copy report
        </button>
      </div>
    {/if}

    {#if $selection.length}
      <div>
        <h2 class="panel-title mb-2">allies · click to see abilities</h2>
        <AllyTable />
      </div>

      <!-- why: collapsed, and not fetched at all until opened -- the enemy
           side of a pull is far longer than the ally side -->
      <div>
        <button type="button" class="flex items-center gap-1.5 text-muted-foreground hover:text-foreground" onclick={() => void toggleEnemies()}>
          <ChevronIcon class="size-3 transition-transform {$showEnemies ? 'rotate-90' : ''}" />
          <h2 class="panel-title">enemies{$showEnemies ? ' · click to see abilities' : ' · list all'}</h2>
        </button>
        {#if $showEnemies}
          <div class="mt-2">
            <AllyTable rows={$enemies} allySide={false} empty="No enemies in this selection." />
          </div>
        {/if}
      </div>
    {/if}

    <HistoryPane />

    {#if $timeline}
      <Card class="rounded-sm">
        <CardContent class="px-3 py-2.5">
          <h2 class="panel-title mb-2">fight timeline</h2>
          <FightTimelineChart />
        </CardContent>
      </Card>
    {/if}
  </div>
</div>

{#if copyNote}
  <div
    class="pointer-events-none fixed z-50 -translate-x-1/2 -translate-y-full rounded-md border border-border bg-card px-2 py-1 text-[11px] text-foreground shadow-md"
    style="left: {copyNote.x}px; top: {copyNote.y - 10}px;"
    transition:fade={{ duration: 150 }}
  >
    {copyNote.text}
  </div>
{/if}

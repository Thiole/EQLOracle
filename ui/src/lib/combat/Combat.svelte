<script lang="ts">
  import * as Select from '$lib/components/ui/select';
  import { Card, CardContent } from '$lib/components/ui/card';
  import AllyTable from './AllyTable.svelte';
  import HistoryPane from './HistoryPane.svelte';
  import FightTimelineChart from './FightTimelineChart.svelte';
  import {
    zoneVisits,
    encounters,
    selectedZoneVisit,
    selectedEncounterId,
    followCurrentFight,
    summary,
    allies,
    timeline,
    selectZoneVisit,
    selectEncounter,
    followCurrent,
    loadZoneVisitsThenJumpOrReset,
  } from '$lib/stores/combat';
  import { fmtDuration } from '$lib/format';

  // why: also the entry point for Game Data's "open in Combat →" -- see
  // that function's own doc for why the jump-or-reset choice lives here
  // rather than as a plain loadZoneVisits() call.
  $effect(() => {
    void loadZoneVisitsThenJumpOrReset();
  });

  const zoneLabel = $derived(
    $selectedZoneVisit === null
      ? `All zones (${$zoneVisits.reduce((n, v) => n + v.fight_count, 0)})`
      : ($zoneVisits.find((v) => v.index === $selectedZoneVisit)?.label ?? '—'),
  );

  const encLabel = $derived.by(() => {
    if ($selectedEncounterId === null) {
      return $followCurrentFight ? 'Current fight — none yet' : `Aggregate (${$encounters.length} fight${$encounters.length === 1 ? '' : 's'})`;
    }
    const e = $encounters.find((e) => e.id === $selectedEncounterId);
    if (!e) return '—';
    const others = e.entities.length > 1 ? ` +${e.entities.length - 1} other${e.entities.length > 2 ? 's' : ''}` : '';
    const tag = e.open ? 'ongoing' : e.slain ? 'kill' : e.wiped ? 'wipe' : 'reset';
    const prefix = $followCurrentFight ? '● Current — ' : '';
    return `${prefix}${e.target}${others} — ${fmtDuration(e.duration_ms)} — ${e.total_damage.toLocaleString()} dmg (${tag})`;
  });

  function onZoneChange(value: string | undefined) {
    const zv = value === '' || value === undefined ? null : Number(value);
    void selectZoneVisit(zv);
  }
  function onEncChange(value: string | undefined) {
    if (value === 'current') {
      void followCurrent();
      return;
    }
    const id = value === '' || value === undefined ? null : Number(value);
    void selectEncounter(id);
  }

  // ---------------------------------------------------------------- row virtualization
  //
  // Both dropdowns get the whole real list from the store (see
  // stores/combat.ts) -- a long-lived character can rack up thousands of
  // zone visits and fights, and mounting one <Select.Item> per row is what
  // actually froze the popover, not the fetch. So only a small slice
  // around whatever's actually scrolled into view gets rendered; the rest
  // is represented by a single spacer div on each side (sized to hold the
  // scrollbar at the right length) rather than being in the DOM at all.
  //
  // Trade-off: bits-ui's own keyboard arrow-nav/typeahead only sees
  // whatever's currently mounted, same as any virtualized listbox -- fine
  // here since these are scroll/click driven, not typed through.
  const ROW_H = 28; // px -- matches select-item.svelte's fixed single-line row height
  const OVERSCAN = 15; // rows kept mounted past each visible edge
  const INITIAL_VISIBLE_GUESS = 24; // rows-visible guess used only until the first real scroll/measure lands

  function scrollRange(scrollTop: number, clientHeight: number, total: number, leadingRows: number) {
    // why: leading fixed rows (All zones / Aggregate, and now also Current
    // fight) aren't part of the virtualized array, but they do occupy real
    // scroll space before the virtualized slice starts -- subtracted out
    // here so the math lines up with real array indices. `leadingRows` is
    // 1 for the zone dropdown, 2 for the fight dropdown (Current fight +
    // Aggregate) -- the two no longer share the same leading-row count.
    const effective = Math.max(0, scrollTop - ROW_H * leadingRows);
    const visibleRows = Math.max(1, Math.ceil(clientHeight / ROW_H));
    // why: ROW_H is a measured *guess*, not exact for every font renderer
    // -- a real scrollTop that runs a little ahead of what ROW_H predicts
    // (a different engine's actual row height, elastic overscroll, etc.)
    // must never push `start` past what the array can actually satisfy,
    // or the slice below comes back empty even though `total` is fine.
    // Clamping to the last valid window (instead of just `total`) also
    // means an overshoot lands on the real tail, not a blank list.
    const rawStart = Math.floor(effective / ROW_H) - OVERSCAN;
    const start = Math.max(0, Math.min(rawStart, Math.max(0, total - visibleRows)));
    return { start, end: Math.min(total, start + visibleRows + OVERSCAN * 2) };
  }

  // why: opening the dropdown should show what's already selected, not
  // always reset to the top of a list that might be thousands long.
  function centeredRange(selectedIndex: number, total: number) {
    const around = Math.max(0, selectedIndex);
    const start = Math.max(0, Math.min(around - Math.floor(INITIAL_VISIBLE_GUESS / 2), Math.max(0, total - INITIAL_VISIBLE_GUESS)));
    return { start, end: Math.min(total, start + INITIAL_VISIBLE_GUESS + OVERSCAN) };
  }

  let zoneRange = $state({ start: 0, end: INITIAL_VISIBLE_GUESS });
  let fightRange = $state({ start: 0, end: INITIAL_VISIBLE_GUESS });

  function onZoneScroll(e: Event) {
    const el = e.currentTarget as HTMLElement;
    zoneRange = scrollRange(el.scrollTop, el.clientHeight, $zoneVisits.length, 1);
  }
  function onFightScroll(e: Event) {
    const el = e.currentTarget as HTMLElement;
    fightRange = scrollRange(el.scrollTop, el.clientHeight, $encounters.length, 2);
  }
  function onZoneOpenChange(open: boolean) {
    if (!open) return;
    const idx = $selectedZoneVisit === null ? -1 : $zoneVisits.findIndex((v) => v.index === $selectedZoneVisit);
    zoneRange = centeredRange(idx, $zoneVisits.length);
  }
  function onFightOpenChange(open: boolean) {
    if (!open) return;
    const idx = $selectedEncounterId === null ? -1 : $encounters.findIndex((e) => e.id === $selectedEncounterId);
    fightRange = centeredRange(idx, $encounters.length);
  }

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
</script>

<div class="flex flex-col gap-4 p-4">
  <div class="flex gap-3">
    <label class="flex flex-1 items-center gap-2 text-[12px]">
      <span class="shrink-0 text-muted-foreground">zone / instance</span>
      <Select.Root
        type="single"
        value={$selectedZoneVisit === null ? '' : String($selectedZoneVisit)}
        onValueChange={onZoneChange}
        onOpenChange={onZoneOpenChange}
      >
        <Select.Trigger class="h-7 flex-1 rounded-sm text-[12px]">{zoneLabel}</Select.Trigger>
        <Select.Content onscroll={onZoneScroll}>
          <Select.Item value="">All zones ({$zoneVisits.reduce((n, v) => n + v.fight_count, 0)})</Select.Item>
          {#if zoneRange.start > 0}
            <div style="height: {zoneRange.start * ROW_H}px" aria-hidden="true"></div>
          {/if}
          {#each $zoneVisits.slice(zoneRange.start, zoneRange.end) as v (v.index ?? 'unknown')}
            <Select.Item value={v.index === null ? '-1' : String(v.index)}>
              {v.current ? '● ' : ''}{v.label} ({v.fight_count})
            </Select.Item>
          {/each}
          {#if zoneRange.end < $zoneVisits.length}
            <div style="height: {($zoneVisits.length - zoneRange.end) * ROW_H}px" aria-hidden="true"></div>
          {/if}
        </Select.Content>
      </Select.Root>
    </label>

    <label class="flex flex-1 items-center gap-2 text-[12px]">
      <span class="shrink-0 text-muted-foreground">fight</span>
      <Select.Root
        type="single"
        value={$followCurrentFight ? 'current' : $selectedEncounterId === null ? '' : String($selectedEncounterId)}
        onValueChange={onEncChange}
        onOpenChange={onFightOpenChange}
      >
        <Select.Trigger class="h-7 flex-1 rounded-sm text-[12px]">{encLabel}</Select.Trigger>
        <Select.Content onscroll={onFightScroll}>
          <Select.Item value="current">Current fight (auto-updates)</Select.Item>
          <Select.Item value="">Aggregate ({$encounters.length} fight{$encounters.length === 1 ? '' : 's'})</Select.Item>
          {#if fightRange.start > 0}
            <div style="height: {fightRange.start * ROW_H}px" aria-hidden="true"></div>
          {/if}
          {#each $encounters.slice(fightRange.start, fightRange.end) as e (e.id)}
            {@const others = e.entities.length > 1 ? ` +${e.entities.length - 1} other${e.entities.length > 2 ? 's' : ''}` : ''}
            {@const tag = e.open ? 'ongoing' : e.slain ? 'kill' : e.wiped ? 'wipe' : 'reset'}
            <Select.Item value={String(e.id)}>
              {e.target}{others} — {fmtDuration(e.duration_ms)} — {e.total_damage.toLocaleString()} dmg ({tag})
            </Select.Item>
          {/each}
          {#if fightRange.end < $encounters.length}
            <div style="height: {($encounters.length - fightRange.end) * ROW_H}px" aria-hidden="true"></div>
          {/if}
        </Select.Content>
      </Select.Root>
    </label>
  </div>

  {#if stats.length}
    <div class="flex divide-x divide-border rounded-sm border border-border bg-card">
      {#each stats as s (s.label)}
        <div class="flex-1 px-4 py-2.5">
          <div class="stat-figure {s.tone}">{s.value}</div>
          <div class="stat-label mt-0.5">{s.label}</div>
        </div>
      {/each}
    </div>
  {/if}

  <div>
    <h2 class="panel-title mb-2">allies · click to see abilities</h2>
    <AllyTable />
  </div>

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

<script lang="ts">
  // why: shows each detected class chain (docs P1-P9) with the encounters that fed it -- the evidence behind a level estimate
  import { Card, CardContent } from '$lib/components/ui/card';
  import SortableTh from '$lib/character/SortableTh.svelte';
  import { debugConfigurations } from '$lib/stores/debug';
  import { api, type ZoneVisitDto } from '$lib/tauri/api';

  type SortKey = 'classes' | 'zone_visits' | 'level_range';
  let sort = $state<{ key: SortKey; dir: 1 | -1 }>({ key: 'zone_visits', dir: -1 });
  function toggle(key: SortKey) {
    sort = sort.key === key ? { key, dir: (sort.dir * -1) as 1 | -1 } : { key, dir: -1 };
  }

  const sorted = $derived.by(() => {
    const rows = $debugConfigurations?.configurations ?? [];
    const { key, dir } = sort;
    return [...rows].sort((a, b) => {
      if (key === 'classes') return dir * a.classes.join().localeCompare(b.classes.join());
      if (key === 'level_range') return dir * ((a.level_range?.[1] ?? -1) - (b.level_range?.[1] ?? -1));
      return dir * (a.zone_visits - b.zone_visits);
    });
  });

  // why: classes alone no longer identifies one row -- a class-set can
  // now split into several real, time-separate sessions (see combat.rs's
  // SESSION_GAP_MS), each its own row sharing the same classes -- so the
  // row key and the drill-down both fold in level_range too.
  function rowKey(classes: string[], levelRange: [number, number] | null): string {
    return `${classes.join(',')}|${levelRange ? levelRange.join('-') : 'none'}`;
  }

  let expanded = $state<string | null>(null);
  let drillVisits = $state<ZoneVisitDto[] | null>(null);
  async function toggleRow(classes: string[], levelRange: [number, number] | null) {
    const key = rowKey(classes, levelRange);
    if (expanded === key) {
      expanded = null;
      return;
    }
    expanded = key;
    drillVisits = null;
    drillVisits = await api.getConfigurationZoneVisits(classes, levelRange);
  }
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h2 class="panel-title mb-1">character · detected class configurations</h2>
    <p class="mb-2 text-[11px] text-muted-foreground">
      Every 3-class loadout ever confirmed for "You", with the encounters and level range that back it -- a class-set that
      recurs across real, widely time-separated sessions (a respec, then coming back to it later) gets its own row per
      session rather than one row spanning the whole gap. Level estimates (Character tab's "Estimate levels") take the
      highest level seen in any row that includes a class, so a single stray row here is exactly what makes one look wrong.
      Click a row for the zone visits those encounters sat in.
    </p>
    {#if !$debugConfigurations}
      <p class="text-[12px] text-muted-foreground">Loading…</p>
    {:else}
      {#if $debugConfigurations.unresolved_visits}
        <p class="mb-2 text-[11px] text-caution">
          {$debugConfigurations.unresolved_visits} encounter{$debugConfigurations.unresolved_visits === 1 ? '' : 's'} sat in a chain that
          never filled all three slots -- not shown as a configuration of their own.
        </p>
      {/if}
      {#if !sorted.length}
        <p class="text-[12px] text-muted-foreground">No confirmed configuration yet.</p>
      {:else}
        <table class="w-full text-[11px]">
          <thead>
            <tr class="border-b border-border">
              <SortableTh label="classes" active={sort.key === 'classes'} dir={sort.dir} onclick={() => toggle('classes')} />
              <SortableTh
                label="encounters"
                align="right"
                active={sort.key === 'zone_visits'}
                dir={sort.dir}
                onclick={() => toggle('zone_visits')}
              />
              <SortableTh
                label="level range"
                align="right"
                active={sort.key === 'level_range'}
                dir={sort.dir}
                onclick={() => toggle('level_range')}
              />
            </tr>
          </thead>
          <tbody>
            {#each sorted as c (rowKey(c.classes, c.level_range))}
              {@const key = rowKey(c.classes, c.level_range)}
              {@const thin = c.zone_visits <= 2}
              <tr
                class="cursor-pointer border-b border-border/50 hover:bg-muted/40 {thin ? 'bg-caution/5' : ''}"
                onclick={() => toggleRow(c.classes, c.level_range)}
              >
                <td class="px-2 py-1">
                  {expanded === key ? '▾' : '▸'}
                  {c.classes.join(' / ')}
                </td>
                <td class="px-2 py-1 text-right tabular-nums {thin ? 'text-caution' : ''}">{c.zone_visits}</td>
                <td class="px-2 py-1 text-right tabular-nums">{c.level_range ? `${c.level_range[0]}–${c.level_range[1]}` : '—'}</td>
              </tr>
              {#if expanded === key}
                <tr>
                  <td colspan="3" class="bg-muted/10 px-2 py-1.5">
                    {#if !drillVisits}
                      <span class="text-muted-foreground">Loading…</span>
                    {:else if !drillVisits.length}
                      <span class="text-muted-foreground">No visits found for this configuration.</span>
                    {:else}
                      <div class="flex flex-col gap-0.5">
                        {#each drillVisits as v (v.index ?? -1)}
                          <div class="flex items-center gap-2">
                            <span class="text-primary">{v.label}</span>
                            <span class="text-muted-foreground">{v.fight_count} fight{v.fight_count === 1 ? '' : 's'}</span>
                            {#if v.current}<span class="text-good">current</span>{/if}
                          </div>
                        {/each}
                      </div>
                    {/if}
                  </td>
                </tr>
              {/if}
            {/each}
          </tbody>
        </table>
      {/if}
    {/if}
  </CardContent>
</Card>

<script lang="ts">
  // why: raw window into what Ingest actually recorded, for verifying zone tagging against real data
  import { Card, CardContent } from '$lib/components/ui/card';
  import SortableTh from '$lib/character/SortableTh.svelte';
  import { debugEncounters } from '$lib/stores/debug';
  import type { DebugEncounterDto } from '$lib/tauri/api';

  type SortKey = 'id' | 'target' | 'start_ms' | 'duration_ms' | 'tier';
  let sort = $state<{ key: SortKey; dir: 1 | -1 }>({ key: 'id', dir: -1 });
  function toggle(key: SortKey) {
    sort = sort.key === key ? { key, dir: (sort.dir * -1) as 1 | -1 } : { key, dir: -1 };
  }

  const sorted = $derived.by(() => {
    const rows = $debugEncounters ?? [];
    const { key, dir } = sort;
    return [...rows].sort((a, b) => {
      const av = a[key];
      const bv = b[key];
      if (typeof av === 'string') return dir * av.localeCompare(bv as string);
      return dir * ((av as number) - (bv as number));
    });
  });

  const missCount = $derived(($debugEncounters ?? []).filter((e) => e.raw_zone && !e.resolved_zone_id).length);
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h2 class="panel-title mb-1">parsed · recent encounters</h2>
    <p class="mb-2 text-[11px] text-muted-foreground">
      {#if $debugEncounters}
        <b class="text-foreground tabular-nums">{$debugEncounters.length}</b> most recent, newest first.
        {#if missCount}
          <span class="text-bad">{missCount} with a raw zone that failed to resolve.</span>
        {:else}
          Every raw zone resolved.
        {/if}
      {:else}
        Loading…
      {/if}
    </p>
    <div class="max-h-[560px] overflow-y-auto rounded-sm border border-border">
      <table class="w-full text-[11px]">
        <thead class="sticky top-0 bg-card">
          <tr class="border-b border-border">
            <SortableTh label="id" active={sort.key === 'id'} dir={sort.dir} onclick={() => toggle('id')} />
            <SortableTh label="target" active={sort.key === 'target'} dir={sort.dir} onclick={() => toggle('target')} />
            <SortableTh label="start" active={sort.key === 'start_ms'} dir={sort.dir} onclick={() => toggle('start_ms')} />
            <SortableTh
              label="duration"
              align="right"
              active={sort.key === 'duration_ms'}
              dir={sort.dir}
              onclick={() => toggle('duration_ms')}
            />
            <th class="px-2 py-0.5 text-left font-normal text-muted-foreground">raw zone</th>
            <th class="px-2 py-0.5 text-left font-normal text-muted-foreground">resolved</th>
            <SortableTh label="tier" align="right" active={sort.key === 'tier'} dir={sort.dir} onclick={() => toggle('tier')} />
            <th class="px-2 py-0.5 text-left font-normal text-muted-foreground">classes</th>
          </tr>
        </thead>
        <tbody>
          {#each sorted as e (e.id)}
            {@const miss = e.raw_zone && !e.resolved_zone_id}
            <tr class="border-b border-border/50 {miss ? 'bg-bad/5' : ''}">
              <td class="px-2 py-0.5 tabular-nums text-muted-foreground">{e.id}</td>
              <td class="px-2 py-0.5">{e.target}</td>
              <td class="px-2 py-0.5 whitespace-nowrap tabular-nums text-muted-foreground">{new Date(e.start_ms).toLocaleString()}</td>
              <td class="px-2 py-0.5 text-right tabular-nums">{(e.duration_ms / 1000).toFixed(1)}s</td>
              <td class="px-2 py-0.5 {e.raw_zone ? '' : 'text-muted-foreground'}">{e.raw_zone ?? '— unknown —'}</td>
              <td class="px-2 py-0.5 {miss ? 'text-bad' : e.resolved_zone_id ? 'text-good' : 'text-muted-foreground'}">
                {e.resolved_zone_id ?? (e.raw_zone ? 'failed to resolve' : '—')}
              </td>
              <td class="px-2 py-0.5 text-right tabular-nums">{e.tier}</td>
              <td class="px-2 py-0.5 {e.player_classes.length ? '' : 'text-muted-foreground'}">{e.player_classes.join(' / ') || '— unresolved —'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </CardContent>
</Card>

<script lang="ts">
  // why: coverage gaps ranked by count -- the live version of `eqlp coverage --top N`
  import { Card, CardContent } from '$lib/components/ui/card';
  import SortableTh from '$lib/character/SortableTh.svelte';
  import { unmatchedCoverage } from '$lib/stores/debug';

  type SortKey = 'shape' | 'count';
  let sort = $state<{ key: SortKey; dir: 1 | -1 }>({ key: 'count', dir: -1 });
  function toggle(key: SortKey) {
    sort = sort.key === key ? { key, dir: (sort.dir * -1) as 1 | -1 } : { key, dir: -1 };
  }

  const sorted = $derived.by(() => {
    const rows = $unmatchedCoverage?.shapes ?? [];
    const { key, dir } = sort;
    return [...rows].sort((a, b) => (key === 'shape' ? dir * a.shape.localeCompare(b.shape) : dir * (a.count - b.count)));
  });

  const pct = $derived.by(() => {
    const c = $unmatchedCoverage;
    if (!c || !c.total_lines) return 0;
    return (100 * c.unmatched_total) / c.total_lines;
  });
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h2 class="panel-title mb-1">unparsed · coverage gaps</h2>
    {#if !$unmatchedCoverage}
      <p class="text-[12px] text-muted-foreground">Loading…</p>
    {:else}
      <div class="mb-2 flex divide-x divide-border rounded-sm border border-border">
        <div class="flex-1 px-3 py-1.5">
          <div class="stat-figure">{$unmatchedCoverage.total_lines.toLocaleString()}</div>
          <div class="stat-label">total lines</div>
        </div>
        <div class="flex-1 px-3 py-1.5">
          <div class="stat-figure {pct > 5 ? 'text-bad' : ''}">{pct.toFixed(1)}%</div>
          <div class="stat-label">unmatched ({$unmatchedCoverage.unmatched_total.toLocaleString()})</div>
        </div>
        <div class="flex-1 px-3 py-1.5">
          <div class="stat-figure">{$unmatchedCoverage.distinct_shapes.toLocaleString()}</div>
          <div class="stat-label">distinct shapes</div>
        </div>
        <div class="flex-1 px-3 py-1.5">
          <div class="stat-figure {$unmatchedCoverage.shapes_overflow ? 'text-caution' : ''}">
            {$unmatchedCoverage.shapes_overflow.toLocaleString()}
          </div>
          <div class="stat-label">lines dropped past shape cap</div>
        </div>
      </div>
      <div class="max-h-[500px] overflow-y-auto rounded-sm border border-border">
        <table class="w-full text-[11px]">
          <thead class="sticky top-0 bg-card">
            <tr class="border-b border-border">
              <SortableTh label="shape" active={sort.key === 'shape'} dir={sort.dir} onclick={() => toggle('shape')} />
              <SortableTh label="count" align="right" active={sort.key === 'count'} dir={sort.dir} onclick={() => toggle('count')} />
              <th class="px-2 py-0.5 text-left font-normal text-muted-foreground">real example</th>
            </tr>
          </thead>
          <tbody>
            {#each sorted as s (s.shape)}
              <tr class="border-b border-border/50">
                <td class="px-2 py-0.5 font-mono text-muted-foreground">{s.shape}</td>
                <td class="px-2 py-0.5 text-right tabular-nums">{s.count.toLocaleString()}</td>
                <td class="px-2 py-0.5 truncate text-primary" title={s.example}>{s.example}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </CardContent>
</Card>

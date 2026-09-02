<script lang="ts">
  import { fmtLogDate } from '$lib/utils';
  // why: past parses vs the selected fight's target, from parse_history.jsonl
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Checkbox } from '$lib/components/ui/checkbox';
  import { historyTarget, historyConfirmedOnly, historyRecords, loadoutSummaries, setHistoryConfirmedOnly } from '$lib/stores/combat';
  import { fmtDuration } from '$lib/format';

  function fmtLoadout(loadout: string[]): string {
    return loadout.length > 0 ? loadout.join(' / ') : '—';
  }
  function fmtRatio(r: number | null): string {
    return r == null ? '—' : `${(r * 100).toFixed(0)}%`;
  }

  // why: derived from the same records the table renders, stays in sync
  const best = $derived($historyRecords.length ? $historyRecords.reduce((a, b) => (b.player_dps > a.player_dps ? b : a)) : null);
</script>

{#if $historyTarget}
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <div class="mb-2 flex items-center justify-between gap-3">
        <h2 class="panel-title">past parses · <span class="text-primary normal-case">{$historyTarget}</span></h2>
        <label class="flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <Checkbox checked={$historyConfirmedOnly} onCheckedChange={(v: boolean) => setHistoryConfirmedOnly(v)} />
          confirmed kills only
        </label>
      </div>

      {#if best}
        <div class="mb-3 flex items-baseline gap-2">
          <span class="stat-figure text-good">{best.player_dps.toFixed(1)}</span>
          <span class="stat-label">highest dps vs target</span>
          <span class="text-[11px] text-muted-foreground">— {best.zone || 'unknown zone'} — {fmtLogDate(best.start_ms)}</span>
        </div>
      {/if}

      <h3 class="stat-label mb-1">bundled by class combination</h3>
      {#if !$loadoutSummaries.length}
        <p class="mb-3 text-[12px] text-muted-foreground">No past parses recorded against this target yet.</p>
      {:else}
        <div class="mb-3 overflow-x-auto">
          <table class="w-full text-[11px]">
            <thead>
              <tr class="border-b border-border text-muted-foreground">
                <th class="px-2 py-0.5 text-left font-normal">loadout</th>
                <th class="px-2 py-0.5 text-right font-normal">fights</th>
                <th class="px-2 py-0.5 text-right font-normal">kills</th>
                <th class="px-2 py-0.5 text-right font-normal">avg dps</th>
                <th class="px-2 py-0.5 text-right font-normal">avg vs. baseline</th>
              </tr>
            </thead>
            <tbody>
              {#each $loadoutSummaries as l (l.loadout.join('/'))}
                <tr class="border-b border-border/50">
                  <td class="px-2 py-0.5">{fmtLoadout(l.loadout)}</td>
                  <td class="px-2 py-0.5 text-right tabular-nums">{l.fights.toLocaleString()}</td>
                  <td class="px-2 py-0.5 text-right tabular-nums">{l.confirmed_kills.toLocaleString()}</td>
                  <td class="px-2 py-0.5 text-right tabular-nums">{l.avg_dps.toFixed(1)}</td>
                  <td class="px-2 py-0.5 text-right tabular-nums">{fmtRatio(l.avg_score_ratio)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}

      <h3 class="stat-label mb-1">every parse</h3>
      {#if !$historyRecords.length}
        <p class="text-[12px] text-muted-foreground">No past parses recorded against this target yet.</p>
      {:else}
        <div class="max-h-80 overflow-y-auto overflow-x-auto rounded-sm border border-border">
          <table class="w-full text-[11px]">
            <thead>
              <tr class="sticky top-0 border-b border-border bg-card text-muted-foreground">
                <th class="px-2 py-0.5 text-left font-normal">when</th>
                <th class="px-2 py-0.5 text-left font-normal">zone</th>
                <th class="px-2 py-0.5 text-left font-normal">loadout</th>
                <th class="px-2 py-0.5 text-right font-normal">duration</th>
                <th class="px-2 py-0.5 text-right font-normal">your dps</th>
                <th class="px-2 py-0.5 text-right font-normal">vs. avg (target, tier)</th>
                <th class="px-2 py-0.5 text-left font-normal">result</th>
              </tr>
            </thead>
            <tbody>
              {#each $historyRecords as r, i (`${r.start_ms}-${i}`)}
                <tr class="border-b border-border/50">
                  <td class="px-2 py-0.5 whitespace-nowrap text-muted-foreground">{new Date(r.start_ms).toLocaleString()}</td>
                  <td class="px-2 py-0.5">{r.zone || '—'}</td>
                  <td class="px-2 py-0.5">{fmtLoadout(r.loadout)}</td>
                  <td class="px-2 py-0.5 text-right tabular-nums">{fmtDuration(r.duration_ms)}</td>
                  <td class="px-2 py-0.5 text-right tabular-nums">{r.player_dps.toFixed(1)}</td>
                  <td class="px-2 py-0.5 text-right tabular-nums">{fmtRatio(r.score_ratio)}</td>
                  <td class="px-2 py-0.5 {r.confirmed_kill ? 'text-good' : 'text-muted-foreground'}">{r.confirmed_kill ? 'kill' : 'reset'}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </CardContent>
  </Card>
{/if}

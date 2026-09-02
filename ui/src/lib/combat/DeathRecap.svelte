<script lang="ts">
  import { fmtLogTime } from '$lib/utils';
  // why: the Death Recap PAGE -- its own activeModule ('deathrecap'),
  // reached through the timed death toast (DeathRecapBanner), not a
  // sidebar tab and not a Combat-page panel (player's own call: the
  // inline version overcrowded Combat). Shows ONE death's recap at a
  // time; the session's other deaths are picker chips, not stacked
  // panels. Refreshes off parse-tick like every other live view.
  import { Card, CardContent } from '$lib/components/ui/card';
  import { api, type DeathRecapDto } from '$lib/tauri/api';
  import { listen } from '$lib/tauri/invoke';
  import { activeModule } from '$lib/stores/shell';
  import { recapPinned, deathList } from '$lib/stores/deathRecap';
  import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';

  let recap = $state<DeathRecapDto | null>(null);

  async function refresh() {
    // why: null-guarded -- the mock harness returns null for a command
    // with no fixture table, and a bare destructure would throw there
    const res = await api.getDeathRecap($recapPinned).catch(() => null);
    if (!res) return;
    recap = res[0];
    deathList.set(res[1]);
  }

  $effect(() => {
    void $recapPinned; // why: re-fetch when the pinned death changes
    void refresh();
    const un = listen('parse-tick', () => void refresh());
    return () => {
      void un.then((f) => f());
    };
  });

  function pick(ts: number) {
    // why: picking the latest death unpins (follow mode); an earlier one pins
    recapPinned.set($deathList[$deathList.length - 1] === ts ? null : ts);
  }

  const fmtTime = (ms: number) => fmtLogTime(ms);
</script>

<div class="flex flex-col gap-4 p-4">
  <div class="flex items-center gap-3">
    <button
      type="button"
      class="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] text-muted-foreground hover:border-foreground/30 hover:text-foreground"
      onclick={() => activeModule.set('combat')}
    >
      <ArrowLeftIcon class="size-3" />
      combat
    </button>
    <h2 class="panel-title">death recap</h2>
  </div>

  {#if !recap}
    <p class="text-[12px] text-muted-foreground">No deaths observed this session. Good.</p>
  {:else}
    {#if $deathList.length > 1}
      <!-- why: one chip per death this session -- a picker, never
           stacked recap panels; latest is follow-mode (unpinned) -->
      <div class="flex flex-wrap gap-1">
        {#each $deathList as d (d)}
          <button
            type="button"
            class="rounded-md border px-1.5 py-0.5 text-[10px] tabular-nums {recap.death_ts_ms === d
              ? 'border-primary text-primary'
              : 'border-border text-muted-foreground hover:text-foreground'}"
            onclick={() => pick(d)}
          >
            {fmtTime(d)}
          </button>
        {/each}
      </div>
    {/if}

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        <div class="mb-2 flex flex-wrap gap-x-4 gap-y-1 text-[12px]">
          <span class="text-muted-foreground"
            >died <span class="tabular-nums text-foreground">{fmtTime(recap.death_ts_ms)}</span></span
          >
          <span class="text-muted-foreground"
            >last {Math.round(recap.window_ms / 1000)}s · took
            <span class="tabular-nums text-bad">{recap.total_incoming.toLocaleString()}</span></span
          >
          <span class="text-muted-foreground"
            >healed <span class="tabular-nums text-good">{recap.total_healed.toLocaleString()}</span></span
          >
          {#if recap.killing_blow}
            <span class="text-muted-foreground">
              killing blow <span class="text-foreground">{recap.killing_blow.source}</span>
              · {recap.killing_blow.ability}
              · <span class="tabular-nums text-bad">{recap.killing_blow.amount.toLocaleString()}</span>
            </span>
          {/if}
        </div>

        <div class="grid grid-cols-1 gap-3 lg:grid-cols-2">
          <div>
            <h4 class="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">incoming</h4>
            <table class="w-full text-[11px]">
              <tbody>
                {#each recap.incoming as r (r.source + r.ability)}
                  <tr class="border-b border-border/50">
                    <td class="py-0.5">{r.source} <span class="text-muted-foreground">· {r.ability}</span></td>
                    <td class="py-0.5 text-right tabular-nums text-bad">{r.total.toLocaleString()}</td>
                    <td class="py-0.5 text-right tabular-nums text-muted-foreground"
                      >{r.hits}x, max {r.max_hit.toLocaleString()}</td
                    >
                    <td class="py-0.5 text-right tabular-nums text-muted-foreground">
                      {#if r.avoided > 0}{r.avoided} avoided{/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
          <div>
            <h4 class="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">heals received</h4>
            {#if recap.heals.length === 0}
              <p class="text-[11px] text-muted-foreground">none in the window</p>
            {:else}
              <table class="w-full text-[11px]">
                <tbody>
                  {#each recap.heals as r (r.source + r.ability)}
                    <tr class="border-b border-border/50">
                      <td class="py-0.5">{r.source} <span class="text-muted-foreground">· {r.ability}</span></td>
                      <td class="py-0.5 text-right tabular-nums text-good">{r.total.toLocaleString()}</td>
                      <td class="py-0.5 text-right tabular-nums text-muted-foreground">{r.hits}x</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            {/if}
          </div>
        </div>
      </CardContent>
    </Card>
  {/if}
</div>

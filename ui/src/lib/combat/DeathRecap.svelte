<script lang="ts">
  // why: "why did I just die" -- the trailing 30s of incoming damage,
  // avoided swings, and heals before a player death, straight off
  // deathrecap.rs. Refreshes on parse-tick like every other Combat
  // panel; hidden entirely until a death has actually been observed
  // this session (most sessions, ideally, never show it).
  import { Card, CardContent } from '$lib/components/ui/card';
  import { api, type DeathRecapDto } from '$lib/tauri/api';
  import { listen } from '$lib/tauri/invoke';

  let recap = $state<DeathRecapDto | null>(null);
  let deaths = $state<number[]>([]);
  // why: null = follow the latest death; a click pins a specific one
  let pinned = $state<number | null>(null);

  async function refresh() {
    // why: null-guarded -- the mock harness returns null for a command
    // with no fixture table, and a bare destructure would throw there
    const res = await api.getDeathRecap(pinned).catch(() => null);
    if (!res) return;
    recap = res[0];
    deaths = res[1];
  }

  $effect(() => {
    void refresh();
    const un = listen('parse-tick', () => void refresh());
    return () => {
      void un.then((f) => f());
    };
  });

  function pick(ts: number) {
    pinned = pinned === ts ? null : ts;
    void refresh();
  }

  const fmtTime = (ms: number) => new Date(ms).toLocaleTimeString();
</script>

{#if recap}
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-2">death recap · last {Math.round(recap.window_ms / 1000)}s</h2>

      {#if deaths.length > 1}
        <!-- why: one chip per death this session, latest followed by
             default -- clicking pins an earlier one, clicking again unpins -->
        <div class="mb-2 flex flex-wrap gap-1">
          {#each deaths as d (d)}
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

      <div class="mb-2 flex flex-wrap gap-x-4 gap-y-1 text-[12px]">
        <span class="text-muted-foreground">died <span class="tabular-nums text-foreground">{fmtTime(recap.death_ts_ms)}</span></span>
        <span class="text-muted-foreground">took <span class="tabular-nums text-bad">{recap.total_incoming.toLocaleString()}</span></span>
        <span class="text-muted-foreground">healed <span class="tabular-nums text-good">{recap.total_healed.toLocaleString()}</span></span>
        {#if recap.killing_blow}
          <span class="text-muted-foreground">
            killing blow <span class="text-foreground">{recap.killing_blow.source}</span>
            · {recap.killing_blow.ability}
            · <span class="tabular-nums text-bad">{recap.killing_blow.amount.toLocaleString()}</span>
          </span>
        {/if}
      </div>

      <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <div>
          <h4 class="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">incoming</h4>
          <table class="w-full text-[11px]">
            <tbody>
              {#each recap.incoming as r (r.source + r.ability)}
                <tr class="border-b border-border/50">
                  <td class="py-0.5">{r.source} <span class="text-muted-foreground">· {r.ability}</span></td>
                  <td class="py-0.5 text-right tabular-nums text-bad">{r.total.toLocaleString()}</td>
                  <td class="py-0.5 text-right tabular-nums text-muted-foreground">{r.hits}x, max {r.max_hit.toLocaleString()}</td>
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

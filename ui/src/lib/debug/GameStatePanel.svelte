<script lang="ts">
  // why: compact live dump of current backend belief -- not a polished
  // feature, a scratchpad view of in-progress state worth eyeballing. Two
  // columns so the two sides of the open fights can be compared at a
  // glance: who the app thinks is with you (and what each has out) against
  // what it thinks is still standing.
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Button } from '$lib/components/ui/button';
  import { gameState, refreshGameState } from '$lib/stores/debug';
  import { refreshOn } from '$lib/tauri/events';
  import type { PartyMemberDto } from '$lib/tauri/api';

  let refreshing = $state(false);
  async function refresh() {
    refreshing = true;
    try {
      await refreshGameState();
    } finally {
      refreshing = false;
    }
  }

  // why: this is a live belief dump, so it follows the log rather than a
  // button. '*' is any parsed line -- a quiet tick costs nothing, and the
  // fan-out upstream is already coalesced to one pass per window.
  $effect(() => refreshOn('*', () => void refreshGameState()));

  // why: labels the evidence channel behind each row -- see
  // eqlp_session::group's own doc for what each channel actually means
  function viaLabel(via: PartyMemberDto['via']): string {
    switch (via) {
      case 'you':
        return 'you';
      case 'joined':
        return 'roster line';
      case 'strong':
        return 'quick buff';
      case 'weak':
        return 'shared target';
    }
  }
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <div class="mb-1 flex items-center justify-between">
      <h2 class="panel-title">game state · live</h2>
      <Button size="sm" variant="ghost" class="h-6 text-[11px]" onclick={refresh} disabled={refreshing}>
        {refreshing ? 'refreshing…' : 'refresh'}
      </Button>
    </div>
    <p class="mb-2 text-[11px] text-muted-foreground">
      What the backend currently believes, following the log as it parses. Left is your side -- party membership
      (GroupTracker) with each member's pets indented under them. Right is what it thinks is still alive against you.
    </p>
    {#if !$gameState}
      <p class="text-[12px] text-muted-foreground">Loading…</p>
    {:else}
      <div class="mb-3 flex divide-x divide-border rounded-sm border border-border">
        <div class="flex-1 px-3 py-1.5">
          <div class="stat-figure">{$gameState.your_classes.length ? $gameState.your_classes.join(' / ') : '—'}</div>
          <div class="stat-label">your classes</div>
        </div>
        <div class="flex-1 px-3 py-1.5">
          <div class="stat-figure">{$gameState.your_level ?? '—'}</div>
          <div class="stat-label">your level</div>
        </div>
        <div class="flex-1 px-3 py-1.5">
          <div class="stat-figure">{$gameState.party.length}</div>
          <div class="stat-label">party members</div>
        </div>
        <div class="flex-1 px-3 py-1.5">
          <div class="stat-figure">{$gameState.enemies.length}</div>
          <div class="stat-label">enemies alive</div>
        </div>
        <div class="flex-1 px-3 py-1.5">
          <div class="stat-figure">{$gameState.known_players}</div>
          <div class="stat-label">known players (whole log)</div>
        </div>
      </div>

      <!-- why: stacks below 900px, where two columns leave ~288px each and
           a charm instance name alone overflows that -->
      <div class="grid grid-cols-1 gap-3 min-[900px]:grid-cols-2">
        <div class="rounded-sm border border-border">
          <p class="border-b border-border px-2 py-1 text-[10px] uppercase tracking-wide text-muted-foreground">
            your side
          </p>
          <ul class="px-2 py-1 text-[11px]">
            {#each $gameState.party as p (p.name)}
              <li class="border-b border-border/50 py-0.5 last:border-0">
                <div class="flex items-baseline justify-between gap-2">
                  <span class="text-primary">{p.name}</span>
                  <span class="shrink-0 text-muted-foreground">
                    {viaLabel(p.via)}{p.via === 'weak' ? ` · ${p.sessions}` : ''}
                  </span>
                </div>
                {#each p.pets as pet (pet.name)}
                  <div class="flex items-baseline justify-between gap-2 pl-4 text-muted-foreground">
                    <span class="font-mono">{pet.name}</span>
                    <span class="shrink-0">{pet.kind}</span>
                  </div>
                {/each}
              </li>
            {/each}
          </ul>
        </div>

        <div class="rounded-sm border border-border">
          <p class="border-b border-border px-2 py-1 text-[10px] uppercase tracking-wide text-muted-foreground">
            still alive against you
          </p>
          {#if !$gameState.enemies.length}
            <p class="px-2 py-1 text-[11px] text-muted-foreground">No open fight.</p>
          {:else}
            <ul class="px-2 py-1 text-[11px]">
              {#each $gameState.enemies as e (e)}
                <li class="border-b border-border/50 py-0.5 font-mono last:border-0">{e}</li>
              {/each}
            </ul>
          {/if}
        </div>
      </div>
    {/if}
  </CardContent>
</Card>

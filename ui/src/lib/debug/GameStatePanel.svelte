<script lang="ts">
  // why: compact live dump of current backend belief -- not a polished
  // feature, a scratchpad view of in-progress state (GroupTracker today,
  // more later) worth eyeballing without a dedicated UI for it yet
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Button } from '$lib/components/ui/button';
  import { gameState, refreshGameState } from '$lib/stores/debug';
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

  // why: labels the evidence channel behind each row -- see
  // eqlp_session::group's own doc for what "quick buff"/"shared target" mean
  function viaLabel(via: PartyMemberDto['via']): string {
    switch (via) {
      case 'you':
        return 'you';
      case 'confirmed':
        return 'confirmed';
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
      What the backend currently believes -- party membership (GroupTracker) and "You"'s own class/level assumption. Grows
      as more backend state becomes worth watching live; not meant to be pretty.
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
      </div>
      <table class="w-full text-[11px]">
        <thead>
          <tr class="border-b border-border text-left text-muted-foreground">
            <th class="px-2 py-0.5 font-normal">name</th>
            <th class="px-2 py-0.5 font-normal">via</th>
            <th class="px-2 py-0.5 text-right font-normal">sessions</th>
          </tr>
        </thead>
        <tbody>
          {#each $gameState.party as p (p.name)}
            <tr class="border-b border-border/50">
              <td class="px-2 py-0.5 text-primary">{p.name}</td>
              <td class="px-2 py-0.5 text-muted-foreground">{viaLabel(p.via)}</td>
              <td class="px-2 py-0.5 text-right tabular-nums text-muted-foreground">{p.via === 'weak' ? p.sessions : '—'}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </CardContent>
</Card>

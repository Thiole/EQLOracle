<script lang="ts">
  // why: no prior design existed for this tab -- a thin composition of
  // data every other module already fetches (character, zone, mob
  // history), not a new backend module. Landing page: "what's going on
  // right now", link out to the module that actually owns each answer.
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Badge } from '$lib/components/ui/badge';
  import { api, type MobDto, type ZoneContextDto } from '$lib/tauri/api';
  import { status } from '$lib/stores/status';
  import { activeModule } from '$lib/stores/shell';
  import { race, activeClasses, levels, currentLevel, loadCharacterModule } from '$lib/stores/character';
  import { displayZoneName } from '$lib/utils';

  $effect(() => {
    void loadCharacterModule();
  });

  let zoneCtx = $state<ZoneContextDto | null>(null);
  let mobs = $state<MobDto[] | null>(null);

  $effect(() => {
    api.getZoneContext().then((z) => (zoneCtx = z));
    api.listMobs().then((list) => (mobs = list ?? []));
  });

  const totalKills = $derived((mobs ?? []).reduce((n, m) => n + m.kills, 0));
  const totalPulls = $derived((mobs ?? []).reduce((n, m) => n + m.pulls, 0));
  const topMobs = $derived((mobs ?? []).slice(0, 5));

  function goto(module: string) {
    activeModule.set(module);
  }
</script>

<div class="flex flex-col gap-3 p-3">
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="stat-figure mb-1.5 text-[18px]">Character</h2>
      {#if $currentLevel === null && !$activeClasses.length}
        <p class="text-[11px] text-muted-foreground">No class evidence parsed yet this session.</p>
      {:else}
        <p class="text-[12px]">
          {#if $race}{$race} · {/if}level {$currentLevel ?? '?'}
        </p>
        {#if $activeClasses.length}
          <p class="mt-0.5 text-[11px] text-muted-foreground">
            {$activeClasses.map((c) => `${c} ${$levels[c] ?? '?'}`).join(' / ')}
          </p>
        {/if}
      {/if}
      <button
        type="button"
        class="mt-2 text-[11px] text-brand-soft hover:text-primary hover:underline"
        onclick={() => goto('character')}
      >
        open Character →
      </button>
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="stat-figure mb-1.5 text-[18px]">Session</h2>
      {#if $status}
        <div class="flex flex-wrap items-center gap-2 text-[11px]">
          <span class="text-muted-foreground">watching</span>
          <span class="font-mono text-foreground">{$status.status.file ?? '—'}</span>
          <Badge variant={$status.status.watching ? 'default' : 'outline'} class="h-5 text-[10px]">
            {$status.status.tail_status}
          </Badge>
        </div>
      {/if}
      <p class="mt-1 text-[12px]">
        {zoneCtx?.current ? displayZoneName(zoneCtx.current) : 'no zone parsed yet'}
      </p>
      <button
        type="button"
        class="mt-2 text-[11px] text-brand-soft hover:text-primary hover:underline"
        onclick={() => goto('maps')}
      >
        open Maps →
      </button>
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="stat-figure mb-1.5 text-[18px]">Loot History</h2>
      {#if !mobs}
        <p class="text-[11px] text-muted-foreground">Loading…</p>
      {:else if !mobs.length}
        <p class="text-[11px] text-muted-foreground">No confirmed pulls yet this session.</p>
      {:else}
        <p class="text-[11px] text-muted-foreground">
          {totalKills.toLocaleString()} kill{totalKills === 1 ? '' : 's'} across {mobs.length.toLocaleString()} mob type{mobs.length === 1
            ? ''
            : 's'} ({totalPulls.toLocaleString()} pull{totalPulls === 1 ? '' : 's'} total)
        </p>
        <ul class="mt-1.5 flex flex-col gap-0.5 text-[11px]">
          {#each topMobs as m (m.name)}
            <li class="flex justify-between">
              <span class="text-foreground">{m.name}</span>
              <span class="text-muted-foreground">{m.kills.toLocaleString()} kill{m.kills === 1 ? '' : 's'}</span>
            </li>
          {/each}
        </ul>
      {/if}
      <button
        type="button"
        class="mt-2 text-[11px] text-brand-soft hover:text-primary hover:underline"
        onclick={() => goto('monsters')}
      >
        open Loot History →
      </button>
    </CardContent>
  </Card>
</div>

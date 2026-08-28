<script lang="ts">
  // why: composes data other modules already fetch (character, zone, mob
  // history) plus overview.rs's own dedicated session-rate stats -- a
  // real backend module (get_session) that already existed, wired to no
  // frontend until now. Landing page: "what's going on right now", link
  // out to the module that actually owns each answer.
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Badge } from '$lib/components/ui/badge';
  import { Button } from '$lib/components/ui/button';
  import { api, type MobDto, type ZoneContextDto } from '$lib/tauri/api';
  import { status } from '$lib/stores/status';
  import { activeModule } from '$lib/stores/shell';
  import { race, activeClasses, defaultClasses, classConfigurations, loadCharacterModule } from '$lib/stores/character';
  import { session, refreshSession, resetSession } from '$lib/stores/session';
  import { displayZoneName } from '$lib/utils';

  $effect(() => {
    void loadCharacterModule();
    void refreshSession();
  });

  let zoneCtx = $state<ZoneContextDto | null>(null);
  let mobs = $state<MobDto[] | null>(null);
  let resetting = $state(false);

  $effect(() => {
    api.getZoneContext().then((z) => (zoneCtx = z));
    api.listMobs().then((list) => (mobs = list ?? []));
  });

  async function restart() {
    resetting = true;
    try {
      await resetSession();
    } finally {
      resetting = false;
    }
  }

  function sameClassSet(a: string[], b: string[]): boolean {
    if (a.length !== b.length) return false;
    const sa = [...a].sort();
    const sb = [...b].sort();
    return sa.every((c, i) => c === sb[i]);
  }

  // why: NOT the Character Planner's `levels` store -- that's a manual,
  // edit-by-hand gear-planning value, seeded once and never re-synced to
  // a real "Welcome to level N!" line. This instead reads
  // classConfigurations' own level_range, which class_configurations.rs
  // computes fresh from real level.up lines strictly inside the
  // confirmed configuration's own active zone-visit windows -- so it
  // reflects a ding attributed to the class you were actually on when it
  // happened, not just the file-wide latest level. Several "sessions" of
  // the same trio can exist (a revisit after a long gap splits into its
  // own entry -- see class_configurations' own SESSION_GAP_MS doc); the
  // highest upper bound across all of them is the real, confirmed peak.
  const dominantLevel = $derived(
    ($classConfigurations?.configurations ?? [])
      .filter((c) => sameClassSet(c.classes, $defaultClasses))
      .reduce<number | null>((best, c) => {
        const hi = c.level_range?.[1] ?? null;
        return hi !== null && (best === null || hi > best) ? hi : best;
      }, null),
  );
  const shownLevel = $derived(dominantLevel ?? $session?.current_level ?? null);
  const shownClasses = $derived($defaultClasses.length ? $defaultClasses : $activeClasses);

  const totalKills = $derived((mobs ?? []).reduce((n, m) => n + m.kills, 0));
  const totalPulls = $derived((mobs ?? []).reduce((n, m) => n + m.pulls, 0));
  const topMobs = $derived((mobs ?? []).slice(0, 5));

  function fmtHours(h: number): string {
    return h < 1 ? `${Math.round(h * 60)}m` : `${h.toFixed(1)}h`;
  }

  function goto(module: string) {
    activeModule.set(module);
  }
</script>

<div class="flex flex-col gap-3 p-3">
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="stat-figure mb-1.5 text-[18px]">Character</h2>
      {#if shownLevel === null && !shownClasses.length}
        <p class="text-[11px] text-muted-foreground">No class evidence parsed yet this session.</p>
      {:else}
        <p class="text-[12px]">
          {#if $race}{$race} · {/if}level {shownLevel ?? '?'}
          {#if $session?.progress_pct != null}
            <span class="text-muted-foreground">({$session.progress_pct.toFixed(1)}% to next)</span>
          {/if}
        </p>
        {#if shownClasses.length}
          <p class="mt-0.5 text-[11px] text-muted-foreground">
            {shownClasses.join(' / ')}{#if !$defaultClasses.length} (not yet confirmed){/if}
          </p>
        {/if}
      {/if}
      <button type="button" class="mt-2 text-[11px] text-brand-soft hover:text-primary hover:underline" onclick={() => goto('character')}>
        open Character →
      </button>
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <div class="mb-1.5 flex items-center justify-between">
        <h2 class="stat-figure text-[18px]">Session</h2>
        <Button size="sm" variant="ghost" class="h-6 text-[11px]" onclick={restart} disabled={resetting} title="Zero out plat/motes/levels/AA and start counting from right now">
          {resetting ? 'restarting…' : 'restart'}
        </Button>
      </div>
      {#if $status}
        <div class="flex flex-wrap items-center gap-2 text-[11px]">
          <span class="text-muted-foreground">watching</span>
          <span class="font-mono text-foreground">{$status.status.file ?? '—'}</span>
          <Badge variant={$status.status.watching ? 'default' : 'outline'} class="h-5 text-[10px]">
            {$status.status.tail_status}
          </Badge>
          {#if $session?.afk}
            <Badge variant="outline" class="h-5 text-[10px]">afk</Badge>
          {/if}
        </div>
      {/if}
      <p class="mt-1 text-[12px]">
        {zoneCtx?.current ? displayZoneName(zoneCtx.current) : 'no zone parsed yet'}
      </p>
      {#if $session?.platinum_per_hour != null || $session?.xp_pct_per_hour != null}
        <p class="mt-1 text-[11px] text-muted-foreground">
          {#if $session.platinum_per_hour != null}{Math.round($session.platinum_per_hour).toLocaleString()}pp/hr{/if}
          {#if $session.platinum_per_hour != null && $session.xp_pct_per_hour != null} · {/if}
          {#if $session.xp_pct_per_hour != null}{$session.xp_pct_per_hour.toFixed(2)}%xp/hr{/if}
          {#if $session.eta_hours != null} · {fmtHours($session.eta_hours)} to next level{/if}
        </p>
      {/if}
      {#if $session && ($session.motes_found > 0 || $session.aa_spent > 0 || $session.levels_gained)}
        <p class="mt-1 text-[11px] text-muted-foreground">
          {$session.motes_found.toLocaleString()} mote{$session.motes_found === 1 ? '' : 's'} found
          {#if $session.motes_per_hour != null}
            <span class="text-muted-foreground/70">({Math.round($session.motes_per_hour)}/hr)</span>
          {/if}
          {#if $session.levels_gained}
            · +{$session.levels_gained} level{$session.levels_gained === 1 ? '' : 's'}
          {/if}
          {#if $session.aa_spent > 0}
            · {$session.aa_spent} AA spent
          {/if}
        </p>
      {/if}
      <button type="button" class="mt-2 text-[11px] text-brand-soft hover:text-primary hover:underline" onclick={() => goto('maps')}>
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
      <button type="button" class="mt-2 text-[11px] text-brand-soft hover:text-primary hover:underline" onclick={() => goto('monsters')}>
        open Loot History →
      </button>
    </CardContent>
  </Card>
</div>

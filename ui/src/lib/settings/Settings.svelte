<script lang="ts">
  import * as Select from '$lib/components/ui/select';
  import { Button } from '$lib/components/ui/button';
  import { Checkbox } from '$lib/components/ui/checkbox';
  import { Card, CardContent } from '$lib/components/ui/card';
  import {
    volume,
    era,
    eraOptions,
    currentEra,
    saveProfile,
    updateChannel,
    settingsLoaded,
    setVolume,
    setEra,
    setSaveProfile,
    setUpdateChannel,
    loadPreferences,
  } from '$lib/stores/settings';
  import { mapPacks, rescanMapFolder } from '$lib/stores/maps';
  import SpellLinePriority from './SpellLinePriority.svelte';

  $effect(() => {
    void loadPreferences();
  });

  // why: local, transient UI state for the rescan button -- not store
  // state, since nothing outside this button cares whether a rescan is
  // in flight or how long ago the last one finished.
  let rescanning = $state(false);
  let rescannedAt = $state<Date | null>(null);

  async function onRescanMaps() {
    rescanning = true;
    try {
      await rescanMapFolder();
      rescannedAt = new Date();
    } finally {
      rescanning = false;
    }
  }

  // why: the dropdown's own display value -- an unsaved preference shows
  // as whatever era is actually live right now, not a blank/unset state.
  const selectedEra = $derived($era ?? $currentEra);
  const eraLabel = (e: string) => (e === $currentEra ? `${e} (current)` : e);
</script>

<div class="flex flex-col gap-3 p-3">
  {#if !$settingsLoaded}
    <p class="text-[12px] text-muted-foreground">Loading…</p>
  {:else}
    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        <h2 class="panel-title mb-1.5">notifications</h2>
        <label class="flex max-w-sm items-center gap-3 text-[12px]">
          <span class="w-16 shrink-0 text-muted-foreground">volume</span>
          <input
            type="range"
            min="0"
            max="100"
            value={$volume}
            oninput={(e) => setVolume(+e.currentTarget.value)}
            class="h-1.5 flex-1 accent-primary"
          />
          <span class="w-9 shrink-0 text-right tabular-nums text-foreground">{$volume}%</span>
        </label>
        <p class="mt-1.5 text-[11px] text-muted-foreground">
          Sets how loud notification sounds play once notifications reach this UI -- not wired up to actual playback yet.
        </p>
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        <h2 class="panel-title mb-1.5">era</h2>
        <label class="flex max-w-sm items-center gap-2 text-[12px]">
          <span class="w-16 shrink-0 text-muted-foreground">era</span>
          <Select.Root type="single" value={selectedEra} onValueChange={(v) => v && setEra(v)}>
            <Select.Trigger class="h-7 flex-1 text-[12px]">{eraLabel(selectedEra)}</Select.Trigger>
            <Select.Content>
              <Select.Item value="All">All eras</Select.Item>
              {#each $eraOptions as e (e)}
                <Select.Item value={e}>{eraLabel(e)}</Select.Item>
              {/each}
            </Select.Content>
          </Select.Root>
        </label>
        <p class="mt-1.5 text-[11px] text-muted-foreground">
          Caps what Game Data and the Gear Planner show to gear/zones/NPCs/spells that exist at or before this era.
          "All eras" turns that off entirely. Defaults to whatever era EQ Legends is actually on right now
          (<b class="text-foreground">{$currentEra}</b>).
        </p>
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        <h2 class="panel-title mb-1.5">character profile</h2>
        <label class="flex items-center gap-1.5 text-[12px]">
          <Checkbox checked={$saveProfile} onCheckedChange={(v: boolean) => setSaveProfile(v)} />
          save your profile across launches
        </label>
        <p class="mt-1.5 text-[11px] text-muted-foreground">
          Off (default): every launch replays the whole log and figures out your classes fresh from what it actually
          sees, same as always. On: also remembers your last-confirmed classes between launches, and falls back to
          them for zone routing on a new launch until this session's own replay reconfirms them itself -- it never
          overrides a class the current session has already confirmed on its own.
        </p>
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        <h2 class="panel-title mb-1.5">update channel</h2>
        <label class="flex max-w-sm items-center gap-2 text-[12px]">
          <span class="w-16 shrink-0 text-muted-foreground">channel</span>
          <Select.Root
            type="single"
            value={$updateChannel}
            onValueChange={(v) => v && setUpdateChannel(v as 'public' | 'beta')}
          >
            <Select.Trigger class="h-7 flex-1 text-[12px]">
              {$updateChannel === 'beta' ? 'Beta' : 'Public'}
            </Select.Trigger>
            <Select.Content>
              <Select.Item value="public">Public</Select.Item>
              <Select.Item value="beta">Beta</Select.Item>
            </Select.Content>
          </Select.Root>
        </label>
        <p class="mt-1.5 text-[11px] text-muted-foreground">
          Public (default): only real, deliberate releases. Beta: every build off the testing branch, ahead of a
          real release but less tested -- expect rough edges.
        </p>
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        <h2 class="panel-title mb-1.5">maps</h2>
        <div class="flex items-center gap-3">
          <Button size="sm" variant="outline" disabled={rescanning} onclick={onRescanMaps}>
            {rescanning ? 'Rescanning…' : 'Rescan maps folder'}
          </Button>
          <p class="text-[11px] text-muted-foreground">
            {$mapPacks.length} pack{$mapPacks.length === 1 ? '' : 's'} known
            {#if rescannedAt}
              · last rescanned {rescannedAt.toLocaleTimeString()}
            {/if}
          </p>
        </div>
        <p class="mt-1.5 text-[11px] text-muted-foreground">
          The Maps module reads zone files fresh every time you open one -- this button is only for the
          <b class="text-foreground">pack list itself</b> (Base game / Brewall / …), which is only checked once per
          launch. Use this if you install a new map pack while the app is already running.
        </p>
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        <SpellLinePriority />
      </CardContent>
    </Card>
  {/if}
</div>

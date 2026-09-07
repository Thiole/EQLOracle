<script lang="ts">
  import * as Select from '$lib/components/ui/select';
  import { HelpTip } from '$lib/components/ui/help';
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
    theme,
    settingsLoaded,
    setVolume,
    setEra,
    setSaveProfile,
    setUpdateChannel,
    setTheme,
    loadPreferences,
  } from '$lib/stores/settings';
  import { mapPacks, rescanMapFolder } from '$lib/stores/maps';
  import { checkForUpdates, availableUpdate, updateCheckError } from '$lib/stores/updater';
  import SpellLinePriority from './SpellLinePriority.svelte';
  import { THEME_CATEGORIES, THEME_SWATCHES, themeName } from './themes';

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

  // why: same local-only shape as rescanning -- checkForUpdates already
  // self-cleans availableUpdate/updateCheckError on every call (see its
  // own doc), this just gates when Settings shows their result so a
  // stale error from the silent launch-time check doesn't leak in here
  // before the user has actually clicked the button themselves.
  let checkingUpdate = $state(false);
  let justCheckedUpdate = $state(false);

  async function onCheckForUpdates() {
    checkingUpdate = true;
    justCheckedUpdate = false;
    await checkForUpdates();
    checkingUpdate = false;
    justCheckedUpdate = true;
  }

  // why: the dropdown's own display value -- an unsaved preference shows
  // as whatever era is actually live right now, not a blank/unset state.
  const selectedEra = $derived($era ?? $currentEra);
  const eraLabel = (e: string) => (e === $currentEra ? `${e} (current)` : e);
</script>

<!-- why: title left, the section's own explanation behind a "?" right --
     the page shows controls, the words are one click away. -->
{#snippet sectionHeader(title: string, help: string)}
  <div class="mb-1.5 flex items-start justify-between gap-2">
    <h2 class="panel-title">{title}</h2>
    <HelpTip label="About {title}" text={help} />
  </div>
{/snippet}

<div class="flex flex-col gap-3 p-3">
  {#if !$settingsLoaded}
    <p class="text-[12px] text-muted-foreground">Loading…</p>
  {:else}
    {#snippet themeSwatch(slug: string)}
      <span class="flex shrink-0 gap-[3px]">
        {#each THEME_SWATCHES[slug] ?? [] as color, i (i)}
          <span class="size-2.5 rounded-full border border-black/20" style="background-color: {color}"></span>
        {/each}
      </span>
    {/snippet}

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        {@render sectionHeader(
          'notifications',
          'Sets how loud notification sounds play once notifications reach this UI -- not wired up to actual playback yet.',
        )}
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
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        <div class="mb-1.5 flex items-start justify-between gap-2">
          <h2 class="panel-title">era</h2>
          <HelpTip label="About era">
            Caps what Game Data, the Gear Planner, GPS destinations, buff suggestions and the spellbook show to
            things that exist at or before this era. "All eras" turns that off entirely. Defaults to whatever era
            EQ Legends is actually on right now (<b class="text-foreground">{$currentEra}</b>).
          </HelpTip>
        </div>
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
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        {@render sectionHeader(
          'character profile',
          "Off (default): every launch replays the whole log and figures out your classes fresh from what it actually sees. On: also remembers your last-confirmed classes between launches, and falls back to them for zone routing until this session's own replay reconfirms them -- it never overrides a class the current session has already confirmed on its own.",
        )}
        <label class="flex items-center gap-1.5 text-[12px]">
          <Checkbox checked={$saveProfile} onCheckedChange={(v: boolean) => setSaveProfile(v)} />
          save your profile across launches
        </label>
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        {@render sectionHeader(
          'update channel',
          'Public (default): only real, deliberate releases. Beta: every build off the testing branch, ahead of a real release but less tested -- expect rough edges.',
        )}
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

        <div class="mt-2.5 flex items-center gap-2">
          <Button size="sm" variant="outline" class="h-7 text-[11px]" disabled={checkingUpdate} onclick={onCheckForUpdates}>
            {checkingUpdate ? 'Checking…' : 'Check for updates'}
          </Button>
          {#if justCheckedUpdate && !checkingUpdate}
            {#if $availableUpdate}
              <span class="text-[11px] text-primary">Update found -- see the prompt in the corner.</span>
            {:else if $updateCheckError}
              <span class="text-[11px] text-bad">Check failed: {$updateCheckError}</span>
            {:else}
              <span class="text-[11px] text-muted-foreground">You're up to date.</span>
            {/if}
          {/if}
        </div>
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        {@render sectionHeader(
          'theme',
          'Recolors the whole app. "Default (brass)" is this app\'s own original look; everything else is a real preset pulled from the shadcn/ui theme ecosystem -- dark variants only, same as this app\'s own always-dark stance.',
        )}
        <label class="flex max-w-sm items-center gap-2 text-[12px]">
          <span class="w-16 shrink-0 text-muted-foreground">theme</span>
          <Select.Root type="single" value={$theme} onValueChange={(v) => v && setTheme(v)}>
            <Select.Trigger class="h-7 flex-1 text-[12px]">
              <span class="flex items-center gap-2">
                {@render themeSwatch($theme)}
                {themeName($theme)}
              </span>
            </Select.Trigger>
            <Select.Content>
              {#each THEME_CATEGORIES as cat (cat.label)}
                <Select.Group>
                  <Select.GroupHeading class="text-[10px] tracking-[0.1em] text-muted-foreground uppercase"
                    >{cat.label}</Select.GroupHeading
                  >
                  {#each cat.themes as t (t.slug)}
                    <Select.Item value={t.slug}>
                      <span class="flex items-center gap-2">
                        {@render themeSwatch(t.slug)}
                        {t.name}
                      </span>
                    </Select.Item>
                  {/each}
                </Select.Group>
              {/each}
            </Select.Content>
          </Select.Root>
        </label>
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        {@render sectionHeader(
          'maps',
          'The Maps module reads zone files fresh every time you open one. This button is only for the pack list itself (Base game / Brewall / ...), which is checked once per launch -- use it if you install a new map pack while the app is running.',
        )}
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
      </CardContent>
    </Card>

    <Card class="rounded-sm">
      <CardContent class="px-3 py-2.5">
        <SpellLinePriority />
      </CardContent>
    </Card>
  {/if}
</div>

<script lang="ts">
  // why: each overlay widget is its own self-contained card -- enable +
  // opacity together, not one shared window-wide toggle/slider, and its
  // own real OS window (see commands::overlay_label's own doc), not
  // content stacked inside one shared overlay surface -- so reposition/
  // lock is per-widget too, not one button for everything. More widgets
  // land as more cards here, each independently on/off, see-through, and
  // positioned.
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Checkbox } from '$lib/components/ui/checkbox';
  import { api } from '$lib/tauri/api';
  import {
    dpsMeterEnabled,
    dpsMeterOpacity,
    setDpsMeterEnabled,
    setDpsMeterOpacity,
    skillTrackerEnabled,
    skillTrackerOpacity,
    setSkillTrackerEnabled,
    setSkillTrackerOpacity,
    trackedSkills,
    toggleTrackedSkill,
    trackedTargetEffects,
    toggleTrackedTargetEffect,
    loadPreferences,
  } from '$lib/stores/settings';
  import { windowCapability, loadWindowCapability } from '$lib/stores/overlay';
  import TrackedSkillsList from './TrackedSkillsList.svelte';

  $effect(() => {
    void loadPreferences();
    void loadWindowCapability();
  });

  let enableError = $state<string | null>(null);
  let skillTrackerError = $state<string | null>(null);
  // why: each widget's own window starts locked (click-through) --
  // matches every widget window's own real default at open
  let locked = $state<Record<string, boolean>>({ dps_meter: true, skill_tracker: true });

  async function onToggleDpsMeter(on: boolean) {
    enableError = null;
    try {
      await setDpsMeterEnabled(on);
      locked.dps_meter = true;
    } catch (e) {
      enableError = e instanceof Error ? e.message : String(e);
    }
  }

  async function onToggleSkillTracker(on: boolean) {
    skillTrackerError = null;
    try {
      await setSkillTrackerEnabled(on);
      locked.skill_tracker = true;
    } catch (e) {
      skillTrackerError = e instanceof Error ? e.message : String(e);
    }
  }

  async function toggleLocked(widget: string) {
    locked[widget] = !locked[widget];
    await api.setOverlayLocked(widget, locked[widget]).catch(() => {});
  }

  const capped = $derived($windowCapability?.capability === 'docked');
</script>

{#snippet repositionButton(widget: string)}
  <button
    type="button"
    class="mt-2 rounded-md border border-border px-2 py-1 text-[11px] text-muted-foreground hover:border-foreground/30 hover:text-foreground"
    onclick={() => void toggleLocked(widget)}
  >
    {locked[widget] ? 'unlock to reposition' : 'lock (click-through) — drag its title bar to move it, then lock'}
  </button>
{/snippet}

{#snippet alphaPreview(opacity: number, onInput: (v: number) => void, disabled: boolean)}
  <div class="mt-2.5 flex items-center gap-3 {disabled ? 'opacity-40' : ''}">
    <input
      type="range"
      min="0.1"
      max="1"
      step="0.05"
      value={opacity}
      {disabled}
      oninput={(e) => onInput(+e.currentTarget.value)}
      class="h-1.5 max-w-64 flex-1 accent-primary"
    />
    <span class="w-10 shrink-0 text-right text-[12px] tabular-nums text-foreground">{Math.round(opacity * 100)}%</span>
    <!-- why: a real alpha-preview checker, not just a number -- lets you see
         how see-through the panel will actually read before it's on screen -->
    <div
      class="h-8 w-16 shrink-0 rounded-sm border border-border"
      style="background-image: repeating-conic-gradient(#3a3d42 0% 25%, #26282c 0% 50%); background-size: 8px 8px;"
    >
      <div class="size-full rounded-[3px]" style:background-color="rgba(10, 11, 13, {opacity})"></div>
    </div>
  </div>
  <p class="mt-1 text-[11px] text-muted-foreground">How see-through this widget's own panel reads over the game.</p>
{/snippet}

<div class="flex flex-col gap-3 p-3">
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-1.5">overlay</h2>
      {#if !$windowCapability}
        <p class="text-[11px] text-muted-foreground">Checking what this session can do…</p>
      {:else if capped}
        <p class="text-[11px] text-caution">{$windowCapability.reason}</p>
        <p class="mt-1 text-[11px] text-muted-foreground">
          The floating overlay isn't available here -- everything below stays saved for whenever it is.
        </p>
      {:else}
        <p class="text-[11px] text-muted-foreground">
          Each widget below is its own little window -- its own on/off, its own transparency, and its own position.
        </p>
      {/if}
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-1.5">DPS meter</h2>
      <label class="flex items-center gap-2 text-[12px] {capped ? 'text-muted-foreground' : 'text-foreground'}">
        <Checkbox checked={$dpsMeterEnabled} disabled={capped} onCheckedChange={(v: boolean) => void onToggleDpsMeter(v)} />
        enable
      </label>
      <p class="mt-0.5 text-[11px] text-muted-foreground">Players and assumed pets, rolling recent-fight damage.</p>
      {#if capped}
        <p class="mt-1 text-[11px] text-muted-foreground">Needs the floating overlay -- see above.</p>
      {/if}
      {#if enableError}
        <p class="mt-1 text-[11px] text-bad">{enableError}</p>
      {/if}
      {#if $dpsMeterEnabled && !capped}
        {@render repositionButton('dps_meter')}
      {/if}

      {@render alphaPreview($dpsMeterOpacity, (v) => void setDpsMeterOpacity(v), capped)}
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-1.5">skill tracker</h2>
      <label class="flex items-center gap-2 text-[12px] {capped ? 'text-muted-foreground' : 'text-foreground'}">
        <Checkbox checked={$skillTrackerEnabled} disabled={capped} onCheckedChange={(v: boolean) => void onToggleSkillTracker(v)} />
        enable
      </label>
      <p class="mt-0.5 text-[11px] text-muted-foreground">Charm, invisibility, hide, and sneak always show; cooldowns below are yours to pick.</p>
      {#if capped}
        <p class="mt-1 text-[11px] text-muted-foreground">Needs the floating overlay -- see above.</p>
      {/if}
      {#if skillTrackerError}
        <p class="mt-1 text-[11px] text-bad">{skillTrackerError}</p>
      {/if}
      {#if $skillTrackerEnabled && !capped}
        {@render repositionButton('skill_tracker')}
      {/if}

      <div class="mt-2.5">
        <p class="text-[11px] text-muted-foreground">
          tracked cooldowns <span class="text-muted-foreground/70">(add an ability from Combat's own breakdown, or track a spell right here)</span>
        </p>
        <div class="mt-1">
          <TrackedSkillsList
            items={$trackedSkills}
            onRemove={(name) => void toggleTrackedSkill(name)}
            ariaLabel="Tracked cooldowns"
            emptyLabel="Nothing tracked yet."
          />
        </div>
      </div>
      <div class="mt-2.5">
        <p class="text-[11px] text-muted-foreground">
          target effects <span class="text-muted-foreground/70">(a DoT or debuff -- landed? how long's left? add spells from Character → Spellbook's own "overlay spell tracking" section)</span>
        </p>
        <div class="mt-1">
          <TrackedSkillsList
            items={$trackedTargetEffects}
            onRemove={(name) => void toggleTrackedTargetEffect(name)}
            ariaLabel="Tracked target effects"
            emptyLabel="Nothing tracked yet."
          />
        </div>
      </div>

      {@render alphaPreview($skillTrackerOpacity, (v) => void setSkillTrackerOpacity(v), capped)}
    </CardContent>
  </Card>

  <Card class="rounded-sm border-dashed">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-1.5 text-muted-foreground">more widgets</h2>
      <p class="text-[11px] text-muted-foreground">More overlay widgets land here as their own cards as they're built.</p>
    </CardContent>
  </Card>
</div>

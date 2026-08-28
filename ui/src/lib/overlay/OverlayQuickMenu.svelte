<script lang="ts">
  // why: a top-bar shortcut for the overlay system -- the full picture
  // (opacity sliders, tracked-item lists, per-widget descriptions) still
  // lives in Settings -> Overlay (OverlaySettings.svelte); this is just
  // "turn it on and see what's running" without digging into a sidebar
  // tab mid-fight. Reuses the exact same settings.ts stores/setters
  // OverlaySettings.svelte does -- both surfaces control the same real
  // windows, neither owns its own copy of the truth.
  //
  // overlayEnabled (settings.ts) is the real gate here: off, every
  // per-widget row below is disabled (greyed, per Checkbox's own
  // disabled:opacity-50) regardless of that widget's own checked state
  // -- you flip the master on right here first, then the individual
  // rows become real toggles. See overlayEnabled's own doc for why it's
  // an explicit flag, not derived from "are all 4 currently on".
  //
  // Deliberately NOT a generic dropdown-menu primitive -- those
  // typically close on any item selection, which is wrong here: ticking
  // one widget's checkbox (or hitting lock/unlock) shouldn't close the
  // whole panel. A plain click-outside listener instead, so only
  // clicking the trigger again or clicking elsewhere on the page closes
  // it.
  import { Checkbox } from '$lib/components/ui/checkbox';
  import { api } from '$lib/tauri/api';
  import {
    overlayEnabled,
    setOverlayEnabledAll,
    dpsMeterEnabled,
    setDpsMeterEnabled,
    skillTrackerEnabled,
    setSkillTrackerEnabled,
    dropWatchEnabled,
    setDropWatchEnabled,
    ccTrackerEnabled,
    setCcTrackerEnabled,
  } from '$lib/stores/settings';
  import { windowCapability, loadWindowCapability } from '$lib/stores/overlay';

  let open = $state(false);
  let panelEl: HTMLDivElement | undefined = $state();
  let buttonEl: HTMLButtonElement | undefined = $state();

  const capped = $derived($windowCapability?.capability === 'docked');

  // why: each widget's own window starts locked (click-through) --
  // matches every widget window's own real default at open, same as
  // OverlaySettings' own `locked` state (a separate copy is fine: both
  // are just this component's own optimistic label, the real lock state
  // lives in the Rust-side window itself)
  let locked = $state<Record<string, boolean>>({
    dps_meter: true,
    skill_tracker: true,
    drop_watch: true,
    cc_tracker: true,
  });

  async function toggleLocked(widget: string) {
    locked[widget] = !locked[widget];
    await api.setOverlayLocked(widget, locked[widget]).catch(() => {});
  }

  function onDocPointerDown(e: PointerEvent) {
    if (!open) return;
    const t = e.target as Node;
    if (panelEl?.contains(t) || buttonEl?.contains(t)) return;
    open = false;
  }

  $effect(() => {
    void loadWindowCapability();
    document.addEventListener('pointerdown', onDocPointerDown);
    return () => document.removeEventListener('pointerdown', onDocPointerDown);
  });
</script>

<!-- why: 4 explicit rows, not a loop over an array of stores -- Svelte's
     `$store` auto-subscription only recognizes a literal store
     identifier at compile time, not a dynamically-picked one
     (`$(row.enabled)` isn't real syntax, it doesn't reactively
     subscribe). Same reason OverlaySettings.svelte itself is 4 explicit
     Card blocks rather than a loop. -->
{#snippet row(id: string, label: string, enabled: boolean, setEnabled: (on: boolean) => void)}
  <div class="flex items-center justify-between gap-2">
    <label class="flex items-center gap-2 {$overlayEnabled ? 'text-foreground' : 'text-muted-foreground'}">
      <Checkbox checked={enabled} disabled={!$overlayEnabled} onCheckedChange={setEnabled} />
      {label}
    </label>
    {#if enabled}
      <button
        type="button"
        disabled={!$overlayEnabled}
        onclick={() => void toggleLocked(id)}
        class="rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
      >
        {locked[id] ? 'unlock' : 'lock'}
      </button>
    {/if}
  </div>
{/snippet}

<div class="relative">
  <button
    type="button"
    bind:this={buttonEl}
    onclick={() => (open = !open)}
    class="flex items-center gap-1.5 rounded-md border px-2 py-1 text-[11px] font-medium transition-colors {$overlayEnabled
      ? 'border-good bg-good text-background'
      : 'border-border text-muted-foreground hover:border-foreground/30 hover:text-foreground'}"
  >
    Overlay
  </button>

  {#if open}
    <div
      bind:this={panelEl}
      class="absolute right-0 top-full z-50 mt-1 w-64 rounded-md border border-border bg-card p-2.5 text-[12px] shadow-lg"
    >
      {#if !$windowCapability}
        <p class="text-[11px] text-muted-foreground">Checking what this session can do…</p>
      {:else if capped}
        <p class="text-[11px] text-caution">{$windowCapability.reason}</p>
      {:else}
        <label class="flex items-center gap-2 font-medium text-foreground">
          <Checkbox checked={$overlayEnabled} onCheckedChange={(v: boolean) => void setOverlayEnabledAll(v)} />
          Overlay enable
        </label>
        <div class="mt-2 flex flex-col gap-1.5 border-t border-border/60 pt-2">
          {@render row('dps_meter', 'DPS meter', $dpsMeterEnabled, (v) => void setDpsMeterEnabled(v))}
          {@render row('skill_tracker', 'Skill Tracker', $skillTrackerEnabled, (v) => void setSkillTrackerEnabled(v))}
          {@render row('drop_watch', 'Drop Watch', $dropWatchEnabled, (v) => void setDropWatchEnabled(v))}
          {@render row('cc_tracker', 'CC Tracker', $ccTrackerEnabled, (v) => void setCcTrackerEnabled(v))}
        </div>
      {/if}
    </div>
  {/if}
</div>

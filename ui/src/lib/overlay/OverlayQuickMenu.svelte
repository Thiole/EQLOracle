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
    sessionWidgetEnabled,
    setSessionWidgetEnabled,
    groupBuffsEnabled,
    setGroupBuffsEnabled,
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
    session: true,
    group_buffs: true,
  });

  // why: reads the widget id off the clicked element's own data-widget
  // attribute, NOT a closure over the snippet's own `id` parameter --
  // real bug, caught live: with 4 rows rendered off the same {#snippet
  // row(...)}, a click was firing with a stale/wrong id (always
  // whichever widget was most recently enabled, not the row actually
  // clicked). Root cause not fully isolated (Svelte 5 snippets are
  // young; a re-render triggered by one store changing may be reusing a
  // handler closure across sibling {@render} calls) -- reading the id
  // from the DOM element itself sidesteps the question entirely: a
  // data-* attribute is tied 1:1 to that one rendered node, there's no
  // closure to go stale.
  function widgetOf(e: Event): string {
    return (e.currentTarget as HTMLElement).dataset.widget ?? '';
  }

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
      <div class="flex shrink-0 items-center gap-1">
        <button
          type="button"
          data-widget={id}
          disabled={!$overlayEnabled}
          onclick={(e) => void toggleLocked(widgetOf(e))}
          class="rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
        >
          {locked[id] ? 'unlock' : 'lock'}
        </button>
        <button
          type="button"
          data-widget={id}
          disabled={!$overlayEnabled}
          onclick={(e) => void api.locateOverlay(widgetOf(e))}
          title="Bring this widget's window to front and flash it"
          class="rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
        >
          locate
        </button>
      </div>
    {/if}
  </div>
{/snippet}

<div class="relative">
  <!-- why: state in the label too, not color alone -- readable colorblind;
       deny color is the same bad token the rest of the app denies with -->
  <button
    type="button"
    bind:this={buttonEl}
    onclick={() => (open = !open)}
    class="flex items-center gap-1.5 rounded-md border px-2 py-1 text-[11px] font-medium transition-colors {$overlayEnabled
      ? 'border-good bg-good text-background'
      : 'border-bad bg-bad/10 text-bad hover:bg-bad/20'}"
  >
    Overlay: {$overlayEnabled ? 'enabled' : 'disabled'}
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
          {@render row('session', 'Session', $sessionWidgetEnabled, (v) => void setSessionWidgetEnabled(v))}
          {@render row('group_buffs', 'Group Buffs', $groupBuffsEnabled, (v) => void setGroupBuffsEnabled(v))}
        </div>
      {/if}
    </div>
  {/if}
</div>

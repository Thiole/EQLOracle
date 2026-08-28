<script lang="ts">
  // why: the overlay window's own separate Svelte app -- a distinct
  // webview/JS realm from the main window (see overlay-main.ts), so it
  // can't share the main window's stores directly. One shared bundle for
  // every overlay widget: each widget is its own real OS window (see
  // commands::overlay_label's own doc), and this component renders
  // exactly the one widget its own window's label names (via
  // currentOverlayWidget) -- not a container stacking several widgets,
  // that's the whole point of the per-window split.
  import {
    api,
    type LiveMeterDto,
    type StatusEffectsDto,
    type SkillStatusDto,
    type TargetEffectsDto,
    type DropWatchRowDto,
  } from '$lib/tauri/api';
  import { listen } from '$lib/tauri/invoke';
  import { currentOverlayWidget } from '$lib/tauri/window';
  import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
  import DpsMeterWidget from './DpsMeterWidget.svelte';
  import SkillTrackerWidget from './SkillTrackerWidget.svelte';
  import DropWatchWidget from './DropWatchWidget.svelte';
  import CCTrackerWidget from './CCTrackerWidget.svelte';
  import { asCcSize, CC_SIZE_WINDOW_DIMS, DEFAULT_CC_SIZE, type CcSize } from './ccSize';

  const widget = currentOverlayWidget();

  let opacity = $state(0.85);
  // why: the SEPARATE "everything" fade -- see PreferencesDto's own doc
  // on overlay_dps_meter_overall_opacity. 1.0 (fully opaque) by default.
  let overallOpacity = $state(1.0);
  let trackedSkillNames = $state<string[]>([]);
  let trackedTargetEffectNames = $state<string[]>([]);
  let trackedDropNames = $state<string[]>([]);
  let meter = $state<LiveMeterDto | null>(null);
  let status = $state<StatusEffectsDto | null>(null);
  let skills = $state<SkillStatusDto[]>([]);
  let targetEffects = $state<TargetEffectsDto | null>(null);
  let dropRows = $state<DropWatchRowDto[]>([]);
  let ccSize = $state<CcSize>(DEFAULT_CC_SIZE);

  async function refreshPrefs() {
    const p = await api.getPreferences();
    // why: overlay theme should match the main window. Same
    // attribute-on-<html> mechanism app.css's themed pages use (see
    // overlay.css's doc for the themes.css import) -- this window is a
    // separate JS realm, can't share stores/settings.ts's
    // theme.subscribe, applies it independently. Re-applied every
    // refreshPrefs() poll (cheap, no-op if unchanged), so a theme
    // switch in Settings takes effect without reopening this window.
    if (typeof document !== 'undefined') {
      document.documentElement.dataset.theme = p.theme;
    }
    if (widget === 'dps_meter') {
      opacity = p.overlay_dps_meter_opacity;
      overallOpacity = p.overlay_dps_meter_overall_opacity;
    } else if (widget === 'skill_tracker') {
      opacity = p.overlay_skill_tracker_opacity;
      overallOpacity = p.overlay_skill_tracker_overall_opacity;
      // why: re-read every tick, not just on mount -- picking a
      // different skill to track in Settings while this window is
      // already open should show up without needing to reopen it, and
      // this list changes rarely enough that re-fetching preferences
      // alongside the data poll is cheap either way
      trackedSkillNames = p.tracked_skills;
      trackedTargetEffectNames = p.tracked_target_effects;
    } else if (widget === 'drop_watch') {
      opacity = p.overlay_drop_watch_opacity;
      overallOpacity = p.overlay_drop_watch_overall_opacity;
      trackedDropNames = p.tracked_drop_items;
    } else if (widget === 'cc_tracker') {
      opacity = p.overlay_cc_tracker_opacity;
      overallOpacity = p.overlay_cc_tracker_overall_opacity;
      // why: NOT resized here -- this only sets the local class/render
      // size. The window's own dimensions are set once at open time by
      // set_overlay_enabled (reading this same persisted value), and
      // live-resized only by the 'overlay-size' listener below, so a
      // plain poll never fights a mid-drag/mid-resize window.
      ccSize = asCcSize(p.overlay_cc_tracker_size);
    }
  }

  async function refresh() {
    if (widget === 'dps_meter') {
      meter = await api.getLiveMeter();
    } else if (widget === 'skill_tracker') {
      const [s, sk, te] = await Promise.all([api.getStatusEffects(), api.getSkillStatus(), api.getTargetEffects()]);
      status = s;
      skills = sk;
      targetEffects = te;
    } else if (widget === 'drop_watch') {
      dropRows = await api.getDropWatch();
    } else if (widget === 'cc_tracker') {
      status = await api.getStatusEffects();
    }
  }

  $effect(() => {
    void refreshPrefs();
    void refresh();
    const unlistenTick = listen('parse-tick', () => {
      void refreshPrefs();
      void refresh();
    });
    const unlistenOpacity = listen<number>('overlay-opacity', (e) => (opacity = e.payload));
    const unlistenOverallOpacity = listen<number>('overlay-overall-opacity', (e) => (overallOpacity = e.payload));
    // why: the one live-push that resizes the real OS window, not just a
    // CSS value -- set_overlay_size (commands.rs) only emits, this
    // window is the one that knows its own new dims (see ccSize.ts's own
    // doc) and calls setSize on itself. Only ever emitted to this
    // window's own label when widget is actually 'cc_tracker' (see
    // overlay_label's own doc), but guarded here too rather than trust
    // that.
    const unlistenSize = listen<string>('overlay-size', (e) => {
      if (widget !== 'cc_tracker') return;
      ccSize = asCcSize(e.payload);
      const { w, h } = CC_SIZE_WINDOW_DIMS[ccSize];
      void getCurrentWindow().setSize(new LogicalSize(w, h));
    });
    return () => {
      void unlistenTick.then((f) => f());
      void unlistenOpacity.then((f) => f());
      void unlistenOverallOpacity.then((f) => f());
      void unlistenSize.then((f) => f());
    };
  });
</script>

<!-- why: NOT data-tauri-drag-region -- a real check against this exact
     stack (XWayland via KWin) found that move request silently doesn't
     move the window (a resize-border drag does). set_overlay_locked
     switches to real decorations instead while unlocked, so dragging
     the actual title bar (every window manager supports that) repositions it. -->
<div class="min-h-screen w-screen p-2">
  {#if widget === 'dps_meter'}
    <DpsMeterWidget {meter} {opacity} {overallOpacity} />
  {:else if widget === 'skill_tracker'}
    <SkillTrackerWidget
      {status}
      {skills}
      {trackedSkillNames}
      {trackedTargetEffectNames}
      {targetEffects}
      {opacity}
      {overallOpacity}
    />
  {:else if widget === 'drop_watch'}
    <DropWatchWidget rows={dropRows} trackedNames={trackedDropNames} {opacity} {overallOpacity} />
  {:else if widget === 'cc_tracker'}
    <CCTrackerWidget {status} {opacity} {overallOpacity} size={ccSize} />
  {/if}
</div>

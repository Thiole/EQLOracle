<script lang="ts">
  // why: the overlay window's own separate Svelte app -- a distinct
  // webview/JS realm from the main window (see overlay-main.ts), so it
  // can't share the main window's stores directly. One shared bundle for
  // every overlay widget: each widget is its own real OS window (see
  // commands::overlay_label's own doc), and this component renders
  // exactly the one widget its own window's label names (via
  // currentOverlayWidget) -- not a container stacking several widgets,
  // that's the whole point of the per-window split.
  import { api, type LiveMeterDto, type StatusEffectsDto, type SkillStatusDto, type TargetEffectsDto } from '$lib/tauri/api';
  import { listen } from '$lib/tauri/invoke';
  import { currentOverlayWidget } from '$lib/tauri/window';
  import DpsMeterWidget from './DpsMeterWidget.svelte';
  import SkillTrackerWidget from './SkillTrackerWidget.svelte';

  const widget = currentOverlayWidget();

  let opacity = $state(0.85);
  let trackedSkillNames = $state<string[]>([]);
  let trackedTargetEffectNames = $state<string[]>([]);
  let meter = $state<LiveMeterDto | null>(null);
  let status = $state<StatusEffectsDto | null>(null);
  let skills = $state<SkillStatusDto[]>([]);
  let targetEffects = $state<TargetEffectsDto | null>(null);

  async function refreshPrefs() {
    const p = await api.getPreferences();
    if (widget === 'dps_meter') opacity = p.overlay_dps_meter_opacity;
    else if (widget === 'skill_tracker') {
      opacity = p.overlay_skill_tracker_opacity;
      // why: re-read every tick, not just on mount -- picking a
      // different skill to track in Settings while this window is
      // already open should show up without needing to reopen it, and
      // this list changes rarely enough that re-fetching preferences
      // alongside the data poll is cheap either way
      trackedSkillNames = p.tracked_skills;
      trackedTargetEffectNames = p.tracked_target_effects;
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
    return () => {
      void unlistenTick.then((f) => f());
      void unlistenOpacity.then((f) => f());
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
    <DpsMeterWidget {meter} {opacity} />
  {:else if widget === 'skill_tracker'}
    <SkillTrackerWidget {status} {skills} {trackedSkillNames} {trackedTargetEffectNames} {targetEffects} {opacity} />
  {/if}
</div>

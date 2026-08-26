<script lang="ts">
  // why: the overlay window's own separate Svelte app -- a distinct
  // webview/JS realm from the main window (see overlay-main.ts), so it
  // can't share the main window's stores directly. One shared bundle for
  // every overlay widget: each widget is its own real OS window (see
  // commands::overlay_label's own doc), and this component renders
  // exactly the one widget its own window's label names (via
  // currentOverlayWidget) -- not a container stacking several widgets,
  // that's the whole point of the per-window split.
  import { api, type LiveMeterDto, type StatusEffectsDto } from '$lib/tauri/api';
  import { listen } from '$lib/tauri/invoke';
  import { currentOverlayWidget } from '$lib/tauri/window';
  import DpsMeterWidget from './DpsMeterWidget.svelte';
  import StatusEffectsWidget from './StatusEffectsWidget.svelte';

  const widget = currentOverlayWidget();

  let opacity = $state(0.85);
  let meter = $state<LiveMeterDto | null>(null);
  let effects = $state<StatusEffectsDto | null>(null);

  async function refresh() {
    if (widget === 'dps_meter') meter = await api.getLiveMeter();
    else if (widget === 'status_effects') effects = await api.getStatusEffects();
  }

  $effect(() => {
    void api.getPreferences().then((p) => {
      if (widget === 'dps_meter') opacity = p.overlay_dps_meter_opacity;
      else if (widget === 'status_effects') opacity = p.overlay_status_effects_opacity;
    });
    void refresh();
    const unlistenTick = listen('parse-tick', () => void refresh());
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
  {:else if widget === 'status_effects'}
    <StatusEffectsWidget status={effects} {opacity} />
  {/if}
</div>

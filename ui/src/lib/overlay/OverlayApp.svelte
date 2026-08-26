<script lang="ts">
  // why: the overlay window's own separate Svelte app -- a distinct
  // webview/JS realm from the main window (see overlay-main.ts), so it
  // can't share the main window's stores directly. Fetches its own
  // initial state and listens for its own events instead. Bare,
  // transparent container -- each widget owns its own panel background/
  // opacity (see DpsMeterWidget's own doc), not one shared window-wide
  // value; more widgets stack here as they're built. Which widgets are
  // actually enabled is real backend state (AppState::overlay_widgets),
  // fetched on mount and kept live via the "overlay-widgets" event --
  // this window can't just assume "I'm open, so my one widget must be
  // why" once a second widget exists.
  import { api, type LiveMeterDto, type StatusEffectsDto } from '$lib/tauri/api';
  import { listen } from '$lib/tauri/invoke';
  import DpsMeterWidget from './DpsMeterWidget.svelte';
  import StatusEffectsWidget from './StatusEffectsWidget.svelte';

  let dpsMeterOpacity = $state(0.85);
  let statusEffectsOpacity = $state(0.85);
  let meter = $state<LiveMeterDto | null>(null);
  let effects = $state<StatusEffectsDto | null>(null);
  let enabledWidgets = $state<Set<string>>(new Set());

  async function refresh() {
    const widgets = enabledWidgets;
    if (widgets.has('dps_meter')) meter = await api.getLiveMeter();
    if (widgets.has('status_effects')) effects = await api.getStatusEffects();
  }

  $effect(() => {
    void api.getPreferences().then((p) => {
      dpsMeterOpacity = p.overlay_dps_meter_opacity;
      statusEffectsOpacity = p.overlay_status_effects_opacity;
    });
    void api.getOverlayEnabledWidgets().then((w) => {
      enabledWidgets = new Set(w);
      void refresh();
    });
    const unlistenTick = listen('parse-tick', () => void refresh());
    const unlistenOpacity = listen<{ widget: string; opacity: number }>('overlay-opacity', (e) => {
      if (e.payload.widget === 'dps_meter') dpsMeterOpacity = e.payload.opacity;
      if (e.payload.widget === 'status_effects') statusEffectsOpacity = e.payload.opacity;
    });
    const unlistenWidgets = listen<string[]>('overlay-widgets', (e) => {
      enabledWidgets = new Set(e.payload);
      void refresh();
    });
    return () => {
      void unlistenTick.then((f) => f());
      void unlistenOpacity.then((f) => f());
      void unlistenWidgets.then((f) => f());
    };
  });
</script>

<!-- why: NOT data-tauri-drag-region -- a real check against this exact
     stack (XWayland via KWin) found that move request silently doesn't
     move the window (a resize-border drag does). set_overlay_locked
     switches to real decorations instead while unlocked, so dragging
     the actual title bar (every window manager supports that) repositions it. -->
<div class="flex min-h-screen w-screen flex-col gap-2 p-2">
  {#if enabledWidgets.has('dps_meter')}
    <DpsMeterWidget {meter} opacity={dpsMeterOpacity} />
  {/if}
  {#if enabledWidgets.has('status_effects')}
    <StatusEffectsWidget status={effects} opacity={statusEffectsOpacity} />
  {/if}
</div>

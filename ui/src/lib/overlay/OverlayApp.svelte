<script lang="ts">
  // why: the overlay window's own separate Svelte app -- a distinct
  // webview/JS realm from the main window (see overlay-main.ts), so it
  // can't share the main window's stores directly. Fetches its own
  // initial state and listens for its own events instead. Bare,
  // transparent container -- each widget owns its own panel background/
  // opacity (see DpsMeterWidget's own doc), not one shared window-wide
  // value; more widgets stack here as they're built.
  import { api, type LiveMeterDto } from '$lib/tauri/api';
  import { listen } from '$lib/tauri/invoke';
  import DpsMeterWidget from './DpsMeterWidget.svelte';

  let dpsMeterOpacity = $state(0.85);
  let meter = $state<LiveMeterDto | null>(null);

  async function refresh() {
    meter = await api.getLiveMeter();
  }

  $effect(() => {
    void api.getPreferences().then((p) => {
      dpsMeterOpacity = p.overlay_dps_meter_opacity;
    });
    void refresh();
    const unlistenTick = listen('parse-tick', () => void refresh());
    const unlistenOpacity = listen<{ widget: string; opacity: number }>('overlay-opacity', (e) => {
      if (e.payload.widget === 'dps_meter') dpsMeterOpacity = e.payload.opacity;
    });
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
  <DpsMeterWidget {meter} opacity={dpsMeterOpacity} />
</div>

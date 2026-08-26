<script lang="ts">
  // why: the overlay window's own separate Svelte app -- a distinct
  // webview/JS realm from the main window (see overlay-main.ts), so it
  // can't share the main window's stores directly. Fetches its own
  // initial state and listens for its own events instead.
  import { api, type LiveMeterDto } from '$lib/tauri/api';
  import { listen } from '$lib/tauri/invoke';
  import DpsMeterWidget from './DpsMeterWidget.svelte';

  let opacity = $state(0.85);
  let dpsMeterOn = $state(true);
  let meter = $state<LiveMeterDto | null>(null);

  async function refresh() {
    meter = await api.getLiveMeter();
  }

  $effect(() => {
    void api.getPreferences().then((p) => {
      opacity = p.overlay_opacity;
      dpsMeterOn = p.overlay_dps_meter;
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

<!-- data-tauri-drag-region: only actually draggable while unlocked (see
     the Overlay tab's own "reposition" toggle) -- click-through ignores
     every pointer event, including this one, while locked. -->
<div
  data-tauri-drag-region
  class="min-h-screen w-screen cursor-move p-2"
  style:background-color="rgba(10, 11, 13, {opacity})"
>
  {#if dpsMeterOn}
    <DpsMeterWidget {meter} />
  {/if}
</div>

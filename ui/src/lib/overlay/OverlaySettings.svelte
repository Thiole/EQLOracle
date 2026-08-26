<script lang="ts">
  // why: each overlay widget is its own self-contained card -- enable +
  // opacity together, not one shared window-wide toggle/slider. More
  // widgets (a party tracker is next) land as more cards here, each
  // independently on/off and independently see-through.
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Checkbox } from '$lib/components/ui/checkbox';
  import { api } from '$lib/tauri/api';
  import { dpsMeterEnabled, dpsMeterOpacity, setDpsMeterEnabled, setDpsMeterOpacity, loadPreferences } from '$lib/stores/settings';
  import { windowCapability, loadWindowCapability } from '$lib/stores/overlay';

  $effect(() => {
    void loadPreferences();
    void loadWindowCapability();
  });

  let enableError = $state<string | null>(null);
  let locked = $state(true);

  async function onToggleDpsMeter(on: boolean) {
    enableError = null;
    try {
      await setDpsMeterEnabled(on);
      locked = true;
    } catch (e) {
      enableError = e instanceof Error ? e.message : String(e);
    }
  }

  async function toggleLocked() {
    locked = !locked;
    await api.setOverlayLocked(locked).catch(() => {});
  }

  const capped = $derived($windowCapability?.capability === 'docked');
  // why: any widget being on means the real window is open -- today
  // that's just the DPS meter, so this is equivalent to $dpsMeterEnabled,
  // but written as an "any of them" check so it doesn't need touching
  // when a second widget exists
  const overlayOpen = $derived($dpsMeterEnabled);
</script>

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
        <p class="text-[11px] text-muted-foreground">Each widget below has its own on/off and its own transparency.</p>
        {#if overlayOpen}
          <button
            type="button"
            class="mt-2 rounded-md border border-border px-2 py-1 text-[11px] text-muted-foreground hover:border-foreground/30 hover:text-foreground"
            onclick={toggleLocked}
          >
            {locked ? 'unlock to reposition' : 'lock (click-through) — drag its title bar to move it, then lock'}
          </button>
        {/if}
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

      <div class="mt-2.5 flex items-center gap-3 {capped ? 'opacity-40' : ''}">
        <input
          type="range"
          min="0.1"
          max="1"
          step="0.05"
          value={$dpsMeterOpacity}
          disabled={capped}
          oninput={(e) => void setDpsMeterOpacity(+e.currentTarget.value)}
          class="h-1.5 max-w-64 flex-1 accent-primary"
        />
        <span class="w-10 shrink-0 text-right text-[12px] tabular-nums text-foreground">{Math.round($dpsMeterOpacity * 100)}%</span>
        <!-- why: a real alpha-preview checker, not just a number -- lets you see
             how see-through the panel will actually read before it's on screen -->
        <div
          class="h-8 w-16 shrink-0 rounded-sm border border-border"
          style="background-image: repeating-conic-gradient(#3a3d42 0% 25%, #26282c 0% 50%); background-size: 8px 8px;"
        >
          <div class="size-full rounded-[3px]" style:background-color="rgba(10, 11, 13, {$dpsMeterOpacity})"></div>
        </div>
      </div>
      <p class="mt-1 text-[11px] text-muted-foreground">How see-through this widget's own panel reads over the game.</p>
    </CardContent>
  </Card>

  <Card class="rounded-sm border-dashed">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-1.5 text-muted-foreground">more widgets</h2>
      <p class="text-[11px] text-muted-foreground">More overlay widgets land here as their own cards as they're built.</p>
    </CardContent>
  </Card>
</div>

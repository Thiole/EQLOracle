<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Checkbox } from '$lib/components/ui/checkbox';
  import { api } from '$lib/tauri/api';
  import {
    overlayEnabled,
    overlayOpacity,
    overlayDpsMeter,
    setOverlayEnabled,
    setOverlayOpacity,
    setOverlayDpsMeter,
    loadPreferences,
  } from '$lib/stores/settings';
  import { windowCapability, loadWindowCapability } from '$lib/stores/overlay';

  $effect(() => {
    void loadPreferences();
    void loadWindowCapability();
  });

  let enableError = $state<string | null>(null);
  let locked = $state(true);

  async function onToggleEnabled(on: boolean) {
    enableError = null;
    try {
      await setOverlayEnabled(on);
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
        <label class="flex items-center gap-2 text-[12px] text-foreground">
          <Checkbox checked={$overlayEnabled} onCheckedChange={(v: boolean) => void onToggleEnabled(v)} />
          show the floating overlay over the game
        </label>
        {#if enableError}
          <p class="mt-1 text-[11px] text-bad">{enableError}</p>
        {/if}
        {#if $overlayEnabled}
          <button
            type="button"
            class="mt-2 rounded-md border border-border px-2 py-1 text-[11px] text-muted-foreground hover:border-foreground/30 hover:text-foreground"
            onclick={toggleLocked}
          >
            {locked ? 'unlock to reposition' : 'lock (click-through) — drag the panel now'}
          </button>
        {/if}
      {/if}
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-1.5">transparency</h2>
      <div class="flex items-center gap-3">
        <input
          type="range"
          min="0.1"
          max="1"
          step="0.05"
          value={$overlayOpacity}
          oninput={(e) => void setOverlayOpacity(+e.currentTarget.value)}
          class="h-1.5 max-w-64 flex-1 accent-primary"
        />
        <span class="w-10 shrink-0 text-right text-[12px] tabular-nums text-foreground">{Math.round($overlayOpacity * 100)}%</span>
        <!-- why: a real alpha-preview checker, not just a number -- lets you see
             how see-through the panel will actually read before it's on screen -->
        <div
          class="h-8 w-16 shrink-0 rounded-sm border border-border"
          style="background-image: repeating-conic-gradient(#3a3d42 0% 25%, #26282c 0% 50%); background-size: 8px 8px;"
        >
          <div class="size-full rounded-[3px]" style:background-color="rgba(10, 11, 13, {$overlayOpacity})"></div>
        </div>
      </div>
      <p class="mt-1.5 text-[11px] text-muted-foreground">How see-through the overlay panel's background reads over the game.</p>
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-1.5">widgets</h2>
      <label class="flex items-center gap-2 text-[12px] text-foreground">
        <Checkbox checked={$overlayDpsMeter} onCheckedChange={(v: boolean) => void setOverlayDpsMeter(v)} />
        DPS meter — players and assumed pets, rolling recent-fight damage
      </label>
      <p class="mt-1.5 text-[11px] text-muted-foreground">More overlay widgets land here as they're built.</p>
    </CardContent>
  </Card>
</div>

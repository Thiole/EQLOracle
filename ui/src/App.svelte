<script lang="ts">
  import { onMount } from 'svelte';
  import FirstLaunch from '$lib/shell/FirstLaunch.svelte';
  import Toolbar from '$lib/shell/Toolbar.svelte';
  import Sidebar from '$lib/shell/Sidebar.svelte';
  import Combat from '$lib/combat/Combat.svelte';
  import Character from '$lib/character/Character.svelte';
  import Endgame from '$lib/endgame/Endgame.svelte';
  import Debug from '$lib/debug/Debug.svelte';
  import Info from '$lib/shell/Info.svelte';
  import GameData from '$lib/gamedata/GameData.svelte';
  import Maps from '$lib/maps/Maps.svelte';
  import Settings from '$lib/settings/Settings.svelte';
  import InventoryDumpBanner from '$lib/shell/InventoryDumpBanner.svelte';
  import UpdateBanner from '$lib/shell/UpdateBanner.svelte';
  import { status, refreshStatus } from '$lib/stores/status';
  import { loadPreferences } from '$lib/stores/settings';
  import { loadGameDataModule } from '$lib/stores/gamedata';
  import { activeModule } from '$lib/stores/shell';
  import { initTauriEvents } from '$lib/tauri/events';
  import { checkForUpdates } from '$lib/stores/updater';

  onMount(() => {
    void refreshStatus();
    void initTauriEvents();
    void loadPreferences();
    // why: loaded here, not on-demand when Game Data first mounts -- the
    // Gear Planner's own item preview links to zone/NPC pages too (see
    // gdOpenPage's own doc), and those links need the catalogs already
    // in memory to know whether a name is real, whichever module the
    // user opens first.
    void loadGameDataModule();
    // why: once per launch, silent on failure (offline is normal) --
    // UpdateBanner only renders once configured, see below
    void checkForUpdates();
  });
</script>

<!-- data-app-root: what ui/tests/interaction/window-identity.spec.ts's
     "exactly one app root is mounted" case looks for. -->
<div data-app-root class="h-screen w-screen overflow-hidden bg-background text-foreground">
  {#if $status === null}
    <!-- Loading -- refreshStatus() hasn't resolved yet. -->
  {:else if !$status.configured}
    <FirstLaunch />
  {:else}
    <div class="flex h-full flex-col">
      <Toolbar />
      <div class="flex flex-1 overflow-hidden">
        <Sidebar bind:active={$activeModule} />
        <main class="flex-1 overflow-y-auto">
          {#if $activeModule === 'combat'}
            <Combat />
          {:else if $activeModule === 'character'}
            <Character />
          {:else if $activeModule === 'endgame'}
            <Endgame />
          {:else if $activeModule === 'debug'}
            <Debug />
          {:else if $activeModule === 'info'}
            <Info />
          {:else if $activeModule === 'gamedata'}
            <GameData />
          {:else if $activeModule === 'maps'}
            <Maps />
          {:else if $activeModule === 'settings'}
            <Settings />
          {/if}
        </main>
      </div>
    </div>
    <InventoryDumpBanner />
    <UpdateBanner />
  {/if}
</div>

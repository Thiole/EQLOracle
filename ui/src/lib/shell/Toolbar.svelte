<script lang="ts">
  import { Badge } from '$lib/components/ui/badge';
  import { api } from '$lib/tauri/api';
  import { status } from '$lib/stores/status';
  import { minimizeWindow, toggleMaximizeWindow, closeWindow } from '$lib/tauri/window';
  import OverlayQuickMenu from '$lib/overlay/OverlayQuickMenu.svelte';
  import ImportMenu from '$lib/shell/ImportMenu.svelte';
  import { HelpTip } from '$lib/components/ui/help';
  import { PAGE_HELP } from '$lib/shell/pageHelp';
  import { activeModule } from '$lib/stores/shell';
  import MinusIcon from '@lucide/svelte/icons/minus';
  import SquareIcon from '@lucide/svelte/icons/square';
  import XIcon from '@lucide/svelte/icons/x';

  // why: Windows runs frameless and this row IS the title bar (drag
  // region + window controls); Linux keeps native decorations and this
  // row stays a plain toolbar -- KWin/XWayland silently drops Tauri's
  // drag-region move request on an undecorated window (confirmed live
  // on this exact machine; same limitation documented on the overlay
  // widget windows, see OverlayApp.svelte's own doc). The backend owns
  // that platform fact -- see commands::get_ui_shell.
  let customTitlebar = $state(false);
  $effect(() => {
    void api.getUiShell().then((s) => (customTitlebar = s?.custom_titlebar ?? false));
  });

  async function changeFolder() {
    const path = await api.pickLogDirectory();
    if (!path) return;
    status.set(await api.setLogDirectory(path));
  }
</script>

<!-- One slim row, titlebar + connection state combined -- the legacy app
     split these into two separate bars; folded into one here since
     "vertical space is expensive" and neither needs its own row. -->
<header
  class="flex h-9 shrink-0 items-center justify-between border-b border-border bg-card pl-3 text-[12px]"
  class:pr-3={!customTitlebar}
>
  <!-- why: Tauri's drag handler fires only when the mousedown target
       ITSELF carries the attribute -- it does not walk up to a parent.
       So every non-interactive part of the bar carries it: the row, the
       title, the "watching" label, the file name, the character, the
       badge and the spacer. Reported real on Windows: with only the
       spacer marked, a long file name shrank it to its 16px minimum and
       the bar had almost nothing left to grab. The interactive children
       (change folder, the menus, the window controls) deliberately do
       not carry it, so they stay clickable. -->
  <!-- why: the page "?" lives here, not over <main> -- absolutely
       positioned inside the page it collided with whatever that page put
       in its own top-right corner (Combat's fight picker). This row is
       always free and is still the top right of the window. -->
  <div
    class="flex min-w-0 flex-1 items-center gap-3 self-stretch"
    data-tauri-drag-region={customTitlebar ? '' : undefined}
    ondblclick={customTitlebar ? toggleMaximizeWindow : undefined}
    role="presentation"
  >
    <span class="shrink-0 font-medium text-foreground" data-tauri-drag-region={customTitlebar ? '' : undefined}>EQL Oracle</span>
    {#if $status}
      <span class="shrink-0 text-muted-foreground" data-tauri-drag-region={customTitlebar ? '' : undefined}>watching</span>
      <span class="truncate font-mono text-foreground" data-tauri-drag-region={customTitlebar ? '' : undefined}>{$status.status.file ?? '—'}</span>
      {#if $status.status.character}
        <span class="shrink-0 text-muted-foreground" data-tauri-drag-region={customTitlebar ? '' : undefined}>{$status.status.character}</span>
      {/if}
      {#if $status.status.backfilling}
        <Badge variant="secondary" class="h-5 shrink-0 text-[10px]">replaying history…</Badge>
      {/if}
    {/if}
    {#if customTitlebar}
      <span
        data-tauri-drag-region
        class="h-full min-w-4 flex-1"
        ondblclick={toggleMaximizeWindow}
        role="presentation"
        data-testid="titlebar-drag-region"
      ></span>
    {/if}
  </div>
  <div class="flex shrink-0 items-center gap-3">
    {#if $status}
      <Badge variant={$status.status.watching ? 'default' : 'outline'} class="h-5 text-[10px]">
        {$status.status.tail_status}
      </Badge>
      <button type="button" class="text-muted-foreground underline-offset-2 hover:text-foreground hover:underline" onclick={changeFolder}>
        change folder
      </button>
    {/if}
    {#if PAGE_HELP[$activeModule]}
      <HelpTip label="About this page" text={PAGE_HELP[$activeModule]} />
    {/if}
    <ImportMenu />
    <OverlayQuickMenu />
    {#if customTitlebar}
      <!-- why: full-height hit targets flush to the window edge, the
           shape every Windows user's muscle memory expects; close gets
           the red hover treatment convention. Close goes through the
           real close path so CloseRequested still runs mark_clean_exit. -->
      <div class="flex h-full items-stretch self-stretch" data-testid="window-controls">
        <button
          type="button"
          class="flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          onclick={minimizeWindow}
          aria-label="Minimize"
        >
          <MinusIcon class="size-3.5" />
        </button>
        <button
          type="button"
          class="flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          onclick={toggleMaximizeWindow}
          aria-label="Maximize"
        >
          <SquareIcon class="size-3" />
        </button>
        <button
          type="button"
          class="flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-destructive hover:text-destructive-foreground"
          onclick={closeWindow}
          aria-label="Close"
        >
          <XIcon class="size-3.5" />
        </button>
      </div>
    {/if}
  </div>
</header>

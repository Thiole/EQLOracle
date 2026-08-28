<script lang="ts">
  import { Badge } from '$lib/components/ui/badge';
  import { api } from '$lib/tauri/api';
  import { status, refreshStatus } from '$lib/stores/status';
  import OverlayQuickMenu from '$lib/overlay/OverlayQuickMenu.svelte';

  async function changeFolder() {
    const path = await api.pickLogDirectory();
    if (!path) return;
    status.set(await api.setLogDirectory(path));
  }
</script>

<!-- One slim row, titlebar + connection state combined -- the legacy app
     split these into two separate bars; folded into one here since
     "vertical space is expensive" and neither needs its own row. -->
<header class="flex h-9 shrink-0 items-center justify-between border-b border-border bg-card px-3 text-[12px]">
  <div class="flex items-center gap-3">
    <span class="font-medium text-foreground">EQL Oracle</span>
    {#if $status}
      <span class="text-muted-foreground">watching</span>
      <span class="font-mono text-foreground">{$status.status.file ?? '—'}</span>
      {#if $status.status.character}
        <span class="text-muted-foreground">{$status.status.character}</span>
      {/if}
      {#if $status.status.backfilling}
        <Badge variant="secondary" class="h-5 text-[10px]">replaying history…</Badge>
      {/if}
    {/if}
  </div>
  <div class="flex items-center gap-3">
    {#if $status}
      <Badge variant={$status.status.watching ? 'default' : 'outline'} class="h-5 text-[10px]">
        {$status.status.tail_status}
      </Badge>
      <button type="button" class="text-muted-foreground underline-offset-2 hover:text-foreground hover:underline" onclick={changeFolder}>
        change folder
      </button>
    {/if}
    <!-- why: rightmost -- closest a web-rendered button gets to "next to
         minimize/maximize/close" on this platform. A real custom title
         bar (decorations: false) was tried first; reverted -- this
         exact machine (KWin/XWayland) silently drops Tauri's
         drag-region move request for an undecorated window, same
         limitation already documented on the overlay widget windows
         (see OverlayApp.svelte's own doc), confirmed live: the window
         became undraggable. Native decorations stay on so dragging by
         the real title bar keeps working; this is one row below it
         instead of beside it. -->
    <OverlayQuickMenu />
  </div>
</header>

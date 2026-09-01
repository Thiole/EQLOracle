<script lang="ts">
  import { fade } from 'svelte/transition';
  import { Input } from '$lib/components/ui/input';
  import CopyIcon from '@lucide/svelte/icons/copy';
  import { pmThreads, pmThreadsError, activePmPlayer, pmHistory, openPmThread, refreshPmThreads } from '$lib/stores/chat';
  import { copyText } from '$lib/clipboard';

  $effect(() => {
    void refreshPmThreads();
  });

  let search = $state('');
  const q = $derived(search.trim().toLowerCase());
  const filteredThreads = $derived(($pmThreads ?? []).filter((t) => !q || t.player.toLowerCase().includes(q)));

  // why: newest-first -- the backend hands back oldest-first (real log
  // order, what pm_history's own tests assert on), reversed only for display
  const historyNewestFirst = $derived($pmHistory ? [...$pmHistory].reverse() : null);

  let copyNote = $state<{ x: number; y: number; text: string } | null>(null);
  let copyNoteTimer: ReturnType<typeof setTimeout> | undefined;

  async function copyTell(event: MouseEvent, player: string) {
    const cmd = `/t ${player}`;
    const ok = await copyText(cmd);
    clearTimeout(copyNoteTimer);
    copyNote = {
      x: event.clientX,
      y: event.clientY,
      text: ok ? `${cmd} copied to clipboard` : 'clipboard copy FAILED',
    };
    copyNoteTimer = setTimeout(() => (copyNote = null), 1400);
  }
</script>

<div class="flex gap-3">
  <div class="flex w-56 shrink-0 flex-col gap-1.5">
    <Input placeholder="search players…" bind:value={search} class="h-7 text-[11px]" />
    <div class="flex h-[520px] flex-col overflow-y-auto rounded-sm border border-border">
      {#if $pmThreadsError}
        <p class="p-1.5 text-[11px] text-bad">Couldn't load PMs: {$pmThreadsError}</p>
      {:else if !$pmThreads}
        <p class="p-1.5 text-[11px] text-muted-foreground">Loading…</p>
      {:else if !$pmThreads.length}
        <p class="p-1.5 text-[11px] text-muted-foreground">No PMs parsed yet this session.</p>
      {:else if !filteredThreads.length}
        <p class="p-1.5 text-[11px] text-muted-foreground">No players match "{search}".</p>
      {:else}
        {#each filteredThreads as t (t.player)}
          <button
            type="button"
            class="flex flex-col gap-0 border-b border-border/50 px-2 py-1.5 text-left last:border-b-0 hover:bg-muted/40 {$activePmPlayer ===
            t.player
              ? 'bg-primary/10'
              : ''}"
            onclick={() => openPmThread(t.player)}
          >
            <span class="text-[12px] font-medium {$activePmPlayer === t.player ? 'text-primary' : 'text-foreground'}">{t.player}</span>
            <span class="truncate text-[10px] text-muted-foreground">{new Date(t.last_ts_ms).toLocaleString()} · {t.last_text}</span>
          </button>
        {/each}
      {/if}
    </div>
  </div>

  <div class="min-w-0 flex-1 rounded-sm border border-border p-2">
    {#if !$activePmPlayer}
      <p class="text-[12px] text-muted-foreground">Select a player to see your history with them.</p>
    {:else}
      <div class="mb-1.5 flex items-center justify-between border-b border-border/50 pb-1.5">
        <span class="text-[12px] font-medium text-foreground">{$activePmPlayer}</span>
        <button
          type="button"
          class="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] text-muted-foreground hover:border-foreground/30 hover:text-foreground"
          title="Copy /t {$activePmPlayer} to your clipboard, to paste in-game"
          onclick={(e) => copyTell(e, $activePmPlayer ?? '')}
        >
          <CopyIcon class="size-3" />
          PM
        </button>
      </div>
      {#if !historyNewestFirst}
        <p class="text-[12px] text-muted-foreground">Loading…</p>
      {:else if !historyNewestFirst.length}
        <p class="text-[12px] text-muted-foreground">No messages with {$activePmPlayer} yet.</p>
      {:else}
        <div class="flex h-[480px] flex-col gap-1.5 overflow-y-auto p-0.5">
          {#each historyNewestFirst as m, i (i)}
            <div class="rounded-md px-2.5 py-1.5 {m.who === 'You' ? 'bg-primary/10' : 'bg-muted/40'}">
              <div class="flex items-baseline gap-2">
                <span class="text-[12px] font-medium {m.who === 'You' ? 'text-primary' : 'text-foreground'}">{m.who}</span>
                <span class="text-[10px] text-muted-foreground">{new Date(m.ts_ms).toLocaleString()}</span>
              </div>
              <p class="mt-0.5 text-[13px] leading-relaxed text-foreground/90">{m.text}</p>
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  </div>
</div>

{#if copyNote}
  <div
    class="pointer-events-none fixed z-50 -translate-x-1/2 -translate-y-full rounded-md border border-border bg-card px-2 py-1 text-[11px] text-foreground shadow-md"
    style="left: {copyNote.x}px; top: {copyNote.y - 10}px;"
    transition:fade={{ duration: 150 }}
  >
    {copyNote.text}
  </div>
{/if}

<script lang="ts">
  import { pmThreads, pmThreadsError, activePmPlayer, pmHistory, openPmThread, refreshPmThreads } from '$lib/stores/chat';

  $effect(() => {
    void refreshPmThreads();
  });
</script>

<div class="flex gap-3">
  <div class="flex w-56 shrink-0 flex-col rounded-sm border border-border">
    {#if $pmThreadsError}
      <p class="p-1.5 text-[11px] text-bad">Couldn't load PMs: {$pmThreadsError}</p>
    {:else if !$pmThreads}
      <p class="p-1.5 text-[11px] text-muted-foreground">Loading…</p>
    {:else if !$pmThreads.length}
      <p class="p-1.5 text-[11px] text-muted-foreground">No PMs parsed yet this session.</p>
    {:else}
      {#each $pmThreads as t (t.player)}
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

  <div class="min-w-0 flex-1 rounded-sm border border-border p-1.5">
    {#if !$activePmPlayer}
      <p class="text-[11px] text-muted-foreground">Select a player to see your history with them.</p>
    {:else if !$pmHistory}
      <p class="text-[11px] text-muted-foreground">Loading…</p>
    {:else if !$pmHistory.length}
      <p class="text-[11px] text-muted-foreground">No messages with {$activePmPlayer} yet.</p>
    {:else}
      <div class="flex max-h-[480px] flex-col gap-0.5 overflow-y-auto">
        {#each $pmHistory as m, i (i)}
          <p class="text-[11px] leading-snug">
            <span class="text-muted-foreground tabular-nums">{new Date(m.ts_ms).toLocaleTimeString()}</span>
            <span class="ml-1.5 font-medium {m.who === 'You' ? 'text-primary' : 'text-foreground'}">{m.who}:</span>
            <span class="ml-1 text-foreground/90">{m.text}</span>
          </p>
        {/each}
      </div>
    {/if}
  </div>
</div>

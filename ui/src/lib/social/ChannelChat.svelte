<script lang="ts">
  // why: same two-column list+detail shape PmList.svelte uses -- Guild/
  // Party/Raid are all "public" channels, one category, picked from a
  // row list exactly like PM's own player list, not a separate tab strip.
  import { guildMessages, partyMessages, raidMessages, channelError, activeChannel, setActiveChannel, refreshChannels, type ChatChannelKind } from '$lib/stores/chat';
  import type { ChatMessageDto } from '$lib/tauri/api';

  $effect(() => {
    void refreshChannels();
  });

  const CHANNELS: { kind: ChatChannelKind; label: string }[] = [
    { kind: 'guild', label: 'Guild' },
    { kind: 'party', label: 'Party' },
    { kind: 'raid', label: 'Raid' },
  ];

  function messagesFor(kind: ChatChannelKind): ChatMessageDto[] | null {
    return kind === 'guild' ? $guildMessages : kind === 'party' ? $partyMessages : $raidMessages;
  }
  function preview(msgs: ChatMessageDto[] | null): string {
    if (!msgs) return 'loading…';
    if (!msgs.length) return 'no messages yet';
    const last = msgs[msgs.length - 1];
    return `${last.who}: ${last.text}`;
  }

  // why: newest-first for display -- the backend hands back oldest-first (real log order)
  const active = $derived.by(() => {
    const msgs = messagesFor($activeChannel);
    return msgs ? [...msgs].reverse() : null;
  });
</script>

<div class="flex gap-3">
  <div class="flex h-[520px] w-56 shrink-0 flex-col overflow-y-auto rounded-sm border border-border">
    {#if $channelError}
      <p class="p-1.5 text-[11px] text-bad">Couldn't load chat: {$channelError}</p>
    {:else}
      {#each CHANNELS as c (c.kind)}
        {@const msgs = messagesFor(c.kind)}
        <button
          type="button"
          class="flex flex-col gap-0 border-b border-border/50 px-2 py-1.5 text-left last:border-b-0 hover:bg-muted/40 {$activeChannel === c.kind
            ? 'bg-primary/10'
            : ''}"
          onclick={() => setActiveChannel(c.kind)}
        >
          <span class="text-[12px] font-medium {$activeChannel === c.kind ? 'text-primary' : 'text-foreground'}">{c.label}</span>
          <span class="truncate text-[10px] text-muted-foreground">{preview(msgs)}</span>
        </button>
      {/each}
    {/if}
  </div>

  <div class="min-w-0 flex-1 rounded-sm border border-border p-2">
    {#if !active}
      <p class="text-[12px] text-muted-foreground">Loading…</p>
    {:else if !active.length}
      <p class="text-[12px] text-muted-foreground">No {$activeChannel} chat parsed yet this session.</p>
    {:else}
      <div class="flex h-[520px] flex-col gap-1.5 overflow-y-auto p-0.5">
        {#each active as m, i (i)}
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
  </div>
</div>

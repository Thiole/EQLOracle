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

  const active = $derived(messagesFor($activeChannel));
</script>

<div class="flex gap-3">
  <div class="flex w-56 shrink-0 flex-col rounded-sm border border-border">
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

  <div class="min-w-0 flex-1 rounded-sm border border-border p-1.5">
    {#if !active}
      <p class="text-[11px] text-muted-foreground">Loading…</p>
    {:else if !active.length}
      <p class="text-[11px] text-muted-foreground">No {$activeChannel} chat parsed yet this session.</p>
    {:else}
      <div class="flex h-[360px] flex-col gap-0.5 overflow-y-auto">
        {#each active as m, i (i)}
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

<script lang="ts">
  import * as Tabs from '$lib/components/ui/tabs';
  import { TAB_LIST_CLASS, TAB_TRIGGER_CLASS } from '$lib/navTabs';
  import { activeChannel, channelMessages, channelError, setActiveChannel, refreshActiveChannel } from '$lib/stores/chat';

  $effect(() => {
    void refreshActiveChannel();
  });

  function onTabChange(v: string | undefined) {
    if (!v) return;
    setActiveChannel(v as 'guild' | 'party' | 'raid');
  }
</script>

<div class="flex flex-col gap-2">
  <Tabs.Root value={$activeChannel} onValueChange={onTabChange}>
    <Tabs.List class={TAB_LIST_CLASS}>
      <Tabs.Trigger value="guild" class={TAB_TRIGGER_CLASS}>Guild</Tabs.Trigger>
      <Tabs.Trigger value="party" class={TAB_TRIGGER_CLASS}>Party</Tabs.Trigger>
      <Tabs.Trigger value="raid" class={TAB_TRIGGER_CLASS}>Raid</Tabs.Trigger>
    </Tabs.List>
  </Tabs.Root>

  {#if $channelError}
    <p class="text-[11px] text-bad">Couldn't load chat: {$channelError}</p>
  {:else if !$channelMessages}
    <p class="text-[11px] text-muted-foreground">Loading…</p>
  {:else if !$channelMessages.length}
    <p class="text-[11px] text-muted-foreground">No {$activeChannel} chat parsed yet this session.</p>
  {:else}
    <div class="flex max-h-[520px] flex-col gap-0.5 overflow-y-auto rounded-sm border border-border p-1.5">
      {#each $channelMessages as m, i (i)}
        <p class="text-[11px] leading-snug">
          <span class="text-muted-foreground tabular-nums">{new Date(m.ts_ms).toLocaleTimeString()}</span>
          <span class="ml-1.5 font-medium {m.who === 'You' ? 'text-primary' : 'text-foreground'}">{m.who}:</span>
          <span class="ml-1 text-foreground/90">{m.text}</span>
        </p>
      {/each}
    </div>
  {/if}
</div>

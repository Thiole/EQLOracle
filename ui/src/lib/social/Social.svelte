<script lang="ts">
  import * as Tabs from '$lib/components/ui/tabs';
  import ChannelChat from './ChannelChat.svelte';
  import PmList from './PmList.svelte';
  import { TAB_LIST_CLASS, TAB_TRIGGER_CLASS } from '$lib/navTabs';

  let sub = $state('channels');
</script>

<div class="flex flex-col gap-3 p-3">
  <Tabs.Root bind:value={sub}>
    <Tabs.List class={TAB_LIST_CLASS}>
      <Tabs.Trigger value="channels" class={TAB_TRIGGER_CLASS}>Channels</Tabs.Trigger>
      <Tabs.Trigger value="pm" class={TAB_TRIGGER_CLASS}>Direct Messages</Tabs.Trigger>
    </Tabs.List>
    <Tabs.Content value="channels"><ChannelChat /></Tabs.Content>
    <Tabs.Content value="pm"><PmList /></Tabs.Content>
  </Tabs.Root>
</div>

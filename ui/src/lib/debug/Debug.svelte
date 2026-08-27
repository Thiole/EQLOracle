<script lang="ts">
  import * as Tabs from '$lib/components/ui/tabs';
  import ParsedPanel from './ParsedPanel.svelte';
  import UnparsedPanel from './UnparsedPanel.svelte';
  import CharacterDebugPanel from './CharacterDebugPanel.svelte';
  import GameStatePanel from './GameStatePanel.svelte';
  import { loadDebugModule } from '$lib/stores/debug';

  let sub = $state('parsed');

  $effect(() => {
    void loadDebugModule();
  });
</script>

<div class="flex flex-col gap-3 p-3">
  <Tabs.Root bind:value={sub}>
    <Tabs.List>
      <Tabs.Trigger value="parsed">Parsed</Tabs.Trigger>
      <Tabs.Trigger value="unparsed">Unparsed</Tabs.Trigger>
      <Tabs.Trigger value="character">Character</Tabs.Trigger>
      <Tabs.Trigger value="gamestate">Game State</Tabs.Trigger>
    </Tabs.List>
    <Tabs.Content value="parsed"><ParsedPanel /></Tabs.Content>
    <Tabs.Content value="unparsed"><UnparsedPanel /></Tabs.Content>
    <Tabs.Content value="character"><CharacterDebugPanel /></Tabs.Content>
    <Tabs.Content value="gamestate"><GameStatePanel /></Tabs.Content>
  </Tabs.Root>
</div>

<script lang="ts">
  import * as Tabs from '$lib/components/ui/tabs';
  import CharacterSheet from './CharacterSheet.svelte';
  import GearPanel from './GearPanel.svelte';
  import AaPanel from './AaPanel.svelte';
  import SpellbookPanel from './SpellbookPanel.svelte';
  import { loadCharacterModule } from '$lib/stores/character';

  let sub = $state('sheet');

  $effect(() => {
    void loadCharacterModule();
  });
</script>

<div class="flex flex-col gap-3 p-3">
  <Tabs.Root bind:value={sub}>
    <Tabs.List>
      <Tabs.Trigger value="sheet">Character</Tabs.Trigger>
      <Tabs.Trigger value="gear">Gear</Tabs.Trigger>
      <Tabs.Trigger value="aa">AA</Tabs.Trigger>
      <Tabs.Trigger value="spellbook">Spellbook</Tabs.Trigger>
    </Tabs.List>
    <Tabs.Content value="sheet"><CharacterSheet /></Tabs.Content>
    <Tabs.Content value="gear"><GearPanel /></Tabs.Content>
    <Tabs.Content value="aa"><AaPanel /></Tabs.Content>
    <Tabs.Content value="spellbook"><SpellbookPanel /></Tabs.Content>
  </Tabs.Root>
</div>

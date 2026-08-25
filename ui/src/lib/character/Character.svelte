<script lang="ts">
  import * as Tabs from '$lib/components/ui/tabs';
  import CharacterSheet from './CharacterSheet.svelte';
  import GearPanel from './GearPanel.svelte';
  import AaPanel from './AaPanel.svelte';
  import SpellbookBuilder from './SpellbookBuilder.svelte';
  import { loadCharacterModule } from '$lib/stores/character';
  import { TAB_LIST_CLASS, TAB_TRIGGER_CLASS } from '$lib/navTabs';

  let sub = $state('sheet');

  $effect(() => {
    void loadCharacterModule();
  });
</script>

<div class="flex flex-col gap-3 p-3">
  <!-- why: Spencer's own order -- Spellbook (the planning tool) sits
       next to Gear, ahead of AA. The old "Known Spells" tab (this
       session's own scribe/memorize evidence) moved to Game Data's own
       Spells tab as a column instead -- it's about the catalog, not
       something to plan, so it fits better linked to the entries it's
       actually about. Styled as separate nav-menu buttons to match
       Endgame/Game Data (see `TAB_TRIGGER_CLASS`'s own doc). -->
  <Tabs.Root bind:value={sub}>
    <Tabs.List class={TAB_LIST_CLASS}>
      <Tabs.Trigger value="sheet" class={TAB_TRIGGER_CLASS}>Character</Tabs.Trigger>
      <Tabs.Trigger value="gear" class={TAB_TRIGGER_CLASS}>Gear</Tabs.Trigger>
      <Tabs.Trigger value="spellbook" class={TAB_TRIGGER_CLASS}>Spellbook</Tabs.Trigger>
      <Tabs.Trigger value="aa" class={TAB_TRIGGER_CLASS}>AA</Tabs.Trigger>
    </Tabs.List>
    <Tabs.Content value="sheet"><CharacterSheet /></Tabs.Content>
    <Tabs.Content value="gear"><GearPanel /></Tabs.Content>
    <Tabs.Content value="spellbook"><SpellbookBuilder /></Tabs.Content>
    <Tabs.Content value="aa"><AaPanel /></Tabs.Content>
  </Tabs.Root>
</div>

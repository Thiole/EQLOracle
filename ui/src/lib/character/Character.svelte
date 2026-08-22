<script lang="ts">
  import * as Tabs from '$lib/components/ui/tabs';
  import CharacterSheet from './CharacterSheet.svelte';
  import GearPanel from './GearPanel.svelte';
  import AaPanel from './AaPanel.svelte';
  import SpellbookPanel from './SpellbookPanel.svelte';
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
       next to Gear, ahead of AA; Known Spells (a passive session log,
       glanced at less) moved last. Styled as separate nav-menu buttons
       to match Endgame/Game Data (see `TAB_TRIGGER_CLASS`'s own doc). -->
  <Tabs.Root bind:value={sub}>
    <Tabs.List class={TAB_LIST_CLASS}>
      <Tabs.Trigger value="sheet" class={TAB_TRIGGER_CLASS}>Character</Tabs.Trigger>
      <Tabs.Trigger value="gear" class={TAB_TRIGGER_CLASS}>Gear</Tabs.Trigger>
      <Tabs.Trigger value="spellbook" class={TAB_TRIGGER_CLASS}>Spellbook</Tabs.Trigger>
      <Tabs.Trigger value="aa" class={TAB_TRIGGER_CLASS}>AA</Tabs.Trigger>
      <!-- why: renamed from "Spellbook" -- this tab is the session's own
           confirmed-spells log (scribe/memorize evidence), not a place
           to plan anything, so "Spellbook" now names the planning tab
           above instead (picking spells into a spellbook, up to 14
           slots). -->
      <Tabs.Trigger value="known-spells" class={TAB_TRIGGER_CLASS}>Known Spells</Tabs.Trigger>
    </Tabs.List>
    <Tabs.Content value="sheet"><CharacterSheet /></Tabs.Content>
    <Tabs.Content value="gear"><GearPanel /></Tabs.Content>
    <Tabs.Content value="spellbook"><SpellbookBuilder /></Tabs.Content>
    <Tabs.Content value="aa"><AaPanel /></Tabs.Content>
    <Tabs.Content value="known-spells"><SpellbookPanel /></Tabs.Content>
  </Tabs.Root>
</div>

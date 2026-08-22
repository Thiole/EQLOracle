<script lang="ts">
  // why: the Endgame module's own tab strip -- Raiding is the first tab;
  // Sky's two turn-in trackers are next, split between the final reward
  // items (Primary Class Unlocks) and the raw material quests that earn
  // them (Sky Quests) -- see skyquests.rs's own module doc for why.
  // Styled as separate nav-menu buttons (see `TAB_TRIGGER_CLASS`'s own
  // doc) rather than the shared Tabs component's default blended-pill
  // look -- Game Data's own tab strip gets the identical treatment for
  // the same reason.
  import * as Tabs from '$lib/components/ui/tabs';
  import { TAB_LIST_CLASS, TAB_TRIGGER_CLASS } from '$lib/navTabs';
  import Raiding from './Raiding.svelte';
  import SkyClassUnlocks from './SkyClassUnlocks.svelte';
  import SkyQuests from './SkyQuests.svelte';

  let sub = $state('raiding');
</script>

<div class="flex flex-col gap-3 p-3">
  <Tabs.Root bind:value={sub}>
    <Tabs.List class={TAB_LIST_CLASS}>
      <Tabs.Trigger value="raiding" class={TAB_TRIGGER_CLASS}>Raiding</Tabs.Trigger>
      <Tabs.Trigger value="sky-unlocks" class={TAB_TRIGGER_CLASS}>Sky - Primary Class Unlocks</Tabs.Trigger>
      <Tabs.Trigger value="sky-quests" class={TAB_TRIGGER_CLASS}>Sky - Quests</Tabs.Trigger>
    </Tabs.List>
    <Tabs.Content value="raiding"><Raiding /></Tabs.Content>
    <Tabs.Content value="sky-unlocks"><SkyClassUnlocks /></Tabs.Content>
    <Tabs.Content value="sky-quests"><SkyQuests /></Tabs.Content>
  </Tabs.Root>
</div>

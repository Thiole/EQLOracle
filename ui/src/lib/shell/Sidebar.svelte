<script lang="ts">
  import { cn } from '$lib/utils';
  import BugIcon from '@lucide/svelte/icons/bug';
  import InfoIcon from '@lucide/svelte/icons/info';
  import SettingsIcon from '@lucide/svelte/icons/settings';

  interface NavItem {
    id: string;
    label: string;
    /** Not wired to a real module yet -- shown, disabled, not a dead end. */
    enabled: boolean;
  }

  const items: NavItem[] = [
    { id: 'overview', label: 'Overview', enabled: true },
    { id: 'combat', label: 'Combat', enabled: true },
    { id: 'monsters', label: 'Loot History', enabled: true },
    { id: 'social', label: 'Social', enabled: true },
    { id: 'character', label: 'Character', enabled: true },
    { id: 'endgame', label: 'Endgame', enabled: true },
    { id: 'gamedata', label: 'Game Data', enabled: true },
    { id: 'skilldata', label: 'Skill Data', enabled: true },
    { id: 'maps', label: 'Maps', enabled: true },
    { id: 'overlay', label: 'Overlay', enabled: true },
  ];

  let { active = $bindable('combat') }: { active?: string } = $props();
</script>

<nav class="flex w-40 shrink-0 flex-col gap-0.5 border-r border-border bg-card p-2" data-slot="sidebar">
  {#each items as item (item.id)}
    <button
      type="button"
      data-module={item.id}
      class={cn(
        'rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors',
        item.enabled ? 'cursor-pointer hover:bg-accent hover:text-accent-foreground' : 'cursor-not-allowed opacity-40',
        active === item.id && item.enabled && 'bg-primary/15 font-medium text-primary',
      )}
      disabled={!item.enabled}
      onclick={() => item.enabled && (active = item.id)}
      title={item.enabled ? undefined : 'Not ported to the new UI yet'}
    >
      {item.label}
    </button>
  {/each}

  <!-- why: debug/settings aren't day-to-day modules -- pinned as a small
       3x2 icon grid at the bottom rather than taking a full-width row
       among the modules actually used while playing. Fixed layout, not
       just append-in-order: info top-middle, debug top-right, settings
       bottom-right, the other 3 cells deliberately blank placeholders
       reserving room for whatever lands here next (row-major grid order
       is what makes blank cells 1/4/5 -- top-left, bottom-left,
       bottom-middle -- land where they do). -->
  <div class="mt-auto grid grid-cols-3 grid-rows-2 gap-1 border-t border-border pt-2">
    <div class="size-8" aria-hidden="true"></div>
    <button
      type="button"
      data-module="info"
      class={cn(
        'flex size-8 items-center justify-center rounded-md transition-colors hover:bg-accent hover:text-accent-foreground',
        active === 'info' && 'bg-primary/15 text-primary',
      )}
      onclick={() => (active = 'info')}
      title="Info"
      aria-label="Info"
    >
      <InfoIcon class="size-4" />
    </button>
    <button
      type="button"
      data-module="debug"
      class={cn(
        'flex size-8 items-center justify-center rounded-md transition-colors hover:bg-accent hover:text-accent-foreground',
        active === 'debug' && 'bg-primary/15 text-primary',
      )}
      onclick={() => (active = 'debug')}
      title="Debug"
      aria-label="Debug"
    >
      <BugIcon class="size-4" />
    </button>
    <div class="size-8" aria-hidden="true"></div>
    <div class="size-8" aria-hidden="true"></div>
    <button
      type="button"
      data-module="settings"
      class={cn(
        'flex size-8 items-center justify-center rounded-md transition-colors hover:bg-accent hover:text-accent-foreground',
        active === 'settings' && 'bg-primary/15 text-primary',
      )}
      onclick={() => (active = 'settings')}
      title="Settings"
      aria-label="Settings"
    >
      <SettingsIcon class="size-4" />
    </button>
  </div>
</nav>

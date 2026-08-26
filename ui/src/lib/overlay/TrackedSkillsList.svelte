<script lang="ts">
  // why: shared between Overlay settings and Spellbook's own tracking
  // sections -- one real listbox implementation, not two copies of the
  // same markup drifting apart. Parameterized over which list it's
  // showing (trackedSkills for cooldowns, or trackedTargetEffects for
  // the per-target panel -- see stores/settings.ts's own doc on why
  // those are two separate lists now) rather than hardcoding one.
  // Click a row to select/highlight it (own keyboard support too), its
  // own × removes it independent of selection.
  import XIcon from '@lucide/svelte/icons/x';

  let {
    items,
    onRemove,
    ariaLabel,
    emptyLabel,
  }: {
    items: string[];
    onRemove: (name: string) => void;
    ariaLabel: string;
    emptyLabel: string;
  } = $props();

  let selected = $state<string | null>(null);
</script>

<div role="listbox" aria-label={ariaLabel} class="max-h-32 overflow-y-auto rounded-sm border border-border">
  {#if !items.length}
    <p class="px-2 py-3 text-center text-[11px] text-muted-foreground italic">{emptyLabel}</p>
  {:else}
    {#each items as skill (skill)}
      <div
        role="option"
        aria-selected={selected === skill}
        tabindex="0"
        class="flex items-center justify-between gap-2 border-b border-border/50 px-2 py-1 text-[12px] last:border-b-0 {selected ===
        skill
          ? 'bg-primary/10 text-primary'
          : 'text-foreground hover:bg-muted/40'}"
        onclick={() => (selected = selected === skill ? null : skill)}
        onkeydown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') selected = selected === skill ? null : skill;
        }}
      >
        <span class="truncate">{skill}</span>
        <button
          type="button"
          class="shrink-0 rounded-sm p-0.5 text-muted-foreground hover:bg-bad/20 hover:text-bad"
          title="Stop tracking {skill}"
          onclick={(e) => {
            e.stopPropagation();
            onRemove(skill);
          }}
        >
          <XIcon class="size-3" />
        </button>
      </div>
    {/each}
  {/if}
</div>

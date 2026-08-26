<script lang="ts">
  // why: shared between Overlay settings and Spellbook's own "overlay
  // spell tracking" section -- one real listbox reflecting
  // stores/settings.ts's trackedSkills, not two copies of the same
  // markup drifting apart. Click a row to select/highlight it (own
  // keyboard support too), its own × removes it independent of selection.
  import XIcon from '@lucide/svelte/icons/x';
  import { trackedSkills, toggleTrackedSkill } from '$lib/stores/settings';

  let selected = $state<string | null>(null);
</script>

<div role="listbox" aria-label="Tracked cooldowns" class="max-h-32 overflow-y-auto rounded-sm border border-border">
  {#if !$trackedSkills.length}
    <p class="px-2 py-3 text-center text-[11px] text-muted-foreground italic">Nothing tracked yet.</p>
  {:else}
    {#each $trackedSkills as skill (skill)}
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
            void toggleTrackedSkill(skill);
          }}
        >
          <XIcon class="size-3" />
        </button>
      </div>
    {/each}
  {/if}
</div>

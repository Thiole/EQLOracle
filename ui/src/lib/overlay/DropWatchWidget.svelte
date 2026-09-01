<script lang="ts">
  // why: "hey, you're fighting something that might drop what you're
  // after" -- see dropwatch.rs's own doc. Rows are the intersection of
  // rows.drops with trackedNames, never the mob's whole loot table; a
  // mob with no overlap contributes nothing here at all, same filter
  // shape SkillTrackerWidget already applies to visibleSkills.
  import type { DropWatchRowDto } from '$lib/tauri/api';

  let {
    rows,
    trackedNames,
    opacity,
    overallOpacity,
  }: {
    rows: DropWatchRowDto[];
    trackedNames: string[];
    opacity: number;
    overallOpacity: number;
  } = $props();

  const matches = $derived(
    rows
      .map((r) => ({ mob: r.mob, drops: r.drops.filter((d) => trackedNames.includes(d)) }))
      .filter((r) => r.drops.length > 0),
  );
</script>

<!-- why: same legibility treatment as every other overlay widget --
     bolder text + dark shadow so it reads against the game itself at
     background opacity 0, theme --background via color-mix so it
     matches whatever theme is active. -->
<div
  class="flex flex-col gap-1.5 rounded-md p-2 text-[12px] font-semibold"
  style:background-color="color-mix(in srgb, var(--background) {opacity * 100}%, transparent)"
  style:opacity={overallOpacity}
  style:text-shadow="0 1px 2px rgba(0, 0, 0, 0.9), 0 0px 4px rgba(0, 0, 0, 0.6)"
>
  {#if !trackedNames.length}
    <!-- why: only shown while genuinely empty -- points at the bell
         icon once, doesn't repeat itself once something's tracked -->
    <p class="text-muted-foreground">Nothing tracked yet -- click the bell on an item in Sky Quests.</p>
  {:else if !matches.length}
    <p class="text-muted-foreground">no tracked drops nearby</p>
  {:else}
    {#each matches as m (m.mob)}
      <div class="flex flex-col gap-0.5">
        <span class="truncate text-foreground">{m.mob}</span>
        <!-- why: wrap, don't truncate -- a long drop list was silently
             cutting off past the widget's right edge; each name stays
             whole on its line (only breaks BETWEEN drops) -->
        <span class="flex flex-wrap gap-x-1 text-[11px] font-normal text-good">
          <span>drops</span>
          {#each m.drops as d, i (d)}
            <span class="whitespace-nowrap">{d}{i < m.drops.length - 1 ? ',' : ''}</span>
          {/each}
        </span>
      </div>
    {/each}
  {/if}
</div>

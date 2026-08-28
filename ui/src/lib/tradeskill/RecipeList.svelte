<script lang="ts">
  // why: one tradeskill's own recipe browser -- sorted by trivial
  // ascending on purpose (asked directly for "how to train it": the
  // natural training path *is* cheapest-to-skill-up-on first, not
  // alphabetical). Real craft-log stats (attempts/successes/capped)
  // join in per recipe when you've actually made it.
  import { Input } from '$lib/components/ui/input';
  import GdLink from '$lib/gamedata/GdLink.svelte';
  import type { RecipeDto, CraftLogEntryDto } from '$lib/tauri/api';
  import { craftedVia } from '$lib/stores/tradeskill';

  let {
    recipes,
    craftLogByItem,
    onJumpToSkill,
  }: {
    recipes: RecipeDto[];
    craftLogByItem: Map<string, CraftLogEntryDto>;
    onJumpToSkill: (skill: string) => void;
  } = $props();

  let search = $state('');
  const q = $derived(search.trim().toLowerCase());

  const filtered = $derived(
    (q ? recipes.filter((r) => r.item.toLowerCase().includes(q)) : recipes)
      .slice()
      .sort((a, b) => (a.trivial ?? 9999) - (b.trivial ?? 9999) || a.item.localeCompare(b.item)),
  );

  function trivialLabel(r: RecipeDto): string {
    if (r.trivial != null) return String(r.trivial);
    return r.trivial_raw ?? '—';
  }
</script>

<div class="flex flex-col gap-2">
  <div class="flex items-center justify-between gap-2">
    <p class="text-[11px] text-muted-foreground">
      {recipes.length} recipe{recipes.length === 1 ? '' : 's'} · sorted by trivial (cheapest to skill up on first)
    </p>
    <Input placeholder="filter recipes…" bind:value={search} class="h-7 w-56 text-[11px]" />
  </div>

  {#if !filtered.length}
    <p class="text-[11px] text-muted-foreground">No recipes match "{search}".</p>
  {:else}
    <div class="flex flex-col rounded-sm border border-border">
      {#each filtered as r (r.item + '::' + JSON.stringify(r.ingredients))}
        {@const log = craftLogByItem.get(r.item.toLowerCase())}
        <div class="border-b border-border/50 px-2.5 py-1.5 last:border-b-0">
          <div class="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5">
            <span class="text-[12px] font-medium text-foreground">
              <GdLink kind="item" name={r.item} />
              {#if r.yield_qty > 1}<span class="text-muted-foreground">×{r.yield_qty}</span>{/if}
            </span>
            <span class="flex items-baseline gap-2 text-[11px] text-muted-foreground">
              <span>trivial {trivialLabel(r)}</span>
              {#if log}
                <span class={log.skill_capped ? 'text-caution' : ''}>
                  you: {log.successes}/{log.attempts}{log.skill_capped ? ' · capped' : ''}
                </span>
              {/if}
            </span>
          </div>
          <div class="mt-0.5 flex flex-wrap items-center gap-x-1 gap-y-0.5 text-[11px] text-muted-foreground">
            <span>needs</span>
            {#each r.ingredients as ing, i (ing.item + i)}
              {@const via = craftedVia(ing.item)}
              <span>
                {#if i > 0},{/if}
                {ing.qty > 1 ? `${ing.qty}× ` : ''}<GdLink kind="item" name={ing.item} />{ing.returned ? ' (returned)' : ''}
                {#if via}
                  <button
                    type="button"
                    class="text-brand-soft hover:text-primary hover:underline"
                    title="Craftable via {via}"
                    onclick={() => onJumpToSkill(via)}
                  >
                    ({via})
                  </button>
                {/if}
              </span>
            {/each}
            {#if r.implements}<span>· {r.implements}</span>{/if}
            {#if r.use}<span>· {r.use}</span>{/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

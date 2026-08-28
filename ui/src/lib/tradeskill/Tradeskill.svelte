<script lang="ts">
  // why: one tab per core tradeskill (confirmed real via Category:Tradeskills
  // -- the 11 race-specific "Cultural Tradeskill" pages are a different,
  // one-off epic-quest-adjacent thing, not covered here) plus a central
  // "Overview" tab: your real craft log (what you've actually made) and
  // a per-skill recipe-count summary, both linking straight into the
  // matching skill's own tab -- the "crosslinks" asked for directly.
  import * as Tabs from '$lib/components/ui/tabs';
  import { Card, CardContent } from '$lib/components/ui/card';
  import GdLink from '$lib/gamedata/GdLink.svelte';
  import RecipeList from './RecipeList.svelte';
  import { tradeskillCatalog, craftLog, loadTradeskillModule } from '$lib/stores/tradeskill';
  import { TAB_LIST_CLASS, TAB_TRIGGER_CLASS } from '$lib/navTabs';

  // why: TAB_LIST_CLASS's own base (inline-flex, no wrap) is fine for
  // the 5-ish tabs Character/Endgame/Game Data each have -- 10 here
  // (Overview + 9 skills) needs to wrap to a second row instead of
  // overflowing, so this extends it rather than editing the shared
  // constant every other tab strip also uses
  const TRADESKILL_TAB_LIST_CLASS = `flex-wrap ${TAB_LIST_CLASS}`;

  let sub = $state('overview');

  $effect(() => {
    void loadTradeskillModule();
  });

  const craftLogByItem = $derived(new Map($craftLog.map((e) => [e.item.toLowerCase(), e])));

  function jumpTo(skill: string) {
    sub = skill;
  }

  const totalAttempts = $derived($craftLog.reduce((n, e) => n + e.attempts, 0));
  const totalRecipes = $derived($tradeskillCatalog.reduce((n, s) => n + s.recipes.length, 0));
  const cappedCount = $derived($craftLog.filter((e) => e.skill_capped).length);
</script>

<div class="flex flex-col gap-3 p-3">
  {#if !$tradeskillCatalog.length}
    <p class="text-[12px] text-muted-foreground">Loading recipe catalog…</p>
  {:else}
    <Tabs.Root bind:value={sub}>
      <Tabs.List class={TRADESKILL_TAB_LIST_CLASS}>
        <Tabs.Trigger value="overview" class={TAB_TRIGGER_CLASS}>Overview</Tabs.Trigger>
        {#each $tradeskillCatalog as s (s.skill)}
          <Tabs.Trigger value={s.skill} class={TAB_TRIGGER_CLASS}>{s.skill}</Tabs.Trigger>
        {/each}
      </Tabs.List>

      <Tabs.Content value="overview">
        <div class="flex flex-col gap-3 pt-3">
          <div class="grid grid-cols-2 gap-3">
            <Card class="rounded-sm">
              <CardContent class="px-3 py-2.5">
                <h2 class="stat-figure mb-1.5 text-[18px]">The 9 tradeskills</h2>
                <ul class="flex flex-col gap-0.5 text-[11px]">
                  {#each $tradeskillCatalog as s (s.skill)}
                    <li class="flex justify-between">
                      <button type="button" class="text-brand-soft hover:text-primary hover:underline" onclick={() => jumpTo(s.skill)}>
                        {s.skill}
                      </button>
                      <span class="text-muted-foreground">{s.recipes.length} recipes</span>
                    </li>
                  {/each}
                </ul>
                <p class="mt-2 text-[10px] text-muted-foreground">
                  {totalRecipes.toLocaleString()} recipes total, scraped from eqlwiki.com. Some specialized armor-material
                  sub-tables aren't captured yet -- a real, stated gap, not silently wrong data.
                </p>
              </CardContent>
            </Card>

            <Card class="rounded-sm">
              <CardContent class="px-3 py-2.5">
                <h2 class="stat-figure mb-1.5 text-[18px]">Your craft log</h2>
                {#if !$craftLog.length}
                  <p class="text-[11px] text-muted-foreground">No combines parsed yet this file.</p>
                {:else}
                  <p class="text-[11px] text-muted-foreground">
                    {totalAttempts.toLocaleString()} attempt{totalAttempts === 1 ? '' : 's'} across {$craftLog.length} distinct
                    item{$craftLog.length === 1 ? '' : 's'}
                    {#if cappedCount}· {cappedCount} at skill cap{/if}
                  </p>
                {/if}
              </CardContent>
            </Card>
          </div>

          {#if $craftLog.length}
            <Card class="rounded-sm">
              <CardContent class="px-3 py-2.5">
                <table class="w-full text-[11px]">
                  <thead>
                    <tr class="text-left text-muted-foreground">
                      <th class="pb-1 font-normal">item</th>
                      <th class="pb-1 font-normal">tradeskill</th>
                      <th class="pb-1 font-normal">trivial</th>
                      <th class="pb-1 font-normal text-right">attempts</th>
                      <th class="pb-1 font-normal text-right">successes</th>
                      <th class="pb-1 font-normal"></th>
                    </tr>
                  </thead>
                  <tbody>
                    {#each $craftLog as e (e.item)}
                      <tr class="border-t border-border/50">
                        <td class="py-0.5"><GdLink kind="item" name={e.item} /></td>
                        <td class="py-0.5 text-muted-foreground">
                          {#if e.tradeskill}
                            <button type="button" class="text-brand-soft hover:text-primary hover:underline" onclick={() => jumpTo(e.tradeskill!)}>
                              {e.tradeskill}
                            </button>
                          {:else}
                            —
                          {/if}
                        </td>
                        <td class="py-0.5 text-muted-foreground">{e.trivial ?? '—'}</td>
                        <td class="py-0.5 text-right tabular-nums">{e.attempts}</td>
                        <td class="py-0.5 text-right tabular-nums">{e.successes}</td>
                        <td class="py-0.5 text-caution">{e.skill_capped ? 'capped' : ''}</td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </CardContent>
            </Card>
          {/if}
        </div>
      </Tabs.Content>

      {#each $tradeskillCatalog as s (s.skill)}
        <Tabs.Content value={s.skill}>
          <div class="pt-3">
            <RecipeList recipes={s.recipes} {craftLogByItem} onJumpToSkill={jumpTo} />
          </div>
        </Tabs.Content>
      {/each}
    </Tabs.Root>
  {/if}
</div>

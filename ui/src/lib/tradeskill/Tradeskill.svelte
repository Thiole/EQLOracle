<script lang="ts">
  // why: one tab per core tradeskill (confirmed real via Category:Tradeskills
  // -- the 11 race-specific "Cultural Tradeskill" pages are a different,
  // one-off epic-quest-adjacent thing, not covered here) plus a central
  // "Overview" tab: your parsed skill levels (incl. recipe-less
  // secondaries like Fishing) and the last 15 successful combines,
  // both linking straight into the matching skill's own tab.
  import * as Tabs from '$lib/components/ui/tabs';
  import { Card, CardContent } from '$lib/components/ui/card';
  import GdLink from '$lib/gamedata/GdLink.svelte';
  import RecipeList from './RecipeList.svelte';
  import {
    tradeskillCatalog,
    craftLog,
    tradeskillLevels,
    recentCrafts,
    loadTradeskillModule,
  } from '$lib/stores/tradeskill';
  import { ICON_BASE } from '$lib/character/constants';
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
  const levelBySkill = $derived(new Map($tradeskillLevels.map((l) => [l.skill, l])));
  const secondaries = $derived($tradeskillLevels.filter((l) => l.secondary));

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
        <div class="grid grid-cols-2 gap-3 pt-3">
          <Card class="rounded-sm">
            <CardContent class="px-3 py-2.5">
              <h2 class="stat-figure mb-1.5 text-[18px]">Your skills</h2>
              <table class="w-full text-[11px]">
                <thead>
                  <tr class="text-left text-muted-foreground">
                    <th class="pb-1 font-normal">skill</th>
                    <th class="pb-1 font-normal text-right">level</th>
                    <th class="pb-1 font-normal text-right">recipes</th>
                  </tr>
                </thead>
                <tbody>
                  {#each $tradeskillCatalog as s (s.skill)}
                    {@const lvl = levelBySkill.get(s.skill)}
                    <tr class="border-t border-border/50">
                      <td class="py-0.5">
                        <button type="button" class="text-brand-soft hover:text-primary hover:underline" onclick={() => jumpTo(s.skill)}>
                          {s.skill}
                        </button>
                      </td>
                      <td
                        class="py-0.5 text-right tabular-nums {lvl?.level != null ? '' : 'text-muted-foreground'}"
                        title={lvl?.at_ms != null ? `last skill-up ${new Date(lvl.at_ms).toLocaleString()}` : undefined}
                      >
                        {lvl?.level ?? '—'}
                      </td>
                      <td class="py-0.5 text-right tabular-nums text-muted-foreground">{s.recipes.length}</td>
                    </tr>
                  {/each}
                  {#each secondaries as l (l.skill)}
                    <tr class="border-t border-border/50">
                      <td class="py-0.5 text-foreground/80">{l.skill}</td>
                      <td
                        class="py-0.5 text-right tabular-nums {l.level != null ? '' : 'text-muted-foreground'}"
                        title={l.at_ms != null ? `last skill-up ${new Date(l.at_ms).toLocaleString()}` : undefined}
                      >
                        {l.level ?? '—'}
                      </td>
                      <td class="py-0.5 text-right text-muted-foreground">—</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
              <p class="mt-2 text-[10px] text-muted-foreground">
                Levels come from "You have become better at…" lines in this log file -- "—" means no skill-up seen yet,
                not level 0. {totalRecipes.toLocaleString()} recipes total, scraped from eqlwiki.com; some specialized
                armor-material sub-tables aren't captured yet.
              </p>
            </CardContent>
          </Card>

          <Card class="rounded-sm">
            <CardContent class="px-3 py-2.5">
              <h2 class="stat-figure mb-1.5 text-[18px]">Recently crafted</h2>
              {#if !$recentCrafts.length}
                <p class="text-[11px] text-muted-foreground">No successful combines parsed yet this file.</p>
              {:else}
                <ul class="flex flex-col">
                  {#each $recentCrafts as c, i (`${c.ts_ms}-${c.item}-${i}`)}
                    <li class="flex items-center gap-1.5 border-t border-border/50 py-0.5 text-[11px] first:border-t-0">
                      {#if c.icon}
                        <img src={ICON_BASE + encodeURIComponent(c.icon)} alt="" class="size-4 shrink-0 rounded-[2px]" />
                      {:else}
                        <span class="size-4 shrink-0 rounded-[2px] bg-muted/40"></span>
                      {/if}
                      <span class="min-w-0 flex-1 truncate"><GdLink kind="item" name={c.item} /></span>
                      {#if c.tradeskill}
                        <button
                          type="button"
                          class="shrink-0 text-brand-soft hover:text-primary hover:underline"
                          onclick={() => jumpTo(c.tradeskill!)}
                        >
                          {c.tradeskill}
                        </button>
                      {/if}
                      <span class="shrink-0 text-muted-foreground">{new Date(c.ts_ms).toLocaleString()}</span>
                    </li>
                  {/each}
                </ul>
                {#if $craftLog.length}
                  <p class="mt-2 text-[10px] text-muted-foreground">
                    {totalAttempts.toLocaleString()} attempt{totalAttempts === 1 ? '' : 's'} across {$craftLog.length} distinct
                    item{$craftLog.length === 1 ? '' : 's'} this file{#if cappedCount}
                      · {cappedCount} at skill cap{/if} -- per-item stats join each recipe in its skill's tab.
                  </p>
                {/if}
              {/if}
            </CardContent>
          </Card>
        </div>
      </Tabs.Content>

      {#each $tradeskillCatalog as s (s.skill)}
        <Tabs.Content value={s.skill}>
          <div class="pt-3">
            <RecipeList
              recipes={s.recipes}
              {craftLogByItem}
              level={levelBySkill.get(s.skill)?.level ?? null}
              onJumpToSkill={jumpTo}
            />
          </div>
        </Tabs.Content>
      {/each}
    </Tabs.Root>
  {/if}
</div>

<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Badge } from '$lib/components/ui/badge';
  import * as Select from '$lib/components/ui/select';
  import { api, type SkyClassDto, type TurnInDto, type TurnInItemDto } from '$lib/tauri/api';

  let classes = $state<SkyClassDto[] | null>(null);
  let error = $state<string | null>(null);

  async function load() {
    error = null;
    try {
      classes = await api.getSkyQuests();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  $effect(() => {
    void load();
  });

  // why: sold-without-keeping beats "owned" beats "looted" beats "never
  // looted" -- an item can only ever be in one of these states at a
  // time from the tracker's own point of view (see skyquests.rs's own
  // doc on why sold is notated separately from ever_looted).
  function itemStatus(it: TurnInItemDto): { label: string; classes: string; inHand: boolean } {
    if (it.sold_without_keeping) return { label: 'sold, not usable', classes: 'border-bad/40 bg-bad/10 text-bad', inHand: false };
    if (it.currently_owned != null && it.currently_owned > 0) return { label: `have ×${it.currently_owned}`, classes: 'border-good/40 bg-good/10 text-good', inHand: true };
    if (it.ever_looted) return { label: 'looted, not on hand', classes: 'border-caution/40 bg-caution/10 text-caution', inHand: false };
    return { label: 'not looted yet', classes: 'border-border text-muted-foreground', inHand: false };
  }

  // why: a plain fact -- how many of *this quest's own* rune + items are
  // still missing, regardless of whether the quest itself is already
  // achievement-confirmed done. Kept separate from sort priority below
  // on purpose: "done" is just a counter of what's already been turned
  // in, not itself a signal about what to do next.
  function questMissingItems(q: TurnInDto): number {
    const all = [q.rune, ...q.items].filter((x): x is TurnInItemDto => x != null);
    return all.filter((it) => !itemStatus(it).inHand).length;
  }

  // why: "closest to turn in" means *actionable* -- a quest that's
  // already done needs nothing from the player, so it has no business
  // sorting above one that's one item away but still open. Done quests
  // sort last (tied, alphabetical among themselves); everything still
  // open sorts by fewest items missing.
  function questSortKey(q: TurnInDto): number {
    return q.completed === true ? Number.POSITIVE_INFINITY : questMissingItems(q);
  }

  // why: a class's own aggregate -- total items still missing across
  // every one of its quests (a done quest naturally contributes 0, same
  // as an in-hand one would), so a class close to a full clear ranks
  // above one that's barely started. This is a real count, unlike
  // `questSortKey`'s own `Infinity` sentinel, which only exists to
  // control display order.
  function classStillNeeded(c: SkyClassDto): number {
    return c.quests.reduce((sum, q) => sum + questMissingItems(q), 0);
  }

  // why: default is "closest to turn in", A-Z demoted to the alternate
  // option and to its own tie-break within it -- same as the Unlocks
  // tab, asked directly to match.
  type SortMode = 'alpha' | 'closest';
  let sortBy = $state<SortMode>('closest');
  const SORT_LABELS: Record<SortMode, string> = { alpha: 'A-Z', closest: 'Closest to turn in' };

  const sortedClasses = $derived.by((): SkyClassDto[] | null => {
    if (!classes) return null;
    // why: quests within each class card always sort nearest-first (done
    // ones demoted to the bottom, out of the way), regardless of the
    // class-level mode -- an actionable, nearly-done quest buried under
    // a pile of already-turned-in ones would defeat the point.
    const withSortedQuests = classes.map((c) => ({
      ...c,
      quests: [...c.quests].sort((a, b) => questSortKey(a) - questSortKey(b) || a.quest.localeCompare(b.quest)),
    }));
    if (sortBy === 'alpha') return withSortedQuests;
    return withSortedQuests.sort((a, b) => classStillNeeded(a) - classStillNeeded(b) || a.class.localeCompare(b.class));
  });
</script>

{#snippet itemChip(it: TurnInItemDto)}
  {@const status = itemStatus(it)}
  <span class="inline-flex items-center gap-1 rounded-sm border px-1.5 py-0.5 text-[10px] {status.classes}" title="{it.item}{it.source ? ` (${it.source})` : ''} -- {status.label}">
    {it.item}
    <span class="opacity-80">· {status.label}</span>
  </span>
{/snippet}

{#if error}
  <div class="flex items-center gap-2 text-[12px]">
    <p class="text-destructive">{error}</p>
    <button type="button" class="text-primary underline" onclick={load}>retry</button>
  </div>
{:else if !classes}
  <p class="text-[12px] text-muted-foreground">Loading…</p>
{:else}
  <div class="flex flex-col gap-3">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <p class="text-[11px] text-muted-foreground">
        From <a class="text-brand-soft hover:text-primary hover:underline" href="https://eqlwiki.com/Plane_of_Sky#Plane_of_Sky_Class_Quests" target="_blank" rel="noopener"
          >Plane of Sky's own class quests</a
        > -- each quest turns in a Wind Rune plus 1-2 drop items for one gear reward. Item status comes from your loot history and
        latest <code class="rounded bg-muted px-1 py-0.5">/outputfile inventory</code> dump. For whether a class is actually
        unlocked, see the <b class="text-foreground">Sky - Primary Class Unlocks</b> tab -- that's tracked separately, off your own
        <code class="rounded bg-muted px-1 py-0.5">Achievements.txt</code>.
      </p>
      <label class="flex shrink-0 items-center gap-1.5 text-[11px]">
        <span class="text-muted-foreground">sort</span>
        <Select.Root type="single" value={sortBy} onValueChange={(v) => v && (sortBy = v as SortMode)}>
          <Select.Trigger class="h-7 w-40 text-[12px]">{SORT_LABELS[sortBy]}</Select.Trigger>
          <Select.Content>
            <Select.Item value="alpha">A-Z</Select.Item>
            <Select.Item value="closest">Closest to turn in</Select.Item>
          </Select.Content>
        </Select.Root>
      </label>
    </div>
    <div class="flex flex-wrap gap-3">
      {#each sortedClasses ?? [] as c (c.class)}
        <Card class="min-w-80 flex-1 rounded-sm">
          <CardContent class="px-3 py-2.5">
            <div class="mb-1.5 flex items-baseline gap-2">
              <h2 class="panel-title">{c.class}</h2>
              {#if c.quest_giver}<span class="text-[11px] text-muted-foreground">{c.quest_giver}</span>{/if}
            </div>

            <div class="flex flex-col divide-y divide-border">
              {#each c.quests as q (q.quest)}
                <div class="flex flex-col gap-1 py-1.5 first:pt-0 last:pb-0">
                  <div class="flex items-baseline justify-between gap-2">
                    <span class="text-[12px] font-medium text-foreground">{q.quest}</span>
                    {#if q.completed === true}
                      <span class="text-[10px] text-good">done · {q.reward}</span>
                    {:else if q.completed === false}
                      <span class="text-[10px] text-muted-foreground">open · {q.reward}</span>
                    {:else}
                      <span class="text-[10px] text-muted-foreground">? · {q.reward}</span>
                    {/if}
                  </div>
                  <div class="flex flex-wrap gap-1">
                    {#if q.rune}{@render itemChip(q.rune)}{/if}
                    {#each q.items as it (it.item)}
                      {@render itemChip(it)}
                    {/each}
                  </div>
                </div>
              {/each}
            </div>
          </CardContent>
        </Card>
      {/each}
    </div>
  </div>
{/if}

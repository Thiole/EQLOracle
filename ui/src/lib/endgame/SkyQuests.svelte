<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Badge } from '$lib/components/ui/badge';
  import * as Select from '$lib/components/ui/select';
  import BellIcon from '@lucide/svelte/icons/bell';
  import GdLink from '$lib/gamedata/GdLink.svelte';
  import { api, type SkyClassDto, type TurnInDto, type TurnInItemDto } from '$lib/tauri/api';
  import { trackedDropItems, toggleTrackedDropItem } from '$lib/stores/settings';

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

  // why: player correction -- Primary Class Unlocks bundles by class
  // because a class only unlocks once its whole quest set is done, a
  // real grouping. Plain Sky Quests has no such grouping: each one is
  // its own independent turn-in, and bundling them by class buried
  // "what's ready right now" inside 16 separate cards you had to open
  // one at a time. Flattened to one quest per card instead, class kept
  // only as a small label on the card, not a container.
  type FlatQuest = TurnInDto & { class: string; questGiver: string | null };

  const flatQuests = $derived.by((): FlatQuest[] | null => {
    if (!classes) return null;
    return classes.flatMap((c) => c.quests.map((q) => ({ ...q, class: c.class, questGiver: c.quest_giver })));
  });

  // why: default is "closest to turn in", A-Z demoted to the alternate
  // option -- same as the Unlocks tab, asked directly to match. Quest
  // names are themselves "<Class> Test of <X>", so sorting by name
  // alone already reads class-grouped in A-Z mode, no separate class key needed.
  type SortMode = 'alpha' | 'closest';
  let sortBy = $state<SortMode>('closest');
  const SORT_LABELS: Record<SortMode, string> = { alpha: 'A-Z', closest: 'Closest to turn in' };

  const sortedQuests = $derived.by((): FlatQuest[] | null => {
    if (!flatQuests) return null;
    const sorted = [...flatQuests];
    if (sortBy === 'alpha') {
      sorted.sort((a, b) => a.quest.localeCompare(b.quest));
    } else {
      sorted.sort((a, b) => questSortKey(a) - questSortKey(b) || a.quest.localeCompare(b.quest));
    }
    return sorted;
  });
</script>

{#snippet itemChip(it: TurnInItemDto)}
  {@const status = itemStatus(it)}
  {@const tracked = $trackedDropItems.includes(it.item)}
  {@const trackable = !it.item.startsWith('Wind Rune ')}
  <!-- why: Drop Watch's own entry point -- see dropwatch.rs's doc. Runes
       aren't mob drops (no wiki drop data for any of them, checked) --
       no bell on those, nothing Drop Watch could ever match anyway.
       Leading badge overlapping the chip's own border (not one more
       inline element crammed after the status text) -- its own visual
       slot, not competing with the chip's text for room. A bell
       (get-notified), not the target icon AllyTable's ability tracker
       uses -- same glyph on an unrelated feature reads as "no idea what
       this does". Always visible, not hover-reveal -- nobody knows this
       feature exists yet. Solid fill in the theme's own yes/no colors
       (good/bad, not literal red/green -- some themes don't use those
       hues for it) -- on/off must be unmistakable at this size, a
       color/outline swap this subtle wasn't. -->
  <span class="relative inline-flex">
    {#if trackable}
      <button
        type="button"
        class="absolute -top-2 -left-2 z-10 flex size-4 items-center justify-center rounded-full border {tracked
          ? 'border-good bg-good text-background'
          : 'border-bad bg-bad text-background'}"
        title={tracked ? `Stop tracking ${it.item} in the Drop Watch overlay` : `Track ${it.item} in the Drop Watch overlay`}
        onclick={() => void toggleTrackedDropItem(it.item)}
      >
        <BellIcon class="size-3" />
      </button>
    {/if}
    <span class="inline-flex items-center gap-1 rounded-sm border px-1.5 py-0.5 text-[10px] {status.classes}" title="{it.item}{it.source ? ` (${it.source})` : ''} -- {status.label}">
      <GdLink kind="item" name={it.item} />
      <span class="opacity-80">· {status.label}</span>
    </span>
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
    <div class="flex flex-wrap gap-2">
      {#each sortedQuests ?? [] as q (q.class + '::' + q.quest)}
        <Card class="min-w-72 flex-1 rounded-sm">
          <CardContent class="px-3 py-2 pb-2.5">
            <div class="mb-0.5 flex items-baseline justify-between gap-2">
              <span class="text-[12px] font-medium text-foreground">{q.quest}</span>
              {#if q.completed === true}
                <span class="shrink-0 text-[10px] text-good">done · {#if q.reward}<GdLink kind="item" name={q.reward} />{/if}</span>
              {:else if q.completed === false}
                <span class="shrink-0 text-[10px] text-muted-foreground">open · {#if q.reward}<GdLink kind="item" name={q.reward} />{/if}</span>
              {:else}
                <span class="shrink-0 text-[10px] text-muted-foreground">? · {#if q.reward}<GdLink kind="item" name={q.reward} />{/if}</span>
              {/if}
            </div>
            <div class="mb-1.5 text-[10px] text-muted-foreground">
              {q.class}{#if q.questGiver} · {q.questGiver}{/if}
            </div>
            <div class="flex flex-wrap gap-1.5 pt-2 pl-2">
              {#if q.rune}{@render itemChip(q.rune)}{/if}
              {#each q.items as it (it.item)}
                {@render itemChip(it)}
              {/each}
            </div>
          </CardContent>
        </Card>
      {/each}
    </div>
  </div>
{/if}

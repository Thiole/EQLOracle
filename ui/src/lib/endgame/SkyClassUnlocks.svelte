<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Badge } from '$lib/components/ui/badge';
  import * as Select from '$lib/components/ui/select';
  import TargetIcon from '@lucide/svelte/icons/target';
  import { api, type SkyClassUnlockDto, type SkyRewardDto } from '$lib/tauri/api';
  import { trackedDropItems, toggleTrackedDropItem } from '$lib/stores/settings';

  let classes = $state<SkyClassUnlockDto[] | null>(null);
  let error = $state<string | null>(null);

  async function load() {
    error = null;
    try {
      classes = await api.getSkyClassUnlocks();
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
  function itemStatus(it: SkyRewardDto): { label: string; classes: string; inHand: boolean } {
    if (it.sold_without_keeping) return { label: 'sold, not usable', classes: 'border-bad/40 bg-bad/10 text-bad', inHand: false };
    if (it.currently_owned != null && it.currently_owned > 0) return { label: `have ×${it.currently_owned}`, classes: 'border-good/40 bg-good/10 text-good', inHand: true };
    if (it.ever_looted) return { label: 'looted, not on hand', classes: 'border-caution/40 bg-caution/10 text-caution', inHand: false };
    return { label: 'not looted yet', classes: 'border-border text-muted-foreground', inHand: false };
  }

  // why: same principle as the Quests tab's own sort, asked directly
  // twice now to make sure it landed everywhere: "done" is just a
  // counter of what's already secured, not a priority signal -- an
  // already-confirmed reward has nothing left to do, so it sorts last,
  // out of the way, rather than crowding the top ahead of rewards that
  // actually need attention. Among the rest, ranked by real proximity:
  // already in hand (just needs confirming) closest, then looted-but-
  // not-on-hand (go find it), then never looted (the underlying quest
  // isn't done yet), then sold-without-keeping (start over) furthest.
  function rewardSortKey(r: SkyRewardDto): number {
    if (r.completed === true) return Number.POSITIVE_INFINITY;
    if (itemStatus(r).inHand) return 0;
    if (r.sold_without_keeping) return 3;
    if (r.ever_looted) return 1;
    return 2;
  }

  // why: asked directly a second time -- this is about closeness to a
  // turn-in, not closeness to 100%. A fully-unlocked class has nothing
  // left to do, so it sorts last (same "done isn't a priority signal"
  // rule as an individual reward), not first just because its own total
  // is technically the lowest. A class still in progress ranks by
  // whatever its single *closest* actionable reward is -- a class that's
  // otherwise untouched but has one reward already sitting in hand is
  // more relevant right now than one that's 5/6 done with the last
  // reward still never looted.
  function classSortKey(c: SkyClassUnlockDto): number {
    if (c.rewards.length > 0 && c.rewards.every((r) => r.completed === true)) return Number.POSITIVE_INFINITY;
    return Math.min(...c.rewards.map(rewardSortKey));
  }

  // why: default is "closest to turn in", A-Z demoted to the alternate
  // option and to its own tie-break within it -- asked directly.
  type SortMode = 'alpha' | 'closest';
  let sortBy = $state<SortMode>('closest');
  const SORT_LABELS: Record<SortMode, string> = { alpha: 'A-Z', closest: 'Closest to turn in' };

  const sortedClasses = $derived.by((): SkyClassUnlockDto[] | null => {
    if (!classes) return null;
    // why: rewards within each class card always sort nearest-first
    // (done ones demoted to the bottom), regardless of the class-level
    // mode -- same reason the Quests tab's own quests do.
    const withSortedRewards = classes.map((c) => ({
      ...c,
      rewards: [...c.rewards].sort((a, b) => rewardSortKey(a) - rewardSortKey(b) || a.name.localeCompare(b.name)),
    }));
    if (sortBy === 'alpha') return withSortedRewards;
    // why: ties broken alphabetically, same reason Raiding's own sort does.
    return withSortedRewards.sort((a, b) => classSortKey(a) - classSortKey(b) || a.class.localeCompare(b.class));
  });
</script>

{#snippet rewardChip(r: SkyRewardDto)}
  {@const status = itemStatus(r)}
  {@const secured = r.completed === true || status.inHand}
  <div class="flex flex-col gap-0.5 rounded-sm border px-2 py-1 {status.classes}">
    <div class="flex items-center justify-between gap-2">
      <span class="text-[11px] font-medium">{r.name}</span>
      {#if r.completed === true}
        <Badge class="h-4 border-good/40 bg-good/10 px-1 text-[9px] text-good" variant="outline">done</Badge>
      {:else if r.completed === false}
        <Badge class="h-4 px-1 text-[9px] text-muted-foreground" variant="outline">open</Badge>
      {:else}
        <Badge class="h-4 px-1 text-[9px] text-muted-foreground" variant="outline" title="no Achievements.txt line found for this reward">?</Badge>
      {/if}
    </div>
    <span class="text-[10px] opacity-80">{status.label}</span>
    {#if !secured}
      <!-- why: asked directly -- a reward that isn't sitting in hand yet
           has to say where it actually comes from, not just a bare
           quest name: which quest, which materials, which mob/island
           each one drops from. Each material is its own Drop Watch
           track button (see dropwatch.rs's doc) -- the runes here
           mostly aren't mob drops at all, but the drop items are. -->
      <p class="flex flex-wrap items-center gap-x-1 text-[10px] opacity-80">
        from <span class="font-medium">{r.quest}</span>:
        {#each r.materials as m, i (m.item)}
          {@const tracked = $trackedDropItems.includes(m.item)}
          <span class="group inline-flex items-center gap-0.5">
            {i > 0 ? ',' : ''}
            {m.item}{#if m.source}<span class="opacity-70"> ({m.source})</span>{/if}
            <button
              type="button"
              class="rounded-sm {tracked ? '' : 'opacity-0 group-hover:opacity-100'}"
              title={tracked ? `Stop tracking ${m.item} in the Drop Watch overlay` : `Track ${m.item} in the Drop Watch overlay`}
              onclick={() => void toggleTrackedDropItem(m.item)}
            >
              <TargetIcon class="size-3" />
            </button>
          </span>
        {/each}
      </p>
    {/if}
  </div>
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
        The final gear pieces each class's Sky quests earn -- a class unlocks once every one is achievement-confirmed. The
        quests/materials that build them live on the <b class="text-foreground">Sky - Quests</b> tab. Unlock/completion status comes
        from your own <code class="rounded bg-muted px-1 py-0.5">Achievements.txt</code>; item status from your loot history and
        latest <code class="rounded bg-muted px-1 py-0.5">/outputfile inventory</code> dump.
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
            <div class="mb-1.5 flex items-center justify-between gap-2">
              <div class="flex items-baseline gap-2">
                <h2 class="panel-title">{c.class}</h2>
                {#if c.quest_giver}<span class="text-[11px] text-muted-foreground">{c.quest_giver}</span>{/if}
              </div>
              {#if c.unlocked === true}
                <Badge class="h-5 border-good/40 bg-good/10 text-[10px] text-good" variant="outline">unlocked</Badge>
              {:else if c.unlocked === false}
                <Badge class="h-5 text-[10px] text-muted-foreground" variant="outline">locked</Badge>
              {:else}
                <Badge class="h-5 text-[10px] text-muted-foreground" variant="outline" title="no Achievements.txt found yet">?</Badge>
              {/if}
            </div>
            <div class="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
              {#each c.rewards as r (r.name)}
                {@render rewardChip(r)}
              {/each}
            </div>
          </CardContent>
        </Card>
      {/each}
    </div>
  </div>
{/if}

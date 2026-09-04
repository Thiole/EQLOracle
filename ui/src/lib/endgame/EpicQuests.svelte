<script lang="ts">
  // why: item-FIRST, asked directly -- the Epic Quests Era isn't open,
  // so this tab is a pre-farm tool: every farmable material per class
  // epic with live ownership status, the same chip-bell Drop Watch
  // entry points the Sky tabs use, a bulk bell for everything still
  // needed, and a concise per-class "+ all <Class>" bell on each card.
  // No completion state exists yet (no achievement line until the era
  // ships) -- "done" here is only "owned enough copies".
  import { Card, CardContent } from '$lib/components/ui/card';
  import BellIcon from '@lucide/svelte/icons/bell';
  import GdLink from '$lib/gamedata/GdLink.svelte';
  import ItemLocateLabel from '$lib/gamedata/ItemLocateLabel.svelte';
  import { api, type EpicClassDto, type EpicItemDto, type LineCounts, type TailStatus } from '$lib/tauri/api';
  import { listen } from '$lib/tauri/invoke';
  import { trackedDropItems, toggleTrackedDropItem, trackDropItems } from '$lib/stores/settings';

  let classes = $state<EpicClassDto[] | null>(null);
  let error = $state<string | null>(null);

  async function load() {
    error = null;
    try {
      classes = await api.getEpicQuests();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  // why: same live-reload contract as SkyQuests -- refresh when the
  // tick's loot counter moves or a fresh inventory dump lands, never on
  // every tick (each call re-reads the dump file)
  let lastLootCount = $state(-1);
  $effect(() => {
    void load();
    const unTick = listen<{ status: TailStatus; counts: LineCounts }>('parse-tick', (e) => {
      const loot = e.payload.counts.by_kind['loot'] ?? 0;
      if (lastLootCount !== -1 && loot !== lastLootCount) void load();
      lastLootCount = loot;
    });
    const unDump = listen('inventory-dump', () => void load());
    return () => {
      void unTick.then((f) => f());
      void unDump.then((f) => f());
    };
  });

  // why: same status ladder as the Sky tabs, plus the qty demand -- an
  // item wanted x2 with 1 owned still reads "needed"
  function itemStatus(it: EpicItemDto): { label: string; classes: string; satisfied: boolean } {
    const owned = it.currently_owned ?? 0;
    if (it.sold_without_keeping && owned === 0)
      return { label: 'sold, not usable', classes: 'border-bad/40 bg-bad/10 text-bad', satisfied: false };
    if (owned >= it.qty)
      return { label: `have ×${owned}${it.qty > 1 ? ` of ${it.qty}` : ''}`, classes: 'border-good/40 bg-good/10 text-good', satisfied: true };
    if (owned > 0)
      return { label: `have ×${owned} of ${it.qty}`, classes: 'border-caution/40 bg-caution/10 text-caution', satisfied: false };
    if (it.ever_looted)
      return { label: 'looted, not on hand', classes: 'border-caution/40 bg-caution/10 text-caution', satisfied: false };
    return { label: 'not looted yet', classes: 'border-border text-muted-foreground', satisfied: false };
  }

  function farmHint(it: EpicItemDto): string {
    const how = it.gather ? it.gather : it.mobs.length ? `kill ${it.mobs.join(' / ')}` : 'see wiki';
    return `${how}${it.zone ? ` -- ${it.zone}` : ''}${it.optional ? ' (optional route)' : ''}`;
  }

  // why: Spencer -- some of these materials are out of era, and the item
  // page never says so; the mob that drops it does. The backend reads the
  // dropper's own page, so a material whose every dropper is past the
  // live era is unfarmable no matter what its item page claims.
  let hideOutOfEra = $state(false);
  function eraNote(it: EpicItemDto): string {
    if (it.in_era) return '';
    const who = it.out_of_era_mobs.length ? it.out_of_era_mobs.join(' / ') : 'its droppers';
    return ` -- out of era: ${who}${it.era ? ` (${it.era})` : ''}`;
  }
  const outOfEraCount = $derived.by(
    () => classes?.reduce((n, c) => n + c.items.filter((it) => !it.in_era).length, 0) ?? 0,
  );

  // why: still-needed and not-yet-tracked, for one class -- feeds both
  // the per-class "+ all" bell and (unioned) the page-wide bulk bell.
  // De-duplicated by Set; an item two classes share is one entry.
  // why: an out-of-era material is not "still needed" for a bell -- the
  // Drop Watch cannot fire on a mob that does not exist yet
  function untrackedNeededOf(c: EpicClassDto): string[] {
    return c.items
      .filter((it) => it.in_era && !itemStatus(it).satisfied && !$trackedDropItems.includes(it.item))
      .map((it) => it.item);
  }

  const untrackedNeededAll = $derived.by((): string[] => {
    if (!classes) return [];
    return [...new Set(classes.flatMap(untrackedNeededOf))];
  });
</script>

{#snippet itemChip(it: EpicItemDto)}
  {@const status = itemStatus(it)}
  {@const tracked = $trackedDropItems.includes(it.item)}
  <!-- why: identical bell affordance to the Sky tabs -- same feature,
       same glyph, same on/off colors; every epic material is a real
       drop (or forage/pickpocket target), so every chip gets one -->
  <span class="relative inline-flex">
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
    <span
      class="inline-flex items-center gap-1 rounded-sm border px-1.5 py-0.5 text-[10px] {status.classes} {it.in_era ? '' : 'opacity-55'}"
      title="{it.item}{it.qty > 1 ? ` ×${it.qty}` : ''} -- {farmHint(it)} -- {status.label}{eraNote(it)}"
    >
      <GdLink kind="item" name={it.item} bell={false} />
      {#if it.qty > 1}<span class="opacity-80">×{it.qty}</span>{/if}
      {#if it.gather}<span class="opacity-70 italic">{it.gather}</span>{/if}
      {#if it.optional}<span class="opacity-70">opt</span>{/if}
      {#if !it.in_era}<span class="rounded-sm bg-muted px-1 text-[9px] text-muted-foreground">{it.era ?? 'out of era'}</span>{/if}
      <span class="opacity-80">· <ItemLocateLabel item={it.item} label={status.label} owned={status.satisfied} /></span>
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
        Farmable materials per <a class="text-brand-soft hover:text-primary hover:underline" href="https://eqlwiki.com/Class_Epic_Quest_List" target="_blank" rel="noopener">class epic quest</a>
        -- the drops you can hunt <i>before</i> the Epic Quests Era opens. NPC-handed quest intermediates are excluded on purpose:
        they need the era's own quest NPCs. Status comes from your loot history and latest
        <code class="rounded bg-muted px-1 py-0.5">/outputfile inventory</code> dump; there's no completion state until the era ships.
      </p>
      <div class="flex shrink-0 items-center gap-2">
      {#if outOfEraCount}
        <label class="flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <input type="checkbox" class="size-3" bind:checked={hideOutOfEra} />
          hide {outOfEraCount} out of era
        </label>
      {/if}
      <button
        type="button"
        class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2 py-1 text-[11px] transition-colors {untrackedNeededAll.length
          ? 'text-foreground hover:border-good/60 hover:bg-good/10 hover:text-good'
          : 'cursor-not-allowed text-muted-foreground opacity-50'}"
        disabled={!untrackedNeededAll.length}
        title={untrackedNeededAll.length
          ? `Track ${untrackedNeededAll.length} still-needed epic material${untrackedNeededAll.length === 1 ? '' : 's'} in the Drop Watch overlay`
          : 'Every still-needed epic material is already tracked'}
        onclick={() => void trackDropItems(untrackedNeededAll)}
      >
        <BellIcon class="size-3.5" />
        notify for all epic items{untrackedNeededAll.length ? ` (${untrackedNeededAll.length})` : ''}
      </button>
      </div>
    </div>
    <div class="grid gap-2 [grid-template-columns:repeat(auto-fill,minmax(340px,1fr))]">
      {#each classes as c (c.class)}
        {@const needed = untrackedNeededOf(c)}
        <Card class="rounded-sm">
          <CardContent class="px-3 py-2 pb-2.5">
            <div class="mb-0.5 flex items-baseline justify-between gap-2">
              <span class="text-[12px] font-medium text-foreground">{c.class}</span>
              <!-- why: the per-class version of the bulk bell, asked
                   directly ("add all Wizard Epic Quest Items ... in a
                   more concise manner") -->
              <button
                type="button"
                class="flex shrink-0 items-center gap-1 rounded-md border border-border px-1.5 py-0.5 text-[10px] transition-colors {needed.length
                  ? 'text-foreground hover:border-good/60 hover:bg-good/10 hover:text-good'
                  : 'cursor-not-allowed text-muted-foreground opacity-50'}"
                disabled={!needed.length}
                title={needed.length
                  ? `Track all ${needed.length} still-needed ${c.class} epic material${needed.length === 1 ? '' : 's'} in the Drop Watch overlay`
                  : c.items.length
                    ? `Every still-needed ${c.class} epic material is already tracked`
                    : 'Nothing pre-farmable for this class'}
                onclick={() => void trackDropItems(needed)}
              >
                <BellIcon class="size-3" />
                + all {c.class}{needed.length ? ` (${needed.length})` : ''}
              </button>
            </div>
            <div class="mb-1.5 text-[10px] text-muted-foreground">
              {#if c.final_reward}<GdLink kind="item" name={c.final_reward} bell={false} />{/if}
              {#if c.quest_giver}&nbsp;· {c.quest_giver}{/if}
              {#if c.start_zone}&nbsp;· {c.start_zone}{/if}
              {#if c.recommended_level}&nbsp;· lvl {c.recommended_level}{/if}
            </div>
            {@const shown = hideOutOfEra ? c.items.filter((it) => it.in_era) : c.items}
            {#if shown.length}
              <div class="flex flex-wrap gap-1.5 pt-2 pl-2">
                {#each shown as it (it.item)}
                  {@render itemChip(it)}
                {/each}
              </div>
            {:else if c.items.length}
              <p class="text-[10px] text-muted-foreground italic">
                Every material for this epic drops from a mob that is out of era.
              </p>
            {:else}
              <!-- why: honest empty state -- Berserker's epic is trial
                   spawns end to end, nothing exists to pre-farm -->
              <p class="text-[10px] text-muted-foreground italic">
                Nothing pre-farmable -- this epic is quest-triggered spawns end to end.
              </p>
            {/if}
          </CardContent>
        </Card>
      {/each}
    </div>
  </div>
{/if}

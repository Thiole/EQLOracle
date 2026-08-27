<script lang="ts">
  // why: 4x6 paper-doll grid, icon-only squares, keyed reactive blocks
  import { untrack } from 'svelte';
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Input } from '$lib/components/ui/input';
  import {
    race,
    activeClasses,
    gearRecommendations,
    gearWeights,
    gearChosen,
    equippedInventory,
    inventoryDumpVersion,
    clearEquippedSlot,
    equippedGearItem,
    chosenGearItem,
    isTwoHandItem,
    refreshEstimate,
  } from '$lib/stores/character';
  import { effectiveEra } from '$lib/stores/settings';
  import { DOLL_ROWS, ICON_BASE, WEIGHT_GROUPS, SLOT_LABELS } from './constants';
  import { api, type ScoredItemDto, type ItemDto } from '$lib/tauri/api';
  import GdLinkList from '$lib/gamedata/GdLinkList.svelte';
  import GdZoneOrMobLink from '$lib/gamedata/GdZoneOrMobLink.svelte';

  const TOP_N = 6;

  /** why: which slot's alt list is open */
  let expandedSlot = $state<string | null>(null);
  /** why: sticks until the next click; survives the mouse leaving */
  let stickyItem = $state<ScoredItemDto | null>(null);
  /** why: transient, mouse-only -- takes priority over stickyItem while set */
  let hoveredItem = $state<ScoredItemDto | null>(null);
  /** why: what the preview card actually shows -- hover wins, sticky is the fallback */
  const previewItem = $derived(hoveredItem ?? stickyItem);
  function hoverItem(item: ScoredItemDto | null | undefined) {
    hoveredItem = item ?? null;
  }

  /** why: "what if I upgrade this" tier the preview panel's picker is set
   * to, per item id -- independent of the item's real tier, and never
   * written back to `chosen`/ownership; a what-if, not a commitment. */
  let plannedTier = $state<Record<string, number>>({});
  /** why: item_at_tier's own re-derived stats/exalts at the planned tier,
   * refetched whenever the plan differs from the previewed item's real
   * tier -- null (falls back to previewItem as-is) when they match. */
  let previewOverride = $state<ItemDto | null>(null);
  let overrideToken = 0;
  $effect(() => {
    const item = previewItem;
    const wantTier = item ? (plannedTier[item.id] ?? item.tier) : 0;
    if (!item || wantTier === item.tier) {
      previewOverride = null;
      return;
    }
    const token = ++overrideToken;
    void api.getItemAtTier(item.id, wantTier).then((dto) => {
      if (token !== overrideToken) return;
      previewOverride = dto;
    });
  });
  /** why: real ownership and real session proc evidence always come from
   * previewItem -- item_at_tier has no dump/session context and always
   * reports owned:0/proc_evidence:null, which must not stomp on "you
   * actually have one of these" or "this really fired" while just
   * previewing a tier bump. */
  const displayedPreview = $derived.by<ScoredItemDto | null>(() => {
    if (!previewItem) return null;
    let result: ScoredItemDto = previewItem;
    if (previewOverride && previewOverride.id === previewItem.id) {
      result = {
        ...result,
        ...previewOverride,
        owned: previewItem.owned,
        proc_evidence: previewItem.proc_evidence,
      };
    }
    if (exaltOverride && exaltOverride.id === previewItem.id) {
      result = { ...result, exalts: exaltOverride.exalts };
    }
    return result;
  });

  /** why: "what if I socket this other item's effect here" per item id ->
   * {socketKey: sourceItemId} -- a what-if the same way plannedTier is,
   * never touching real ownership or any actual game state (this app has
   * no way to know what's really socketed in-game -- see
   * ExaltationProcs' own doc). Kept keyed by item id, not cleared on
   * hover/click changes, so switching preview items and coming back
   * doesn't lose the plan. */
  let exaltAssignments = $state<Record<string, Record<string, string>>>({});
  /** why: which item+socket's candidate picker is open, if any */
  let exaltPickerOpen = $state<{ itemId: string; key: string } | null>(null);
  let exaltCandidates = $state<ItemDto[]>([]);
  let exaltSearch = $state('');
  let exaltCandidateToken = 0;

  function assignmentsFor(itemId: string): Record<string, string> {
    return exaltAssignments[itemId] ?? {};
  }

  async function openExaltPicker(itemId: string, key: string) {
    exaltPickerOpen = { itemId, key };
    exaltSearch = '';
    exaltCandidates = [];
    const token = ++exaltCandidateToken;
    const others = { ...assignmentsFor(itemId) };
    delete others[key];
    const list = await api.getExaltCandidates(itemId, key, others, $activeClasses, $effectiveEra);
    if (token !== exaltCandidateToken) return;
    exaltCandidates = list;
  }
  function closeExaltPicker() {
    exaltPickerOpen = null;
    exaltCandidateToken++; // orphan any in-flight fetch
  }
  function assignExalt(itemId: string, key: string, sourceId: string) {
    exaltAssignments = {
      ...exaltAssignments,
      [itemId]: { ...assignmentsFor(itemId), [key]: sourceId },
    };
    closeExaltPicker();
  }
  function clearExalt(itemId: string, key: string) {
    const next = { ...assignmentsFor(itemId) };
    delete next[key];
    exaltAssignments = { ...exaltAssignments, [itemId]: next };
  }
  const filteredExaltCandidates = $derived.by(() => {
    const q = exaltSearch.trim().toLowerCase();
    return q ? exaltCandidates.filter((c) => c.name.toLowerCase().includes(q)) : exaltCandidates;
  });

  /** why: item_with_exalts' own re-derived `exalts` array, refetched
   * whenever this item has at least one real assignment -- overlays only
   * the exalts field (see displayedPreview below), tier/stats/etc. are
   * untouched by exaltation. */
  let exaltOverride = $state<ItemDto | null>(null);
  let exaltOverrideToken = 0;
  $effect(() => {
    const item = previewItem;
    const assigns = item ? assignmentsFor(item.id) : {};
    if (!item || !Object.keys(assigns).length) {
      exaltOverride = null;
      return;
    }
    const tier = plannedTier[item.id] ?? item.tier;
    const token = ++exaltOverrideToken;
    void api.getItemWithExalts(item.id, tier, assigns).then((dto) => {
      if (token !== exaltOverrideToken) return;
      exaltOverride = dto;
    });
  });

  /** why: filters the open slot's alternatives; cleared on slot change */
  let altSearch = $state('');
  /** why: default on -- the overview is for tracking down what's still needed */
  let hideOwned = $state(true);
  /** why: temporary spell-conjured items hidden from recommendations by default */
  let showSummoned = $state(false);
  /** why: zone groups start collapsed; label -> expanded */
  let expandedZones = $state<Set<string>>(new Set());
  function toggleZone(label: string) {
    const next = new Set(expandedZones);
    next.has(label) ? next.delete(label) : next.add(label);
    expandedZones = next;
  }

  const bySlot = $derived(
    new Map(
      ($gearRecommendations ?? []).map((r) => [
        r.slot,
        showSummoned ? r.items : r.items.filter((it) => !it.name.startsWith('Summoned: ')),
      ]),
    ),
  );

  // why: a real fresh dump replaces picks wholesale -- keyed off the version
  // token, not $equippedInventory itself, since clearEquippedSlot (part of
  // pickAlt's own commit) also mutates that store in place; reacting to the
  // store directly wiped the very pick pickAlt was mid-committing.
  $effect(() => {
    $inventoryDumpVersion;
    if (untrack(() => $equippedInventory)) {
      gearChosen.set({});
      stickyItem = null;
      hoveredItem = null;
      expandedSlot = null;
    }
  });

  // why: local wrappers just so every call site below doesn't need a
  // `$gearChosen`/`$equippedInventory` read of its own -- chosenGearItem/
  // equippedGearItem live in the shared store (see its own doc) so
  // refreshEstimate's gear totals can never disagree with what this doll
  // shows as worn.
  function equippedItem(key: string): ScoredItemDto | undefined {
    $equippedInventory; // establish reactive dependency
    return equippedGearItem(key);
  }
  function chosenItem(key: string, items: ScoredItemDto[]): ScoredItemDto | undefined {
    $gearChosen;
    $equippedInventory;
    return chosenGearItem(key, items);
  }

  const primaryItem = $derived(chosenItem('PRIMARY', bySlot.get('PRIMARY') ?? []));
  // why: 2H primary occupies secondary too, real inventory rule
  const primaryIsTwoHand = $derived(isTwoHandItem(primaryItem));

  $effect(() => {
    if (primaryIsTwoHand && expandedSlot === 'SECONDARY') expandedSlot = null;
  });

  function toggleSlot(key: string, items: ScoredItemDto[]) {
    altSearch = '';
    if (expandedSlot === key) {
      // why: deselecting a slot must not keep acting "item focused"
      expandedSlot = null;
      stickyItem = null;
    } else {
      expandedSlot = key;
      stickyItem = chosenItem(key, items) ?? null;
    }
  }

  /** why: search flattens to one ranked-order list; otherwise top-N + a lighter rest */
  const altSplit = $derived.by(() => {
    const items = expandedSlot ? (bySlot.get(expandedSlot) ?? []) : [];
    const q = altSearch.trim().toLowerCase();
    if (q) return { top: items.filter((it) => it.name.toLowerCase().includes(q)), other: [] as ScoredItemDto[] };
    return { top: items.slice(0, TOP_N), other: items.slice(TOP_N) };
  });

  /** why: no slot selected -- summarize where the current loadout's gear comes from */
  const loadoutOverview = $derived.by(() => {
    type Entry = { key: string; slotLabel: string; owned: boolean; item: ScoredItemDto };
    const groups = new Map<string, Entry[]>();
    let ownedCount = 0;
    let total = 0;
    for (const [key, items] of bySlot) {
      if (key === 'SECONDARY' && primaryIsTwoHand) continue; // occupied by primary, not its own pick
      const item = chosenItem(key, items);
      if (!item) continue;
      total++;
      const owned = item.owned > 0 || !!equippedItem(key);
      if (owned) ownedCount++;
      const label = item.zones[0] || item.source || 'unknown source';
      const list = groups.get(label) ?? [];
      list.push({ key, slotLabel: SLOT_LABELS[key] ?? key, owned, item });
      groups.set(label, list);
    }
    // why: hide-owned filters per group; a group left with nothing drops out
    const visible = [...groups.entries()]
      .map(([label, entries]) => [label, hideOwned ? entries.filter((e) => !e.owned) : entries] as const)
      .filter(([, entries]) => entries.length > 0)
      .sort((a, b) => b[1].length - a[1].length);
    return { groups: visible, ownedCount, total };
  });
  function pickAlt(key: string, item: ScoredItemDto) {
    clearEquippedSlot(key);
    gearChosen.update((c) => ({ ...c, [key]: item.id }));
    stickyItem = item;
    expandedSlot = null;
    void refreshEstimate(); // what's "worn" in this slot just changed
  }

  function iconUrl(icon: string | null): string | null {
    return icon ? ICON_BASE + encodeURIComponent(icon) : null;
  }
  function statSummary(item: ScoredItemDto): string {
    const entries = Object.entries(item.stats);
    if (!entries.length) return '';
    entries.sort((a, b) => ($gearWeights[b[0]] ?? 0) * b[1] - ($gearWeights[a[0]] ?? 0) * a[1]);
    return entries
      .slice(0, 3)
      .map(([k, v]) => `${k}${v >= 0 ? '+' : ''}${v}`)
      .join(' ');
  }
</script>

{#snippet altRow(it: ScoredItemDto, top: boolean, light: boolean)}
  {@const icon = iconUrl(it.icon)}
  {@const active = stickyItem?.id === it.id}
  <button
    type="button"
    class="flex w-full items-start gap-2 rounded-sm text-left transition-colors hover:bg-muted/40 {light ? 'px-1.5 py-0.5' : 'px-1.5 py-1'} {active
      ? 'bg-muted/40 outline outline-primary'
      : ''}"
    onclick={() => pickAlt(expandedSlot!, it)}
    onmouseenter={() => hoverItem(it)}
    onmouseleave={() => hoverItem(null)}
  >
    {#if icon}
      <img src={icon} alt="" loading="lazy" class="{light ? 'size-5' : 'size-6'} shrink-0 rounded-sm border border-border bg-muted/20" />
    {:else}
      <div class="{light ? 'size-5' : 'size-6'} shrink-0 rounded-sm border border-border bg-muted/20"></div>
    {/if}
    <div class="min-w-0 flex-1">
      <div class="flex items-center gap-2">
        <span class="min-w-0 flex-1 truncate {light ? 'text-[11px] text-muted-foreground' : 'text-[12px]'}"
          >{top ? '★ ' : ''}{it.name}{#if it.tier > 0}<span class="text-good"> +{it.tier}</span>{/if}</span
        >
        {#if it.owned > 0}<span class="shrink-0 text-[10px] text-good">×{it.owned}</span>{/if}
        <span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">{it.score.toFixed(1)}</span>
      </div>
      {#if !light}
        {#if it.source}<div class="truncate text-[10px] text-brand-soft">{it.source}</div>{/if}
        <div class="truncate text-[10px] text-primary">{statSummary(it)}</div>
      {/if}
    </div>
  </button>
{/snippet}

<div class="flex flex-col gap-3">
  <p class="rounded-sm border border-border bg-muted/20 px-3 py-2 text-[11px] text-muted-foreground">
    Use the in-game command <code class="text-foreground">/outputfile inventory</code> to pull your currently equipped gear into the doll below.
    Race and active classes are shared with the Character tab.
  </p>

  {#if $equippedInventory}
    {@const resolvedCount = Object.keys($equippedInventory.resolved).length}
    {@const unresolvedCount = Object.keys($equippedInventory.unresolved).length}
    <p class="text-[11px] text-muted-foreground">
      Showing {resolvedCount} equipped item{resolvedCount === 1 ? '' : 's'} from your last dump{unresolvedCount
        ? ` — ${unresolvedCount} not matched to a known item`
        : ''}.
    </p>
  {/if}

  {#if !$activeClasses.length}
    <p class="text-[12px] text-muted-foreground">Mark up to 3 active classes on the Character tab to see recommendations here.</p>
  {:else if !$gearRecommendations}
    <p class="text-[12px] text-muted-foreground">Loading…</p>
  {:else}
    <div class="flex items-center justify-between gap-3">
      <p class="text-[11px] text-muted-foreground">
        Top picks for <b class="text-foreground">{$activeClasses.join(' / ')}</b>{#if $race}, <b class="text-foreground">{$race}</b>{/if} — click a
        slot to see alternatives.
      </p>
      <label class="flex shrink-0 items-center gap-1.5 text-[10px] text-muted-foreground select-none">
        <input type="checkbox" bind:checked={showSummoned} class="size-3 accent-primary" />
        show summon items
      </label>
    </div>

    <div class="grid grid-cols-[352px_1fr] items-start gap-4">
      <!-- why: fixed-width doll column, rest of the width goes to detail -->
      <div class="flex flex-col gap-1.5">
        {#each DOLL_ROWS as row, ri (ri)}
          <!-- why: a short row (a spacer in it) centers instead of a same-size blank tile -->
          <div class="flex gap-1.5 {row.includes(null) ? 'justify-center' : ''}">
            {#each row.filter((e): e is [string, string] => e !== null) as entry (entry[0])}
              {@const [key, label] = entry}
              {@const locked = key === 'SECONDARY' && primaryIsTwoHand}
                {@const items = bySlot.get(key) ?? []}
                {@const equipped = !locked && !!equippedItem(key)}
                {@const top = locked ? primaryItem : chosenItem(key, items)}
                {@const icon = iconUrl(top?.icon ?? null)}
                <button
                  type="button"
                  class="relative flex size-14 shrink-0 items-center justify-center rounded-sm border bg-muted/20 transition-colors {locked
                    ? 'cursor-not-allowed border-dashed border-border bg-background'
                    : expandedSlot === key
                      ? 'border-primary shadow-[0_0_0_1px_var(--color-primary)]'
                      : top
                        ? 'border-border hover:border-primary/60'
                        : 'border-dashed border-border hover:border-primary/60'}"
                  disabled={locked}
                  onclick={() => toggleSlot(key, items)}
                  onmouseenter={() => hoverItem(top)}
                  onmouseleave={() => hoverItem(null)}
                  title={locked
                    ? `Occupied — wielding ${primaryItem?.name ?? 'a two-handed weapon'} two-handed`
                    : top
                      ? `${top.name}${top.tier > 0 ? ` +${top.tier}` : ''}${equipped ? ' — currently equipped' : ''}${top.source ? ` — ${top.source}` : ''}`
                      : 'no eligible item found'}
                >
                  <span class="pointer-events-none absolute top-0.5 left-1 text-[7.5px] tracking-wide text-muted-foreground uppercase">{label}</span>
                  {#if icon}
                    <img src={icon} alt="" loading="lazy" class="size-[78%] rounded-sm object-cover {locked ? 'opacity-40 grayscale' : ''}" />
                  {/if}
                  {#if locked}
                    <span class="pointer-events-none absolute right-1 bottom-0.5 text-[7.5px] text-muted-foreground">2H</span>
                  {:else if top && top.tier > 0}
                    <span class="pointer-events-none absolute right-1 bottom-0.5 text-[7.5px] font-semibold text-good">+{top.tier}</span>
                  {/if}
                  {#if equipped}
                    <span class="pointer-events-none absolute top-0.5 right-0.5 size-[5px] rounded-full bg-good" title="currently equipped"></span>
                  {/if}
                </button>
            {/each}
          </div>
        {/each}

        <Card class="mt-1.5 rounded-sm">
          <CardContent class="px-3 py-2.5">
            <h2 class="panel-title mb-1.5">scoring weights</h2>
            <div class="flex flex-col gap-1">
              {#each WEIGHT_GROUPS as [group, entries] (group)}
                <div class="flex flex-wrap gap-x-3 gap-y-0.5 text-[11px]">
                  {#each entries as [key, label] (key)}
                    <span
                      ><span class="text-muted-foreground">{label}</span>
                      <span class="tabular-nums">{($gearWeights[key] ?? 0).toFixed(2)}</span></span
                    >
                  {/each}
                </div>
              {/each}
            </div>
          </CardContent>
        </Card>

        <!-- why: broken out from item preview -- always visible, not something a click replaces -->
        {#if loadoutOverview.total > 0}
          <Card class="rounded-sm">
            <CardContent class="px-3 py-2.5">
              <div class="mb-1.5 flex items-center justify-between gap-3">
                <h2 class="panel-title">zones</h2>
                <label class="flex items-center gap-1.5 text-[10px] text-muted-foreground select-none">
                  <input type="checkbox" bind:checked={hideOwned} class="size-3 accent-primary" />
                  hide owned
                </label>
              </div>
              <p class="mb-2 text-[11px] text-muted-foreground">
                Where this loadout's gear comes from —
                <b class="text-foreground tabular-nums">{loadoutOverview.ownedCount}</b> of
                <b class="text-foreground tabular-nums">{loadoutOverview.total}</b> pieces already owned{#if hideOwned}, hidden below{/if}.
              </p>
              {#if !loadoutOverview.groups.length}
                <p class="text-[11px] text-muted-foreground">Everything in this loadout is already owned.</p>
              {/if}
              <div class="flex flex-col gap-1">
                {#each loadoutOverview.groups as [label, entries] (label)}
                  {@const open = expandedZones.has(label)}
                  <div>
                    <button
                      type="button"
                      class="flex w-full items-center gap-1 rounded-sm py-0.5 text-left text-[11px] text-brand-soft hover:text-primary"
                      onclick={() => toggleZone(label)}
                    >
                      <span class="w-3 text-[9px] text-muted-foreground">{open ? '▾' : '▸'}</span>
                      <span class="truncate">{label}</span>
                      <span class="shrink-0 text-muted-foreground">({entries.length})</span>
                    </button>
                    {#if open}
                      <div class="mt-0.5 ml-4 flex flex-col gap-0.5">
                        {#each entries as e (e.key)}
                          <button
                            type="button"
                            class="flex items-center gap-1.5 rounded-sm text-left text-[11px] hover:bg-muted/40"
                            onclick={() => (stickyItem = e.item)}
                            onmouseenter={() => hoverItem(e.item)}
                            onmouseleave={() => hoverItem(null)}
                          >
                            <span
                              class="size-[5px] shrink-0 rounded-full {e.owned ? 'bg-good' : 'bg-muted-foreground/40'}"
                              title={e.owned ? 'owned' : 'not owned yet'}
                            ></span>
                            <span class="text-muted-foreground">{e.slotLabel}</span>
                            <span class="truncate">{e.item.name}</span>
                          </button>
                        {/each}
                      </div>
                    {/if}
                  </div>
                {/each}
              </div>
            </CardContent>
          </Card>
        {/if}
      </div>

      <!-- why: right rail -- preview always visible, alternatives beside it, not stacked below the doll -->
      <div class="flex min-w-0 flex-col gap-3">
        <Card class="rounded-sm">
          <CardContent class="px-3 py-2.5">
            <h2 class="panel-title mb-1.5">item preview</h2>
            <!-- why: real bug, caught live -- "jitters around when you mouse
                 over certain things ... it pushes it to another item". This
                 card's own height used to track whatever was hovered (an
                 empty placeholder line vs. a fully exalted item's icon/
                 stats/upgrade row/exalt sockets/zones), and the
                 alternatives list right below it (same column, plain
                 stacked flow) shifted up or down every time. Hovering a
                 row in THAT list moved the row out from under the cursor
                 mid-hover, which fired a new onmouseenter on whatever
                 landed there, which resized the preview again -- a
                 feedback loop, not a one-off flicker. Fixed height +
                 internal scroll: this card's own box never moves the
                 layout around it again, regardless of what's previewed. -->
            <div class="h-[26rem] overflow-y-auto pr-1">
            {#if !displayedPreview}
              <p class="text-[12px] text-muted-foreground">Hover or click a slot to preview an item here.</p>
            {:else}
              {@const item = displayedPreview}
              {@const icon = iconUrl(item.icon)}
              <div class="flex gap-3">
                {#if icon}
                  <img src={icon} alt="" class="size-14 shrink-0 rounded-sm border border-border bg-muted/20" />
                {/if}
                <div class="min-w-0 flex-1">
                  <div class="flex items-center gap-1.5">
                    <span class="stat-figure text-[15px]"
                      >{item.name}{#if item.tier > 0}<span class="text-good"> +{item.tier}</span>{/if}</span
                    >
                    {#if item.owned > 0}
                      <span class="rounded-full border border-good/40 bg-good/10 px-1.5 py-0.5 text-[10px] text-good">owned ×{item.owned}</span>
                    {/if}
                  </div>
                  <div class="mt-0.5 text-[11px] text-muted-foreground">
                    {[
                      item.classes.length ? item.classes.join(' / ') : 'any class',
                      item.slots.join(', '),
                      item.era,
                      item.wt != null ? `WT ${item.wt}` : null,
                      item.size,
                    ]
                      .filter(Boolean)
                      .join(' — ')}
                  </div>
                  {#if Object.keys(item.stats).length}
                    <div class="mt-1.5 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] tabular-nums text-primary">
                      {#each Object.entries(item.stats).sort((a, b) => b[1] - a[1]) as [k, v] (k)}
                        <span>{k} {v >= 0 ? '+' : ''}{v}</span>
                      {/each}
                    </div>
                  {/if}
                </div>
              </div>

              <!-- why: upgrade preview -- "level this to +N" without touching real ownership -->
              <div class="mt-2 flex flex-wrap items-center gap-1">
                <span class="mr-1 text-[10px] tracking-wide text-muted-foreground uppercase">upgrade</span>
                {#each Array.from({ length: 11 }, (_, n) => n) as n (n)}
                  {@const on = (plannedTier[item.id] ?? item.tier) === n}
                  <button
                    type="button"
                    class="rounded-sm border px-1.5 py-0.5 text-[10px] tabular-nums transition-colors {on
                      ? 'border-primary bg-primary/15 text-primary'
                      : 'border-border text-muted-foreground hover:border-primary/50 hover:text-foreground'}"
                    onclick={() => (plannedTier = { ...plannedTier, [item.id]: n })}
                  >
                    +{n}
                  </button>
                {/each}
              </div>

              {#if item.exalts.length}
                <div class="mt-2 flex flex-col gap-1 border-t border-border pt-1.5">
                  <span class="mb-0.5 text-[10px] tracking-wide text-muted-foreground uppercase">exaltations</span>
                  {#each item.exalts as ex (ex.key)}
                    {@const assignedId = assignmentsFor(item.id)[ex.key]}
                    <div class="flex flex-col gap-1">
                      <div class="flex items-center gap-1.5 text-[11px]">
                        <span class="w-24 shrink-0 text-muted-foreground">{ex.label}</span>
                        {#if ex.key === 'proc' && item.proc_evidence}
                          <!-- why: real log evidence trumps the catalog-derived
                               display -- proves this socket is genuinely live
                               without claiming to know which effect it is -->
                          <span class="truncate text-good" title="Never says which effect resulted -- only that it's real and fired.">
                            confirmed active
                            <span class="text-muted-foreground"
                              >— fired {item.proc_evidence.fires}× this session, since {new Date(
                                item.proc_evidence.first_seen_ms,
                              ).toLocaleString()}</span
                            >
                          </span>
                        {:else if !ex.unlocked}
                          <span class="text-muted-foreground/60">locked — needs +{ex.req_tier}</span>
                        {:else}
                          {#if ex.effect}
                            <span class="min-w-0 flex-1 truncate text-good"
                              >{ex.effect.name}{#if ex.effect.detail}<span class="text-muted-foreground"> ({ex.effect.detail})</span
                                >{/if}{#if assignedId}<span class="ml-1 text-brand-soft">— exalted</span>{/if}</span
                            >
                          {:else}
                            <span class="min-w-0 flex-1 text-muted-foreground">open — no effect of its own</span>
                          {/if}
                          {#if ex.key !== 'ornament'}
                            <button
                              type="button"
                              class="shrink-0 rounded-sm border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground hover:border-primary/50 hover:text-foreground"
                              onclick={() =>
                                exaltPickerOpen?.itemId === item.id && exaltPickerOpen?.key === ex.key
                                  ? closeExaltPicker()
                                  : openExaltPicker(item.id, ex.key)}
                            >
                              {assignedId ? 'change' : 'fill'}
                            </button>
                          {/if}
                          {#if assignedId}
                            <button
                              type="button"
                              class="shrink-0 rounded-sm border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground hover:border-bad/50 hover:text-bad"
                              onclick={() => clearExalt(item.id, ex.key)}
                            >
                              clear
                            </button>
                          {/if}
                        {/if}
                      </div>
                      {#if exaltPickerOpen?.itemId === item.id && exaltPickerOpen?.key === ex.key}
                        <div class="ml-24 flex flex-col gap-1 rounded-sm border border-border bg-muted/10 p-1.5">
                          <Input bind:value={exaltSearch} placeholder="search relevant pieces…" class="h-6 text-[11px]" />
                          {#if !exaltCandidates.length}
                            <p class="text-[10px] text-muted-foreground">
                              No eligible {ex.label.toLowerCase()} sources for this item, class, and era.
                            </p>
                          {:else if !filteredExaltCandidates.length}
                            <p class="text-[10px] text-muted-foreground">No match for "{exaltSearch}".</p>
                          {:else}
                            <div class="flex max-h-32 flex-col gap-0.5 overflow-y-auto">
                              {#each filteredExaltCandidates as c (c.id)}
                                <button
                                  type="button"
                                  class="rounded-sm px-1 py-0.5 text-left text-[10px] hover:bg-muted/40"
                                  onclick={() => assignExalt(item.id, ex.key, c.id)}
                                >
                                  <span class="text-foreground">{c.name}</span>
                                  <span class="text-muted-foreground"> — {c.effects[ex.key]?.name ?? '?'}</span>
                                </button>
                              {/each}
                            </div>
                          {/if}
                        </div>
                      {/if}
                    </div>
                  {/each}
                </div>
              {/if}
              {#if item.dmg != null && item.delay != null}
                <div class="mt-1.5 text-[11px] tabular-nums text-muted-foreground">
                  {item.dmg} / {item.delay} ({(item.dmg / item.delay).toFixed(2)} ratio){item.skill ? ` — ${item.skill}` : ''}
                </div>
              {/if}
              {#if item.tags.length}
                <div class="mt-1.5 flex flex-wrap gap-1">
                  {#each item.tags as t (t)}
                    <span class="rounded-full border border-border bg-muted/30 px-1.5 py-0.5 text-[10px] text-muted-foreground">{t}</span>
                  {/each}
                </div>
              {/if}
              {#if item.zones.length}
                <p class="mt-1.5 text-[11px]">
                  Drops in: {#each item.zones as z, i (z + i)}{#if i > 0}<span>, </span>{/if}<GdZoneOrMobLink name={z} />{/each}
                </p>
              {/if}
              {#if item.mobs.length}
                <p class="mt-0.5 text-[11px]">From: <GdLinkList kind="npc" names={item.mobs} /></p>
              {/if}
              {#if !item.zones.length && !item.mobs.length && item.source}
                <div class="mt-1.5 text-[11px] text-brand-soft">{item.source}</div>
              {/if}
            {/if}
            </div>
          </CardContent>
        </Card>

        {#if expandedSlot}
          {@const items = bySlot.get(expandedSlot) ?? []}
          {@const label = DOLL_ROWS.flat().find((e) => e && e[0] === expandedSlot)?.[1]}
          <Card class="rounded-sm">
            <CardContent class="px-3 py-2.5">
              <div class="mb-1.5 flex items-center justify-between gap-3">
                <h2 class="panel-title">alternatives · {label}</h2>
                <Input bind:value={altSearch} placeholder="search…" class="h-6 w-36 text-[11px]" />
              </div>
              {#if !items.length}
                <p class="text-[11px] text-muted-foreground">No eligible items for this slot with the current class/race filter.</p>
              {:else if !altSplit.top.length}
                <p class="text-[11px] text-muted-foreground">No match for "{altSearch}".</p>
              {:else}
                <div class="grid max-h-[300px] grid-cols-2 gap-1 overflow-y-auto rounded-sm border border-border p-1">
                  {#each altSplit.top as it, i (it.id)}
                    {@render altRow(it, i === 0 && !altSearch, false)}
                  {/each}
                </div>
                {#if altSplit.other.length}
                  <h3 class="mt-2.5 mb-1 text-[10px] tracking-wide text-muted-foreground uppercase">other ({altSplit.other.length})</h3>
                  <div class="flex max-h-[160px] flex-col overflow-y-auto rounded-sm border border-border p-1">
                    {#each altSplit.other as it (it.id)}
                      {@render altRow(it, false, true)}
                    {/each}
                  </div>
                {/if}
              {/if}
            </CardContent>
          </Card>
        {/if}
      </div>
    </div>
  {/if}
</div>

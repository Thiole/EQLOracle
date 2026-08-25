<script lang="ts">
  // why: 5 wiki catalogs, one filterable list + cross-linked detail page
  // each -- see stores/gamedata.ts's own doc for why page-open state lives
  // there instead of here (a link deep inside a detail page needs to open
  // a different category's page without a callback threaded through
  // every layer in between).
  import * as Tabs from '$lib/components/ui/tabs';
  import { TAB_LIST_CLASS, TAB_TRIGGER_CLASS } from '$lib/navTabs';
  import { Input } from '$lib/components/ui/input';
  import { Card, CardContent } from '$lib/components/ui/card';
  import {
    zones,
    npcs,
    spells,
    aas,
    items,
    gameDataLoaded,
    gdOpen,
    gdFind,
    GD_LABELS,
    loadGameDataModule,
    refreshItems,
    type GdKind,
  } from '$lib/stores/gamedata';
  import { effectiveEra, eraOptions, passesEra } from '$lib/stores/settings';
  import { spellbook, loadCharacterModule } from '$lib/stores/character';
  import { displayZoneName } from '$lib/utils';
  import type { ZoneDto, ItemDto, NpcDto, AaDto, SpellDto } from '$lib/tauri/api';
  import ZonePage from './ZonePage.svelte';
  import ItemPage from './ItemPage.svelte';
  import NpcPage from './NpcPage.svelte';
  import AaPage from './AaPage.svelte';
  import SpellPage from './SpellPage.svelte';

  const ROW_CAP = 300;

  let activeTab = $state<GdKind>('zone');
  let search = $state('');

  $effect(() => {
    void loadGameDataModule();
    // why: the "this session" known/possible column needs $spellbook,
    // which otherwise only loads once Character's own tab has been
    // opened -- idempotent, cheap, safe to also trigger from here
    void loadCharacterModule();
  });

  // why: items are era-filtered server-side (gearplanner::in_era) -- a
  // real re-fetch, not a client-side re-derive, so this re-runs whenever
  // the Settings module's era preference changes (the initial load too,
  // since $effect fires once on mount as well as on each dependency change).
  $effect(() => {
    void refreshItems($effectiveEra);
  });

  // why: a real tab-click handler, not a `$effect` keyed on activeTab --
  // that ran on mount too (activeTab is read there whether it changed or
  // not), which wiped out gdOpenPage's own gdOpen.set(...) the instant a
  // cross-module link (the Gear Planner's "Drops in:") switched here,
  // before this component had even rendered once. A real handler only
  // ever fires on an actual, deliberate tab click.
  function onTabChange(v: string | undefined) {
    if (!v) return;
    activeTab = v as GdKind;
    gdOpen.set(null);
    search = '';
  }

  function matches(name: string, q: string): boolean {
    return !q || name.toLowerCase().includes(q);
  }
  const q = $derived(search.trim().toLowerCase());

  // why: items are already era-filtered server-side by refreshItems --
  // zones/NPCs/spells carry their own flat `era` field instead, filtered
  // client-side here. AAs carry no era field at all (never filtered).
  const filteredZones = $derived(
    $zones
      .filter((z) => matches(z.name, q) && passesEra(z.era, $effectiveEra, $eraOptions))
      .sort((a, b) => a.name.localeCompare(b.name)),
  );
  const filteredItems = $derived(
    $items.filter((it) => matches(it.name, q)).sort((a, b) => a.name.localeCompare(b.name)),
  );
  const filteredNpcs = $derived(
    $npcs
      .filter((n) => matches(n.name, q) && passesEra(n.era, $effectiveEra, $eraOptions))
      .sort((a, b) => a.name.localeCompare(b.name)),
  );
  const filteredAas = $derived($aas.filter((a) => matches(a.name, q)).sort((a, b) => a.name.localeCompare(b.name)));
  const filteredSpells = $derived(
    $spells
      .filter((s) => matches(s.name, q) && passesEra(s.era, $effectiveEra, $eraOptions))
      .sort((a, b) => a.name.localeCompare(b.name)),
  );

  // why: this session's own scribe/memorize evidence, linked to the
  // catalog by name -- moved here from Character's own "Known Spells"
  // tab (a passive session log fits better as a column on the catalog
  // it's already about than as a separate page)
  const knownByName = $derived(new Map($spellbook.map((s) => [s.name, s.confidence])));

  const totalForTab: Record<GdKind, number> = $derived({
    zone: filteredZones.length,
    item: filteredItems.length,
    npc: filteredNpcs.length,
    aa: filteredAas.length,
    spell: filteredSpells.length,
  });

  const openEntry = $derived($gdOpen ? gdFind($gdOpen.kind, $gdOpen.key) : undefined);
</script>

<div class="flex flex-col gap-3 p-3">
  {#if !$gameDataLoaded}
    <p class="text-[12px] text-muted-foreground">Loading game data…</p>
  {:else}
    <Tabs.Root value={$gdOpen?.kind ?? activeTab} onValueChange={onTabChange}>
      <Tabs.List class={TAB_LIST_CLASS}>
        <Tabs.Trigger value="zone" class={TAB_TRIGGER_CLASS}>Zones</Tabs.Trigger>
        <Tabs.Trigger value="item" class={TAB_TRIGGER_CLASS}>Items</Tabs.Trigger>
        <Tabs.Trigger value="npc" class={TAB_TRIGGER_CLASS}>NPCs</Tabs.Trigger>
        <Tabs.Trigger value="aa" class={TAB_TRIGGER_CLASS}>AAs</Tabs.Trigger>
        <Tabs.Trigger value="spell" class={TAB_TRIGGER_CLASS}>Spells</Tabs.Trigger>
      </Tabs.List>
    </Tabs.Root>

    {#if $gdOpen}
      <!-- why: a real button, not another Card -- each page component
           below brings its own separate boxes (info / cross-refs / your
           session history), not one big card the back link would get
           lost inside. -->
      <button
        type="button"
        class="self-start text-[11px] text-brand-soft hover:text-primary hover:underline"
        onclick={() => {
          // why: a cross-link (Gear Planner's "Drops in:", or another
          // page's own cross-reference) can open a page whose kind isn't
          // whatever tab was last clicked here -- land back on *that*
          // page's own list, not a stale, unrelated one.
          if ($gdOpen) activeTab = $gdOpen.kind;
          gdOpen.set(null);
        }}
      >
        ← back to {GD_LABELS[$gdOpen?.kind ?? activeTab]}
      </button>
      {#if !openEntry}
        <Card class="rounded-sm">
          <CardContent class="px-3 py-2.5">
            <p class="text-[12px] text-muted-foreground">That entry isn't in the catalog.</p>
          </CardContent>
        </Card>
      {:else if $gdOpen.kind === 'zone'}
        <ZonePage zone={openEntry as ZoneDto} />
      {:else if $gdOpen.kind === 'item'}
        <ItemPage item={openEntry as ItemDto} />
      {:else if $gdOpen.kind === 'npc'}
        <NpcPage npc={openEntry as NpcDto} />
      {:else if $gdOpen.kind === 'aa'}
        <AaPage aa={openEntry as AaDto} />
      {:else}
        <SpellPage spell={openEntry as SpellDto} />
      {/if}
    {:else}
      <Card class="rounded-sm">
        <CardContent class="px-3 py-2.5">
          <div class="mb-2 flex items-center justify-between gap-3">
            <p class="text-[11px] text-muted-foreground">
              {totalForTab[activeTab]} {GD_LABELS[activeTab].toLowerCase()}
            </p>
            <Input bind:value={search} placeholder="filter by name…" class="h-6 w-48 text-[11px]" />
          </div>

          {#if activeTab === 'zone'}
            <table class="w-full text-[11px]">
              <thead><tr class="text-left text-muted-foreground"><th class="pb-1 font-normal">zone</th><th class="pb-1 font-normal">era</th><th class="pb-1 font-normal">level range</th></tr></thead>
              <tbody>
                {#each filteredZones.slice(0, ROW_CAP) as z (z.id)}
                  <tr class="cursor-pointer hover:bg-muted/40" onclick={() => gdOpen.set({ kind: 'zone', key: z.name })}>
                    <td class="py-0.5">{displayZoneName(z.name)}</td>
                    <td class="py-0.5 text-muted-foreground">{z.era ?? '—'}</td>
                    <td class="py-0.5 text-muted-foreground">{z.level_range ?? '—'}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else if activeTab === 'item'}
            <table class="w-full text-[11px]">
              <thead><tr class="text-left text-muted-foreground"><th class="pb-1 font-normal">item</th><th class="pb-1 font-normal">slot(s)</th><th class="pb-1 font-normal">class(es)</th><th class="pb-1 font-normal">era</th></tr></thead>
              <tbody>
                {#each filteredItems.slice(0, ROW_CAP) as it (it.id)}
                  <tr class="cursor-pointer hover:bg-muted/40" onclick={() => gdOpen.set({ kind: 'item', key: it.id })}>
                    <td class="py-0.5">{it.name}</td>
                    <td class="py-0.5 text-muted-foreground">{it.slots.join(', ') || '—'}</td>
                    <td class="py-0.5 text-muted-foreground">{it.classes.join(', ') || 'any'}</td>
                    <td class="py-0.5 text-muted-foreground">{it.era ?? '—'}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else if activeTab === 'npc'}
            <table class="w-full text-[11px]">
              <thead><tr class="text-left text-muted-foreground"><th class="pb-1 font-normal">npc</th><th class="pb-1 font-normal">zone</th><th class="pb-1 font-normal">level</th></tr></thead>
              <tbody>
                {#each filteredNpcs.slice(0, ROW_CAP) as n (n.id)}
                  <tr class="cursor-pointer hover:bg-muted/40" onclick={() => gdOpen.set({ kind: 'npc', key: n.name })}>
                    <td class="py-0.5">{n.name}</td>
                    <td class="py-0.5 text-muted-foreground">{n.zone ?? '—'}</td>
                    <td class="py-0.5 text-muted-foreground">{n.level ?? '—'}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else if activeTab === 'aa'}
            <table class="w-full text-[11px]">
              <thead><tr class="text-left text-muted-foreground"><th class="pb-1 font-normal">ability</th><th class="pb-1 font-normal">class</th><th class="pb-1 font-normal">ranks</th><th class="pb-1 font-normal">cost</th></tr></thead>
              <tbody>
                {#each filteredAas.slice(0, ROW_CAP) as a (a.name + '::' + a.category)}
                  <tr class="cursor-pointer hover:bg-muted/40" onclick={() => gdOpen.set({ kind: 'aa', key: `${a.name}::${a.category}` })}>
                    <td class="py-0.5">{a.name}</td>
                    <td class="py-0.5 text-muted-foreground">{a.category}</td>
                    <td class="py-0.5 text-muted-foreground">{a.ranks}</td>
                    <td class="py-0.5 text-muted-foreground">{a.cost_raw}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {:else}
            <table class="w-full text-[11px]">
              <thead><tr class="text-left text-muted-foreground"><th class="pb-1 font-normal">spell</th><th class="pb-1 font-normal">class(es)</th><th class="pb-1 font-normal">mana</th><th class="pb-1 font-normal">cast time</th><th class="pb-1 font-normal">this session</th></tr></thead>
              <tbody>
                {#each filteredSpells.slice(0, ROW_CAP) as s (s.id)}
                  <tr class="cursor-pointer hover:bg-muted/40" onclick={() => gdOpen.set({ kind: 'spell', key: s.id })}>
                    <td class="py-0.5">{s.name}</td>
                    <td class="py-0.5 text-muted-foreground">{s.classes.map((c) => (c.level != null ? `${c.class} ${c.level}` : c.class)).join(', ') || '—'}</td>
                    <td class="py-0.5 text-muted-foreground">{s.mana ?? '—'}</td>
                    <td class="py-0.5 text-muted-foreground">{s.casting_time != null ? `${s.casting_time}s` : '—'}</td>
                    <td class="py-0.5">
                      {#if knownByName.get(s.name) === 'known'}
                        <span class="text-primary">known</span>
                      {:else if knownByName.get(s.name) === 'possible'}
                        <span class="text-muted-foreground">possible</span>
                      {:else}
                        <span class="text-muted-foreground">—</span>
                      {/if}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          {/if}

          {#if totalForTab[activeTab] === 0}
            <p class="mt-2 text-[11px] text-muted-foreground">No {GD_LABELS[activeTab].toLowerCase()} match that filter.</p>
          {:else if totalForTab[activeTab] > ROW_CAP}
            <p class="mt-2 text-[11px] text-muted-foreground">Showing {ROW_CAP} of {totalForTab[activeTab]} — narrow your search to see the rest.</p>
          {/if}
        </CardContent>
      </Card>
    {/if}
  {/if}
</div>

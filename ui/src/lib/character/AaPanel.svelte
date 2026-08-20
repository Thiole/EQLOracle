<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Checkbox } from '$lib/components/ui/checkbox';
  import { aaLog, aaCatalog, classConfigurations, activeClasses } from '$lib/stores/character';
  import { CLASS_CODE, ARCHETYPE_AA_CLASSES } from './constants';
  import ClassTag from './ClassTag.svelte';
  import SortableTh from './SortableTh.svelte';
  import type { AaDto } from '$lib/tauri/api';

  interface Row extends AaDto {
    ownedRank: number | null;
    totalCost: number;
    classCode: string | null;
  }

  // why: newest purchase wins, one owned rank per name
  const ownedByName = $derived.by(() => {
    const m = new Map<string, number>();
    for (const g of $aaLog?.grants ?? []) {
      if (g.rank > (m.get(g.name) ?? 0)) m.set(g.name, g.rank);
    }
    return m;
  });

  function toRow(a: AaDto): Row {
    const totalCost = a.cost_raw
      .split('/')
      .map(Number)
      .filter((n) => !isNaN(n))
      .reduce((s, n) => s + n, 0);
    return { ...a, ownedRank: ownedByName.get(a.name) ?? null, totalCost, classCode: CLASS_CODE[a.category] ?? null };
  }

  const generalRows = $derived($aaCatalog.filter((a) => a.category === 'general').map(toRow));
  const archetypeRows = $derived($aaCatalog.filter((a) => a.category === 'archetype').map(toRow));
  const classRowsAll = $derived($aaCatalog.filter((a) => a.category !== 'general' && a.category !== 'archetype').map(toRow));

  let hideInactive = $state(true);
  const classRows = $derived(hideInactive ? classRowsAll.filter((r) => $activeClasses.includes(r.category)) : classRowsAll);

  const everPlayed = $derived.by(() => {
    const set = new Set<string>();
    for (const cfg of $classConfigurations?.configurations ?? []) for (const c of cfg.classes) set.add(c);
    return set;
  });

  type SortKey = 'name' | 'ranks' | 'totalCost' | 'ownedRank' | 'classCode';
  function makeSort() {
    let state = $state<{ key: SortKey; dir: 1 | -1 }>({ key: 'name', dir: 1 });
    return {
      get key() {
        return state.key;
      },
      get dir() {
        return state.dir;
      },
      toggle(key: SortKey) {
        state = state.key === key ? { key, dir: (state.dir * -1) as 1 | -1 } : { key, dir: 1 };
      },
      apply(rows: Row[]): Row[] {
        const { key, dir } = state;
        return [...rows].sort((a, b) => {
          const av = a[key];
          const bv = b[key];
          if (av == null && bv == null) return 0;
          if (av == null) return 1;
          if (bv == null) return -1;
          if (typeof av === 'string') return dir * av.localeCompare(bv as string);
          return dir * ((av as number) - (bv as number));
        });
      },
    };
  }
  const generalSort = makeSort();
  const archetypeSort = makeSort();
  const classSort = makeSort();

  const sortedGeneral = $derived(generalSort.apply(generalRows));
  const sortedArchetype = $derived(archetypeSort.apply(archetypeRows));
  const sortedClass = $derived(classSort.apply(classRows));

  function classTagClass(category: string): string {
    if ($activeClasses.includes(category)) return 'text-primary';
    if (everPlayed.has(category)) return 'text-caution';
    return 'text-muted-foreground';
  }
</script>

{#snippet aaRow(r: Row, showClass: boolean)}
  <tr class="border-b border-border/50">
    {#if showClass}
      <td class="px-2 py-1">{#if r.classCode}<ClassTag code={r.classCode} />{/if}</td>
    {/if}
    <td class="px-2 py-1" title={r.description ?? undefined}>
      {r.name}{#if !r.certain}<span class="ml-1 text-caution" title="catalog data uncertain">~</span>{/if}
      {#if r.category === 'archetype' && ARCHETYPE_AA_CLASSES[r.name]}
        <div class="mt-0.5 flex flex-wrap gap-0.5">
          {#each ARCHETYPE_AA_CLASSES[r.name] as c (c)}
            <ClassTag code={c} muted />
          {/each}
        </div>
      {/if}
    </td>
    <td class="px-2 py-1 text-right tabular-nums">{r.ranks}</td>
    <td class="px-2 py-1 text-right tabular-nums text-muted-foreground">{r.cost_raw}</td>
    <td class="px-2 py-1 text-right tabular-nums {r.ownedRank ? 'text-primary' : 'text-muted-foreground'}">
      {r.ownedRank ? `${r.ownedRank}/${r.ranks}` : '—'}
    </td>
  </tr>
{/snippet}

<div class="flex flex-col gap-4">
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-1">general</h2>
      <p class="mb-2 text-[11px] text-muted-foreground">Available to every class. {sortedGeneral.length} total.</p>
      <div class="overflow-x-auto">
        <table class="w-full text-[11px]">
          <thead>
            <tr class="border-b border-border">
              <SortableTh label="name" active={generalSort.key === 'name'} dir={generalSort.dir} onclick={() => generalSort.toggle('name')} />
              <SortableTh
                label="ranks"
                align="right"
                active={generalSort.key === 'ranks'}
                dir={generalSort.dir}
                onclick={() => generalSort.toggle('ranks')}
              />
              <SortableTh
                label="cost"
                align="right"
                active={generalSort.key === 'totalCost'}
                dir={generalSort.dir}
                onclick={() => generalSort.toggle('totalCost')}
              />
              <SortableTh
                label="owned"
                align="right"
                active={generalSort.key === 'ownedRank'}
                dir={generalSort.dir}
                onclick={() => generalSort.toggle('ownedRank')}
              />
            </tr>
          </thead>
          <tbody>
            {#each sortedGeneral as r, i (r.name + r.category + i)}
              {@render aaRow(r, false)}
            {/each}
          </tbody>
        </table>
      </div>
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-1">archetype</h2>
      <p class="mb-2 text-[11px] text-muted-foreground">
        Shared by a group of classes. Eligible-class hints below each name are best-effort, not from the scrape -- never used to hide a row.
        {sortedArchetype.length} total.
      </p>
      <div class="overflow-x-auto">
        <table class="w-full text-[11px]">
          <thead>
            <tr class="border-b border-border">
              <SortableTh
                label="name"
                active={archetypeSort.key === 'name'}
                dir={archetypeSort.dir}
                onclick={() => archetypeSort.toggle('name')}
              />
              <SortableTh
                label="ranks"
                align="right"
                active={archetypeSort.key === 'ranks'}
                dir={archetypeSort.dir}
                onclick={() => archetypeSort.toggle('ranks')}
              />
              <SortableTh
                label="cost"
                align="right"
                active={archetypeSort.key === 'totalCost'}
                dir={archetypeSort.dir}
                onclick={() => archetypeSort.toggle('totalCost')}
              />
              <SortableTh
                label="owned"
                align="right"
                active={archetypeSort.key === 'ownedRank'}
                dir={archetypeSort.dir}
                onclick={() => archetypeSort.toggle('ownedRank')}
              />
            </tr>
          </thead>
          <tbody>
            {#each sortedArchetype as r, i (r.name + r.category + i)}
              {@render aaRow(r, false)}
            {/each}
          </tbody>
        </table>
      </div>
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <div class="mb-1 flex items-center justify-between gap-3">
        <h2 class="panel-title">class</h2>
        <label class="flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <Checkbox checked={hideInactive} onCheckedChange={(v: boolean) => (hideInactive = v)} />
          hide non-active classes
        </label>
      </div>
      <p class="mb-2 text-[11px] text-muted-foreground">
        {sortedClass.length} of {classRowsAll.length} shown{#if !$activeClasses.length} — mark active classes on the Character tab to filter{/if}.
      </p>
      <div class="overflow-x-auto">
        <table class="w-full text-[11px]">
          <thead>
            <tr class="border-b border-border">
              <SortableTh
                label="class"
                active={classSort.key === 'classCode'}
                dir={classSort.dir}
                onclick={() => classSort.toggle('classCode')}
              />
              <SortableTh label="name" active={classSort.key === 'name'} dir={classSort.dir} onclick={() => classSort.toggle('name')} />
              <SortableTh
                label="ranks"
                align="right"
                active={classSort.key === 'ranks'}
                dir={classSort.dir}
                onclick={() => classSort.toggle('ranks')}
              />
              <SortableTh
                label="cost"
                align="right"
                active={classSort.key === 'totalCost'}
                dir={classSort.dir}
                onclick={() => classSort.toggle('totalCost')}
              />
              <SortableTh
                label="owned"
                align="right"
                active={classSort.key === 'ownedRank'}
                dir={classSort.dir}
                onclick={() => classSort.toggle('ownedRank')}
              />
            </tr>
          </thead>
          <tbody>
            {#each sortedClass as r, i (r.name + r.category + i)}
              {@render aaRow(r, true)}
            {/each}
          </tbody>
        </table>
      </div>
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-1">purchase log</h2>
      {#if !$aaLog || !$aaLog.grants.length}
        <p class="text-[12px] text-muted-foreground">No AA purchases parsed yet.</p>
      {:else}
        <p class="mb-2 text-[11px] text-muted-foreground">
          <b class="tabular-nums">{$aaLog.grants.length}</b> rank{$aaLog.grants.length === 1 ? '' : 's'} purchased this session,
          <b class="tabular-nums">{$aaLog.total_spent}</b> point{$aaLog.total_spent === 1 ? '' : 's'} spent.
        </p>
        <div class="overflow-x-auto">
          <table class="w-full text-[11px]">
            <thead>
              <tr class="border-b border-border text-muted-foreground">
                <th class="px-2 py-0.5 text-left font-normal">when</th>
                <th class="px-2 py-0.5 text-left font-normal">ability</th>
                <th class="px-2 py-0.5 text-right font-normal">rank</th>
                <th class="px-2 py-0.5 text-right font-normal">cost</th>
                <th class="px-2 py-0.5 text-left font-normal">class</th>
              </tr>
            </thead>
            <tbody>
              {#each [...$aaLog.grants].reverse() as g, i (`${g.ts_ms}-${g.name}-${g.rank}-${i}`)}
                <tr class="border-b border-border/50">
                  <td class="px-2 py-0.5 whitespace-nowrap text-muted-foreground">{new Date(g.ts_ms).toLocaleString()}</td>
                  <td class="px-2 py-0.5" title={g.description ?? undefined}>{g.name}</td>
                  <td class="px-2 py-0.5 text-right tabular-nums">{g.max_rank && g.max_rank > 1 ? `${g.rank} / ${g.max_rank}` : g.rank}</td>
                  <td class="px-2 py-0.5 text-right tabular-nums">{g.cost}</td>
                  <td class="px-2 py-0.5 {classTagClass(g.category ?? '')}">{g.category ?? 'uncatalogued'}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </CardContent>
  </Card>
</div>

<script lang="ts">
  import * as Table from '$lib/components/ui/table';
  import type { AllyDto, CombatSummaryDto } from '$lib/tauri/api';
  import { allies, expandedAlly, allySummary, petSummaries, toggleAlly, scope, refreshSelection } from '$lib/stores/combat';
  import { api } from '$lib/tauri/api';
  import ContextMenu, { type MenuItem } from '$lib/shell/ContextMenu.svelte';
  import { trackedSkills, toggleTrackedSkill } from '$lib/stores/settings';
  import TargetIcon from '@lucide/svelte/icons/target';
  import { sortRows, nextSort, loadCols, saveCols, type Dir } from './grid';
  import type { AbilityRowDto } from '$lib/tauri/api';

  // why: the enemy listing is the same rows minus the ally-side concepts
  // -- a mob's detected class is noise, its chain never confirms, and
  // "suggested" (proven groupmate or not) means nothing about a mob
  let { rows = null, allySide = true, empty = 'No fights parsed for this selection yet.' }:
    { rows?: AllyDto[] | null; allySide?: boolean; empty?: string } = $props();
  const list = $derived(rows ?? $allies);

  // why: right-click on a mob row -- "this is X's pet" for the viewed
  // visit; owners are You and whoever the party roster currently holds
  let menu = $state<{ x: number; y: number; name: string } | null>(null);
  let party = $state<string[]>([]);
  async function openMenu(e: MouseEvent, a: AllyDto) {
    if (a.is_player) return;
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY, name: a.name };
    const gs = await api.getGameState().catch(() => null);
    party = ['You', ...(gs?.party ?? []).map((p) => p.name).filter((n) => n !== 'You')];
  }
  async function assign(pet: string, owner: string | null) {
    const { zv, enc } = scope();
    await api.setPetOwner(zv, enc, pet, owner).catch(() => {});
    await refreshSelection(true);
  }
  async function hide(name: string, hidden: boolean) {
    const { zv, enc } = scope();
    await api.setEntityHidden(zv, enc, name, hidden).catch(() => {});
    await refreshSelection(true);
  }
  const menuItems = $derived.by((): MenuItem[] => {
    const m = menu;
    if (!m) return [];
    const row = list.find((a) => a.name === m.name);
    return [
      { label: 'assign to player', children: party.map((p) => ({ label: p, onSelect: () => void assign(m.name, p) })) },
      { label: 'clear owner', onSelect: () => void assign(m.name, null) },
      row?.hidden
        ? { label: 'unhide', onSelect: () => void hide(m.name, false) }
        : { label: 'hide', onSelect: () => void hide(m.name, true) },
    ];
  });

  // ---------------------------------------------------------------- grid
  // why: every numeric column the row carries; the default set is what
  // the table always showed, the rest is a click away
  type AllyCol = 'total' | 'pct' | 'dps' | 'hits' | 'crits' | 'crit_pct' | 'hit_pct' | 'resist_pct' | 'pet_total';
  const ALLY_COLS: { key: AllyCol; label: string; def: boolean }[] = [
    { key: 'total', label: 'total', def: true },
    { key: 'pct', label: '%', def: true },
    { key: 'dps', label: 'dps', def: true },
    { key: 'hits', label: 'hits', def: true },
    { key: 'crits', label: 'crits', def: false },
    { key: 'crit_pct', label: 'crit%', def: true },
    { key: 'hit_pct', label: 'hit%', def: false },
    { key: 'resist_pct', label: 'resist%', def: false },
    { key: 'pet_total', label: 'pet dmg', def: false },
  ];
  const store = $derived(allySide ? 'ally' : 'enemy');
  // svelte-ignore state_referenced_locally -- a table is one side for its whole life
  let visible = $state(loadCols(allySide ? 'ally' : 'enemy', [...(allySide ? ['class'] : []), ...ALLY_COLS.filter((c) => c.def).map((c) => c.key)]));
  function toggleCol(key: string) {
    const next = new Set(visible);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    visible = next;
    saveCols(store, next);
  }
  const shown = $derived(ALLY_COLS.filter((c) => visible.has(c.key)));
  const cols = $derived(1 + (allySide && visible.has('class') ? 1 : 0) + shown.length);
  let sort = $state<{ key: AllyCol | 'name'; dir: Dir }>({ key: 'total', dir: -1 });
  // why: hidden rows keep their data; "show hidden" brings them back dimmed so they can be unhidden
  let showHidden = $state(false);
  const hiddenCount = $derived(list.filter((a) => a.hidden).length);
  const sorted = $derived(sortRows(showHidden ? list : list.filter((a) => !a.hidden), sort.key, sort.dir));
  const arrow = (key: string, cur: { key: string; dir: Dir }) => (cur.key === key ? (cur.dir === -1 ? ' ▼' : ' ▲') : '');
  const pctCell = (v: number | null) => (v == null ? '—' : `${v.toFixed(1)}%`);
  function allyCell(a: AllyDto, key: AllyCol): string {
    switch (key) {
      case 'total':
      case 'hits':
      case 'crits':
      case 'pet_total':
        return a[key].toLocaleString();
      case 'dps':
        return a.dps.toFixed(1);
      default:
        return pctCell(a[key]);
    }
  }
  let colsOpen = $state(false);

  // why: the ability grid under an expanded row -- sorted, drilled-down
  // specific data per ability; dps runs on the selection's fight time
  type AbilityCol = 'total' | 'pct' | 'dps' | 'hits' | 'avg_hit' | 'avg_crit' | 'crits' | 'min' | 'max' | 'avoided' | 'casts' | 'resisted' | 'interrupted' | 'fizzled';
  const ABILITY_COLS: { key: AbilityCol; label: string; def: boolean }[] = [
    { key: 'total', label: 'total', def: true },
    { key: 'pct', label: 'share', def: true },
    { key: 'dps', label: 'dps', def: true },
    { key: 'hits', label: 'hits', def: true },
    { key: 'avg_hit', label: 'avg', def: true },
    { key: 'avg_crit', label: 'crit avg', def: true },
    { key: 'crits', label: 'crits', def: false },
    { key: 'min', label: 'min', def: false },
    { key: 'max', label: 'max', def: false },
    { key: 'avoided', label: 'avoided', def: true },
    // why: Spencer -- "dont make spell casts its own section, loop it
    // into the columns under abilities": a cast row joins the ability it
    // landed as ("Harm Touch" cast, "Harm Touch X" landed)
    { key: 'casts', label: 'casts', def: true },
    { key: 'resisted', label: 'resisted', def: true },
    { key: 'interrupted', label: 'interrupted', def: false },
    { key: 'fizzled', label: 'fizzled', def: false },
  ];
  type AbilityGridRow = AbilityRowDto & { avoided: number; casts: number; landed: number; resisted: number; interrupted: number; fizzled: number };
  // why: the log appends a live rank to a landed ability ("Harm Touch X")
  // but never to its cast line; the backend keys casts rank-stripped
  const ROMAN = /^[IVXLC]+$/;
  function baseOf(name: string): string {
    const i = name.lastIndexOf(' ');
    return i > 0 && ROMAN.test(name.slice(i + 1)) ? name.slice(0, i) : name;
  }
  const EMPTY_ABILITY: Omit<AbilityRowDto, 'ability'> = { tags: [], total: 0, hits: 0, min: 0, max: 0, crits: 0, avg_hit: 0, avg_crit: 0, pct: 0, dps: 0, missed: 0, blocked: 0, dodged: 0, parried: 0 } as Omit<AbilityRowDto, 'ability'>;
  let abilityVisible = $state(loadCols('ability', ABILITY_COLS.filter((c) => c.def).map((c) => c.key)));
  function toggleAbilityCol(key: string) {
    const next = new Set(abilityVisible);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    abilityVisible = next;
    saveCols('ability', next);
  }
  const abilityShown = $derived(ABILITY_COLS.filter((c) => abilityVisible.has(c.key)));
  let abilitySort = $state<{ key: AbilityCol | 'ability'; dir: Dir }>({ key: 'total', dir: -1 });
  // why: one grid, many entities -- the owner's own rows and each pet's
  function rowsOf(summary: CombatSummaryDto | null | undefined): AbilityGridRow[] {
    const noCast = { casts: 0, landed: 0, resisted: 0, interrupted: 0, fizzled: 0 };
    const rows: AbilityGridRow[] = (summary?.abilities ?? []).map((ab) => ({ ...ab, ...noCast, avoided: ab.missed + ab.blocked + ab.dodged + ab.parried }));
    for (const c of summary?.casts ?? []) {
      const key = c.spell.toLowerCase();
      // why: exact name first, then the rank-stripped landed name
      const row = rows.find((r) => r.ability.toLowerCase() === key) ?? rows.find((r) => baseOf(r.ability).toLowerCase() === key);
      const target = row ?? (rows.push({ ...EMPTY_ABILITY, ability: c.spell, ...noCast, avoided: 0 }), rows[rows.length - 1]);
      target.casts += c.attempts;
      target.landed += c.landed;
      target.resisted += c.resisted;
      target.interrupted += c.interrupted;
      target.fizzled += c.fizzled;
    }
    return sortRows(rows, abilitySort.key, abilitySort.dir);
  }
  const abilityRows = $derived(rowsOf($allySummary));
  // why: the two slices under an owner with a pet -- shares the row's
  // own time window, so % and dps scale with the split of the total
  type Part = { name: string; label: string; pet: boolean; total: number; hits: number; summary: CombatSummaryDto | undefined };
  function partsOf(a: AllyDto): Part[] {
    const petTotal = a.pets.reduce((n, p) => n + p.total, 0);
    const petHits = a.pets.reduce((n, p) => n + p.hits, 0);
    return [
      { name: a.name, label: `${a.name} directly`, pet: false, total: a.total - petTotal, hits: Math.max(0, a.hits - petHits), summary: $allySummary ?? undefined },
      ...a.pets.map((p) => ({ name: p.name, label: p.name, pet: true, total: p.total, hits: p.hits, summary: $petSummaries[p.name] })),
    ];
  }
  function partCell(part: Part, a: AllyDto, key: AllyCol): string {
    const share = a.total > 0 ? part.total / a.total : 0;
    switch (key) {
      case 'total':
        return part.total.toLocaleString();
      case 'hits':
        return part.hits.toLocaleString();
      case 'pct':
        return pctCell(a.pct * share);
      case 'dps':
        return (a.dps * share).toFixed(1);
      case 'crits':
      case 'crit_pct': {
        const abs = part.summary?.abilities ?? [];
        const crits = abs.reduce((n, ab) => n + ab.crits, 0);
        const hits = abs.reduce((n, ab) => n + ab.hits, 0);
        return key === 'crits' ? crits.toLocaleString() : hits ? pctCell((crits / hits) * 100) : '';
      }
      default:
        return '';
    }
  }
  let expandedPart = $state<string | null>(null);
  $effect(() => {
    void $expandedAlly;
    expandedPart = null;
  });
  let abilityColsOpen = $state(false);
  function abilityCell(ab: AbilityGridRow, key: AbilityCol): string {
    switch (key) {
      case 'pct':
        return `${ab.pct.toFixed(1)}%`;
      case 'dps':
        return (ab.dps ?? 0).toFixed(1);
      case 'avg_hit':
        return ab.casts && !ab.hits ? '' : ab.avg_hit.toFixed(0);
      case 'avg_crit':
        return ab.crits > 0 ? ab.avg_crit.toFixed(0) : '—';
      case 'avoided':
        return ab.avoided ? String(ab.avoided) : '';
      case 'casts':
        return ab.casts ? `${ab.landed}/${ab.casts}` : '';
      case 'resisted':
      case 'interrupted':
      case 'fizzled':
        return ab[key] ? String(ab[key]) : '';
      case 'total':
      case 'hits':
      case 'crits':
      case 'min':
      case 'max':
        return ab.casts && !ab.hits && key !== 'total' ? '' : ab[key].toLocaleString();
      default:
        return '';
    }
  }
  function avoidedTitle(ab: AbilityGridRow): string {
    return [ab.missed && `${ab.missed} miss`, ab.blocked && `${ab.blocked} blocked`, ab.dodged && `${ab.dodged} dodged`, ab.parried && `${ab.parried} parried`]
      .filter(Boolean)
      .join(', ');
  }
  // why: the game's own three-letter codes, as /who prints them
  const ABBR: Record<string, string> = {
    Warrior: 'WAR', Cleric: 'CLR', Paladin: 'PAL', Ranger: 'RNG', 'Shadow Knight': 'SHD', Druid: 'DRU',
    Monk: 'MNK', Bard: 'BRD', Rogue: 'ROG', Shaman: 'SHM', Necromancer: 'NEC', Wizard: 'WIZ',
    Magician: 'MAG', Enchanter: 'ENC', Beastlord: 'BST', Berserker: 'BER',
  };
  const abbr = (c: string) => ABBR[c] ?? c.slice(0, 3).toUpperCase();
</script>

{#snippet entityBlock(rows: AbilityGridRow[], summary: CombatSummaryDto, label: string | null, total: number | null)}
  <div class="flex flex-col gap-3">
    {#if label}
      <h4 class="flex items-center justify-between border-b border-border pb-0.5 text-[11px]">
        <span class="font-medium text-foreground">{label}</span>
        {#if total != null}<span class="font-mono tabular-nums text-muted-foreground">{total.toLocaleString()}</span>{/if}
      </h4>
    {/if}
                <div>
                  <h4 class="mb-1 flex items-center justify-between text-[10px] uppercase tracking-wide text-muted-foreground">
                    <span>abilities</span>
                    <button type="button" class="normal-case tracking-normal hover:text-foreground" onclick={() => (abilityColsOpen = !abilityColsOpen)}>columns</button>
                  </h4>
                  {#if abilityColsOpen}
                    <div class="mb-1 flex flex-wrap gap-x-3 gap-y-1 text-[11px]">
                      {#each ABILITY_COLS as c (c.key)}
                        <label class="flex items-center gap-1"><input type="checkbox" checked={abilityVisible.has(c.key)} onchange={() => toggleAbilityCol(c.key)} /> {c.label}</label>
                      {/each}
                    </div>
                  {/if}
                  <!-- why: ten columns outrun a 1366px window -- scroll the grid, never clip it -->
                  <div class="overflow-x-auto">
                  <table class="w-full text-[11px]">
                    <thead>
                      <tr class="border-b border-border text-muted-foreground">
                        <th class="py-0.5 text-left font-normal"><button type="button" class="select-none" onclick={() => (abilitySort = nextSort(abilitySort, 'ability', false))}>ability{arrow('ability', abilitySort)}</button></th>
                        {#each abilityShown as c (c.key)}
                          <th class="py-0.5 text-right font-normal"><button type="button" class="select-none" onclick={() => (abilitySort = nextSort(abilitySort, c.key, true))}>{c.label}{arrow(c.key, abilitySort)}</button></th>
                        {/each}
                      </tr>
                    </thead>
                    <tbody>
                      {#each rows as ab (ab.ability)}
                        <tr class="group border-b border-border/50">
                          <td class="py-0.5">
                            <span class="inline-flex items-center gap-1">
                              {ab.ability}
                              <!-- why: the target icon adds/removes it from
                                   the Skill Tracker overlay's cooldowns section -->
                              <button
                                type="button"
                                class="rounded-sm p-0.5 {$trackedSkills.includes(ab.ability)
                                  ? 'text-primary'
                                  : 'text-muted-foreground opacity-0 group-hover:opacity-100'}"
                                title={$trackedSkills.includes(ab.ability)
                                  ? `Stop tracking ${ab.ability}`
                                  : `Track ${ab.ability} in the Skill Tracker overlay`}
                                onclick={() => void toggleTrackedSkill(ab.ability)}
                              >
                                <TargetIcon class="size-3" />
                              </button>
                            </span>
                          </td>
                          {#each abilityShown as c (c.key)}
                            <td class="py-0.5 text-right tabular-nums {c.key === 'avoided' ? 'text-bad' : c.key === 'total' || c.key === 'dps' ? '' : 'text-muted-foreground'}" title={c.key === 'avoided' ? avoidedTitle(ab) : undefined}>{abilityCell(ab, c.key)}</td>
                          {/each}
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                  </div>
                </div>
  </div>
{/snippet}

{#snippet ownerExtras(a: AllyDto)}
                <!-- why: an observed swing rate, and labelled as one.
                     The log timestamps whole seconds and haste, dual
                     wield and double attack all sit on the weapon's own
                     delay, so this cannot be that number and does not
                     claim to be. Specials are excluded -- Bash, Kick,
                     Backstab and Frenzy run on their own timers. -->
                {#if a.melee_rate && a.melee_rate.rounds > 1}
                  <div class="text-[11px]">
                    <h4 class="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
                      melee swing rate
                    </h4>
                    <p class="flex justify-between gap-3">
                      <span>seconds between swing rounds</span>
                      <span class="font-mono tabular-nums">{a.melee_rate.secs_between_rounds.toFixed(2)}s</span>
                    </p>
                    <p class="flex justify-between gap-3">
                      <span>swings per round</span>
                      <span class="font-mono tabular-nums">
                        {(a.melee_rate.swings / a.melee_rate.rounds).toFixed(2)}
                      </span>
                    </p>
                    <p class="flex justify-between gap-3 text-muted-foreground">
                      <span>{a.melee_rate.swings.toLocaleString()} swings over {a.melee_rate.rounds.toLocaleString()} rounds</span>
                    </p>
                  </div>
                {/if}
                {#if allySide && a.class_source !== 'who' && (a.classes.length < 3 || a.class_prior.length || a.class_conflicts || a.class_chain_end)}
                  <!-- why: Q34 -- what the open slot is stuck between, and the chain's state -->
                  <div class="text-[11px]">
                    <h4 class="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">class detection</h4>
                    {#if a.classes.length < 3}
                      <p>open slot{a.class_candidates.length ? `, between: ${a.class_candidates.join(', ')}` : ': no candidates yet'}</p>
                    {/if}
                    {#if a.class_prior.length}
                      <p>carried as prior, reconfirming: {a.class_prior.join(', ')}</p>
                    {/if}
                    {#if a.class_conflicts}
                      <p class="text-caution">{a.class_conflicts} conflicting encounter{a.class_conflicts === 1 ? '' : 's'} running (3 close the chain)</p>
                    {/if}
                    {#if a.class_chain_end === '??'}
                      <p class="text-bad">chain closed by contradiction -- a new one is confirming</p>
                    {:else if a.class_chain_end === 'swap'}
                      <p class="text-caution">chain closed by a loadout swap signal</p>
                    {:else if a.class_chain_end === 'presence'}
                      <!-- why: not a swap: a new presence (absence, your zone
                           line, a group change). This fight keeps what was known
                           in it; detection since then started clean. -->
                      <p class="text-muted-foreground">this presence ended (absence, your zone line or a group change) -- detection restarted after it</p>
                    {/if}
                  </div>
                {/if}
{/snippet}


{#if list.length === 0}
  <p class="py-4 text-[12px] text-muted-foreground">{empty}</p>
{:else}
  <!-- why: native details/checkboxes -- a column chooser needs no library -->
  <details class="mb-1 text-[11px]" bind:open={colsOpen}>
    <summary class="cursor-pointer select-none text-muted-foreground hover:text-foreground">
      columns{#if hiddenCount}<span class="ml-2">· {hiddenCount} hidden</span>{/if}
    </summary>
    {#if hiddenCount}
      <label class="flex items-center gap-1 py-1"><input type="checkbox" bind:checked={showHidden} /> show hidden</label>
    {/if}
    <div class="flex flex-wrap gap-x-3 gap-y-1 py-1">
      {#if allySide}
        <label class="flex items-center gap-1"><input type="checkbox" checked={visible.has('class')} onchange={() => toggleCol('class')} /> class</label>
      {/if}
      {#each ALLY_COLS as c (c.key)}
        <label class="flex items-center gap-1"><input type="checkbox" checked={visible.has(c.key)} onchange={() => toggleCol(c.key)} /> {c.label}</label>
      {/each}
    </div>
  </details>
  <Table.Root>
    <Table.Header>
      <Table.Row>
        <Table.Head><button type="button" class="select-none" onclick={() => (sort = nextSort(sort, 'name', false))}>name{arrow('name', sort)}</button></Table.Head>
        {#if allySide && visible.has('class')}<Table.Head title="one class model for you and for them: a /who row is ground truth, otherwise evidence per encounter chain. Green once a class clears the bar, yellow while it is still a guess. A chain restarts when they leave, or you zone.">class</Table.Head>{/if}
        {#each shown as c (c.key)}
          <Table.Head class="text-right"><button type="button" class="select-none" onclick={() => (sort = nextSort(sort, c.key, true))}>{c.label}{arrow(c.key, sort)}</button></Table.Head>
        {/each}
      </Table.Row>
    </Table.Header>
    <Table.Body>
      {#each sorted as a (a.name)}
        <Table.Row
          class="cursor-pointer bg-no-repeat {a.hidden ? 'opacity-50' : ''}"
          style="background-image: linear-gradient(to right, color-mix(in srgb, var(--color-primary) 14%, transparent) {a.pct}%, transparent {a.pct}%)"
          onclick={() => toggleAlly(a.name)}
          oncontextmenu={(e) => void openMenu(e, a)}
        >
          <!-- why: a suggested ally (charm pet / co-occurrence, no permanent
               proof -- see AllyDto.suggested's own doc) reads visibly
               tentative, not equal to a proven groupmate; pet_total > 0
               notes how much of an owner's row came via their pet -->
          <Table.Cell class={a.suggested && allySide ? 'text-muted-foreground italic' : a.is_player || a.is_pet ? 'text-primary' : ''}>
            {a.name}{#if a.suggested && allySide}<span
                class="ml-1 rounded-sm border border-border px-1 text-[9px] not-italic text-muted-foreground"
                title="Suggested ally -- included via charm or repeated co-occurrence, not proven">suggested</span
              >{/if}{#if a.pet_total > 0 && a.pet_total < a.total}<span
                class="ml-1 text-[10px] text-muted-foreground"
                title="Damage contributed by this ally's pet">(pet {a.pet_total.toLocaleString()})</span
              >{/if}
          </Table.Cell>
          {#if allySide && visible.has('class')}
          <!-- why: a /who row from THIS presence confirms (green, with
               level); else inferred through combat -- green once a dozen
               votes back it, yellow with a "?" before. Both reset when
               the ally leaves or you zone. -->
          <Table.Cell class="font-mono text-[11px] tabular-nums {a.class_confirmed || a.class_evidence >= 12 ? 'text-good' : 'text-caution'}"
            title={a.class_source === 'who' ? `a /who row this presence (level ${a.level})` : a.classes.length ? `${a.class_source === 'self' ? 'your own' : 'their'} class detection -- ${a.class_evidence} encounter${a.class_evidence === 1 ? '' : 's'} of evidence in this chain${a.class_confirmed ? '' : ', still short of the bar'}` : 'no class evidence yet'}>
            {#if a.class_source !== 'who'}
              <!-- why: docs P9 -- a prior is dimmed, an open slot shows "?",
                   a running conflict adds " ?", a closed chain " ??" -->
              {#each a.classes as c, i (c)}{i ? '/' : ''}<span class={a.class_prior.includes(c) ? 'opacity-60' : ''} title={a.class_prior.includes(c) ? `${c}: carried as a prior, reconfirming` : c}>{abbr(c)}</span>{/each}{#if a.classes.length < 3}{a.classes.length ? '/' : ''}<span class="text-caution" title={a.class_candidates.length ? `open slot, between: ${a.class_candidates.join(', ')}` : 'open slot, no candidates yet'}>?</span>{/if}{#if a.class_chain_end === '??'}<span class="text-bad" title="chain closed by contradiction"> ??</span>{:else if a.class_conflicts}<span class="text-caution" title="{a.class_conflicts} conflicting encounter{a.class_conflicts === 1 ? '' : 's'} running"> ?</span>{/if}{a.level != null ? ` ${a.level}` : ''}
            {:else}
              {a.classes.map(abbr).join('/')}{a.class_confirmed ? (a.level != null ? ` ${a.level}` : '') : a.classes.length && a.class_evidence < 12 ? '?' : ''}
            {/if}
          </Table.Cell>
          {/if}
          {#each shown as c (c.key)}
            <Table.Cell class="text-right tabular-nums">{allyCell(a, c.key)}</Table.Cell>
          {/each}
        </Table.Row>
        {#if $expandedAlly === a.name && $allySummary}
          {#if a.pets.length}
            <!-- why: an owner with a pet is a folder -- two slices in the
                 row's own shape, the player alone and the pet while it
                 was theirs, each opening its own abilities -->
            {#each partsOf(a) as part (part.name)}
              <Table.Row class="cursor-pointer bg-muted/20" onclick={() => (expandedPart = expandedPart === part.name ? null : part.name)}>
                <Table.Cell class="pl-6 {part.pet ? 'text-muted-foreground' : ''}">
                  <span class="mr-1 text-muted-foreground">{expandedPart === part.name ? '▾' : '▸'}</span>{part.label}
                </Table.Cell>
                {#if allySide && visible.has('class')}
                  <Table.Cell class="text-[10px] text-muted-foreground">{part.pet ? 'charmed pet' : ''}</Table.Cell>
                {/if}
                {#each shown as c (c.key)}
                  <Table.Cell class="text-right tabular-nums">{partCell(part, a, c.key)}</Table.Cell>
                {/each}
              </Table.Row>
              {#if expandedPart === part.name && part.summary}
                <Table.Row>
                  <Table.Cell colspan={cols} class="bg-muted/40 p-0">
                    <div class="flex w-0 min-w-full flex-col gap-3 p-3 pl-6 whitespace-normal">
                      {@render entityBlock(rowsOf(part.summary), part.summary, null, null)}
                      {#if !part.pet}{@render ownerExtras(a)}{/if}
                    </div>
                  </Table.Cell>
                </Table.Row>
              {/if}
            {/each}
          {:else}
            <Table.Row>
              <Table.Cell colspan={cols} class="bg-muted/40 p-0">
                <div class="flex w-0 min-w-full flex-col gap-3 p-3 whitespace-normal">
                  {@render entityBlock(abilityRows, $allySummary, null, null)}
                  {@render ownerExtras(a)}
                </div>
              </Table.Cell>
            </Table.Row>
          {/if}
        {/if}
      {/each}
    </Table.Body>
  </Table.Root>
{/if}

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menuItems} onclose={() => (menu = null)} />
{/if}

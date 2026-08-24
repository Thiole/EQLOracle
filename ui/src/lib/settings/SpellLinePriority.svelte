<script lang="ts">
  import { Input } from '$lib/components/ui/input';
  import { Button } from '$lib/components/ui/button';
  import { spells } from '$lib/stores/gamedata';
  import {
    spellLineOverrides, spellLineCustomMembership, openSpellLineKey,
    moveUp, moveDown, resetLine, resetAllSpellLinePriorities, addSpellToLine, removeSpellFromLine,
  } from '$lib/stores/spellLinePriority';
  import { allSpellLines, membersOfLine, effectiveLineKey } from '$lib/character/spellSuggest';

  let search = $state('');
  let selectedKey = $state<string | null>(null);
  let addSearch = $state('');

  // why: a one-shot deep-link seed, not a permanent binding -- a later
  // in-page click on a different line must win, not get fought back to
  // whatever a cross-module link opened this page with (same shape
  // SpellbookBuilder.svelte's own armed/classesSeeded already use).
  $effect(() => {
    if ($openSpellLineKey) {
      selectedKey = $openSpellLineKey;
      openSpellLineKey.set(null);
    }
  });

  const lines = $derived(allSpellLines($spells, $spellLineCustomMembership));

  const filteredLines = $derived(
    (() => {
      const q = search.trim().toLowerCase();
      return q ? lines.filter((l) => l.label.toLowerCase().includes(q)) : lines;
    })(),
  );

  // why: search also matches an individual spell by name, not just an
  // already-formed 2+ line's label -- a brand new manual merge always
  // starts from at least one side with nothing to browse to otherwise
  // (Mesmerization has no natural line-mate at all until you add one).
  const soloMatches = $derived.by(() => {
    const q = search.trim().toLowerCase();
    if (!q) return [];
    const shownKeys = new Set(filteredLines.map((l) => l.key));
    return $spells
      .filter((s) => s.name.toLowerCase().includes(q) && !shownKeys.has(effectiveLineKey(s, $spellLineCustomMembership)))
      .slice(0, 15);
  });

  // why: recomputed straight from the catalog every time, not from
  // `lines` (which only carries 2+ member groups) -- a just-opened
  // singleton, or a line that just gained/lost a manual member, must
  // reflect its real current membership immediately.
  const lineMembers = $derived(selectedKey ? membersOfLine($spells, selectedKey, $spellLineCustomMembership) : []);
  // why: real correction -- name it by the highest-level member, not the
  // raw wiki description text (that read like log prose, not a name).
  const lineLabel = $derived(lineMembers[0]?.name ?? '');

  const effectiveOrder = $derived.by(() => {
    if (!selectedKey) return [];
    const override = $spellLineOverrides[selectedKey];
    if (!override) return lineMembers;
    // why: an override may be stale against a re-scraped catalog (a
    // member renamed/removed) -- known members in saved order first,
    // then any catalog member the override never heard of, appended
    // rather than dropped.
    const byName = new Map(lineMembers.map((s) => [s.name, s]));
    const ordered = override.map((n) => byName.get(n)).filter((s) => s != null);
    const missing = lineMembers.filter((s) => !override.includes(s.name));
    return [...ordered, ...missing];
  });

  const addResults = $derived.by(() => {
    const q = addSearch.trim().toLowerCase();
    if (!q || !selectedKey) return [];
    const already = new Set(lineMembers.map((s) => s.name));
    return $spells.filter((s) => s.name.toLowerCase().includes(q) && !already.has(s.name)).slice(0, 15);
  });

  function openSpell(s: { name: string }) {
    selectedKey = effectiveLineKey($spells.find((x) => x.name === s.name)!, $spellLineCustomMembership);
  }

  function targetLabel(t: string | null): string {
    if (!t) return '';
    if (t === 'PB AE' || t === 'Targeted AE' || t === 'Free Target AE') return 'AE';
    if (t === 'Group' || t === 'Group v1' || t === 'Group v2' || t === 'Party') return 'Group';
    return t;
  }
</script>

<h2 class="panel-title mb-1.5">spell line priority</h2>
<p class="mb-2 text-[11px] text-muted-foreground">
  The Spellbook's Suggest buttons pick the highest-level member of a spell line by default, which isn't always the
  best pick. Rank a line here and Suggest respects it instead. Lines are auto-detected from the wiki's own spell
  descriptions (accurate for real rank upgrades), but can't know which spells across *different* classes overwrite
  each other in the game itself (e.g. one class's Slow vs. another's) -- add those manually below.
</p>

<div class="mb-2 flex items-center justify-between gap-2">
  <Input bind:value={search} placeholder="search spell lines or any spell…" class="h-7 w-72 text-[12px]" />
  {#if Object.keys($spellLineOverrides).length || Object.keys($spellLineCustomMembership).length}
    <Button size="sm" variant="ghost" class="h-7 text-[11px] text-destructive" onclick={resetAllSpellLinePriorities}>
      reset all spell line priorities
    </Button>
  {/if}
</div>

<div class="grid grid-cols-2 gap-3">
  <div class="h-72 overflow-y-auto rounded-sm border border-border">
    {#each filteredLines as line (line.key)}
      <button
        type="button"
        class="block w-full border-b border-border/50 px-2 py-1 text-left text-[11px] leading-tight {selectedKey === line.key
          ? 'bg-primary/15 text-primary'
          : 'text-foreground hover:bg-accent'}"
        onclick={() => (selectedKey = line.key)}
      >
        {line.label}
        {#if $spellLineOverrides[line.key]}<span class="ml-1 text-[9px] text-muted-foreground">(ranked)</span>{/if}
      </button>
    {/each}
    {#each soloMatches as s (s.name)}
      <button
        type="button"
        class="block w-full border-b border-border/50 px-2 py-1 text-left text-[11px] leading-tight text-muted-foreground hover:bg-accent"
        onclick={() => openSpell(s)}
      >
        {s.name} <span class="text-[9px]">(no line yet)</span>
      </button>
    {/each}
    {#if !filteredLines.length && !soloMatches.length}
      <p class="p-2 text-[11px] text-muted-foreground">no matches</p>
    {/if}
  </div>

  <div class="h-72 overflow-y-auto rounded-sm border border-border p-2">
    {#if !selectedKey}
      <p class="text-[11px] text-muted-foreground">Pick a spell line on the left to rank its members.</p>
    {:else}
      <div class="mb-1.5 flex items-center justify-between gap-2">
        <p class="text-[11px] text-muted-foreground">{lineLabel}</p>
        {#if $spellLineOverrides[selectedKey]}
          <Button size="sm" variant="ghost" class="h-6 shrink-0 text-[10px] text-destructive" onclick={() => resetLine(selectedKey!)}>
            reset this line
          </Button>
        {/if}
      </div>
      {#each effectiveOrder as s, i (s.name)}
        <div class="mb-1 flex items-center gap-1.5 rounded-sm border border-border px-1.5 py-1 text-[11px]">
          <div class="flex flex-col">
            <button
              type="button"
              class="leading-none text-muted-foreground disabled:opacity-30 hover:text-primary"
              disabled={i === 0}
              onclick={() => moveUp($spells, selectedKey!, s.name)}
            >
              ▲
            </button>
            <button
              type="button"
              class="leading-none text-muted-foreground disabled:opacity-30 hover:text-primary"
              disabled={i === effectiveOrder.length - 1}
              onclick={() => moveDown($spells, selectedKey!, s.name)}
            >
              ▼
            </button>
          </div>
          <span class="w-4 shrink-0 text-right tabular-nums text-muted-foreground">{i + 1}</span>
          <span class="flex-1 truncate text-foreground">{s.name}</span>
          <span class="shrink-0 text-muted-foreground">
            {s.classes.map((c) => (c.level != null ? `${c.class} ${c.level}` : c.class)).join(', ') || '—'}
          </span>
          {#if targetLabel(s.target_type)}
            <span class="shrink-0 rounded-sm bg-muted px-1 text-[9px] text-muted-foreground">{targetLabel(s.target_type)}</span>
          {/if}
          {#if $spellLineCustomMembership[s.name] === selectedKey}
            <button
              type="button"
              class="shrink-0 text-muted-foreground hover:text-destructive"
              title="remove from this line"
              onclick={() => removeSpellFromLine(s.name)}
            >
              ✕
            </button>
          {/if}
        </div>
      {/each}

      <div class="mt-2 border-t border-border pt-2">
        <p class="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
          add a spell that overwrites this line (any class)
        </p>
        <Input bind:value={addSearch} placeholder="search spells…" class="h-7 text-[12px]" />
        {#if addResults.length}
          <div class="mt-1 max-h-24 overflow-y-auto rounded-sm border border-border">
            {#each addResults as s (s.name)}
              <button
                type="button"
                class="block w-full border-b border-border/50 px-1.5 py-0.5 text-left text-[11px] text-foreground hover:bg-accent"
                onclick={() => {
                  addSpellToLine(selectedKey!, s.name);
                  addSearch = '';
                }}
              >
                {s.name}
                <span class="text-muted-foreground">
                  ({s.classes.map((c) => c.class).join(', ') || '—'})
                </span>
              </button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>
</div>

<script lang="ts">
  import { Input } from '$lib/components/ui/input';
  import { Button } from '$lib/components/ui/button';
  import { spells } from '$lib/stores/gamedata';
  import {
    spellLineOverrides, openSpellLineKey, moveUp, moveDown, resetLine, resetAllSpellLinePriorities,
  } from '$lib/stores/spellLinePriority';
  import { allSpellLines } from '$lib/character/spellSuggest';

  let search = $state('');
  let selectedKey = $state<string | null>(null);

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

  const lines = $derived(allSpellLines($spells));

  const filteredLines = $derived(
    (() => {
      const q = search.trim().toLowerCase();
      return q ? lines.filter((l) => l.label.toLowerCase().includes(q)) : lines;
    })(),
  );

  const selectedLine = $derived(lines.find((l) => l.key === selectedKey) ?? null);

  const effectiveOrder = $derived.by(() => {
    if (!selectedLine) return [];
    const override = $spellLineOverrides[selectedLine.key];
    if (!override) return selectedLine.members;
    // why: an override may be stale against a re-scraped catalog (a
    // member renamed/removed) -- known members in saved order first,
    // then any catalog member the override never heard of, appended
    // rather than dropped.
    const byName = new Map(selectedLine.members.map((s) => [s.name, s]));
    const ordered = override.map((n) => byName.get(n)).filter((s) => s != null);
    const missing = selectedLine.members.filter((s) => !override.includes(s.name));
    return [...ordered, ...missing];
  });

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
  best pick (an AE mez can beat a later single-target one, for example). Rank a line here and Suggest respects it instead.
</p>

<div class="mb-2 flex items-center justify-between gap-2">
  <Input bind:value={search} placeholder="search spell lines…" class="h-7 w-64 text-[12px]" />
  {#if Object.keys($spellLineOverrides).length}
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
    {:else}
      <p class="p-2 text-[11px] text-muted-foreground">no matches</p>
    {/each}
  </div>

  <div class="h-72 overflow-y-auto rounded-sm border border-border p-2">
    {#if !selectedLine}
      <p class="text-[11px] text-muted-foreground">Pick a spell line on the left to rank its members.</p>
    {:else}
      <div class="mb-1.5 flex items-center justify-between gap-2">
        <p class="text-[11px] text-muted-foreground">{selectedLine.members.length} members, most-preferred first</p>
        {#if $spellLineOverrides[selectedLine.key]}
          <Button size="sm" variant="ghost" class="h-6 text-[10px] text-destructive" onclick={() => resetLine(selectedLine.key)}>
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
              onclick={() => moveUp($spells, selectedLine.key, s.name)}
            >
              ▲
            </button>
            <button
              type="button"
              class="leading-none text-muted-foreground disabled:opacity-30 hover:text-primary"
              disabled={i === effectiveOrder.length - 1}
              onclick={() => moveDown($spells, selectedLine.key, s.name)}
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
        </div>
      {/each}
    {/if}
  </div>
</div>

<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Checkbox } from '$lib/components/ui/checkbox';
  import { ICON_BASE, ALL_CLASSES, MAX_CHARACTER_LEVEL } from '$lib/character/constants';
  import { activeClasses, damageSpells } from '$lib/stores/character';
  import { usableClasses as levelUsableClasses, lineKey, simulateRotation, collapseSequence } from '$lib/character/spellSuggest';
  import { api, type DamageSpellDto } from '$lib/tauri/api';

  // why: three modes, matching exactly what was asked for -- DPM
  // (mana-limited fights), DPS assuming you actually wait out each
  // spell's own recast timer, and DPS ignoring recast (as if every
  // spell could be woven back-to-back with nothing else in the way).
  type Mode = 'dpm' | 'dps_reuse' | 'dps_ignore';
  let mode = $state<Mode>('dps_reuse');

  // why: eqlwiki "Stances & Invocations" page (fetched directly --
  // neither invocation's exact formula was already in any local pack).
  // Only one invocation is ever active at a time in-game; only the two
  // with a damage-relevant, fully-stated formula are modeled here --
  // Arcane Mastery (cast/recovery time + detrimental mana cost) and
  // Empower (flat damage % at a mana-cost premium). The others either
  // don't touch damage math (Recovery, Inviolable, Unyielding) or touch
  // something this calculator doesn't model at all (Over Channel's
  // resist adjust -- no resist-chance modeling here; Spellblade/
  // Inversion/Divine's endurance/proc mechanics).
  type Invocation = 'none' | 'arcane_mastery' | 'empower';
  let invocation = $state<Invocation>('none');
  const INTEL_CLASSES = ['Enchanter', 'Magician', 'Necromancer', 'Wizard'];
  const NON_HYBRID_CASTERS = ['Cleric', 'Druid', 'Enchanter', 'Magician', 'Necromancer', 'Shaman', 'Wizard'];

  let selectedClasses = $state<string[]>([]);
  let classesSeeded = false;
  $effect(() => {
    if (!classesSeeded && $activeClasses.length) {
      selectedClasses = [...$activeClasses];
      classesSeeded = true;
    }
  });
  function toggleClass(c: string) {
    selectedClasses = selectedClasses.includes(c) ? selectedClasses.filter((x) => x !== c) : [...selectedClasses, c];
  }

  // why: "assume rank 10" swaps the whole data source to a hypothetical
  // max-rank preview ("what's worth ranking up") instead of this
  // session's real observed ranks -- fetched separately since it's a
  // distinct dataset, not something the shared real-rank store carries.
  let assumeMaxRank = $state(false);
  let assumeMaxRankSpells = $state<DamageSpellDto[] | null>(null);
  let assumeMaxRankError = $state<string | null>(null);
  $effect(() => {
    if (!assumeMaxRank || assumeMaxRankSpells !== null) return;
    api
      .getDamageSpells(true)
      .then((s) => (assumeMaxRankSpells = s))
      .catch((e) => (assumeMaxRankError = e instanceof Error ? e.message : String(e)));
  });
  const spells = $derived(assumeMaxRank ? assumeMaxRankSpells : $damageSpells);

  // why: usable = at least one class entry (scoped to the selected
  // classes, if any are chosen) at or below the level cap -- the exact
  // same rule the "Suggested spells" picker above already uses, so what
  // shows up here never quietly disagrees with what shows up there.
  function usableClasses(s: DamageSpellDto) {
    const pool = levelUsableClasses(s.classes);
    return selectedClasses.length ? pool.filter((c) => selectedClasses.includes(c.class)) : pool;
  }

  function metricOf(s: DamageSpellDto): number {
    return mode === 'dpm' ? s.dpm : mode === 'dps_reuse' ? s.dps_with_reuse : s.dps_ignoring_reuse;
  }
  function metricLabel(): string {
    return mode === 'dpm' ? 'DPM' : mode === 'dps_reuse' ? 'DPS' : 'DPS (no reuse)';
  }

  // why: "additional" in the wiki's own wording -- the base percentage
  // already covers having one qualifying class, each other one in the
  // trio adds the increment. Counted against the player's own chosen
  // trio (`selectedClasses`), not per-spell -- an invocation is a
  // player-wide choice, not a per-cast one.
  function countInSet(set: string[]): number {
    return selectedClasses.filter((c) => set.includes(c)).length;
  }

  function applyInvocation(s: DamageSpellDto): DamageSpellDto {
    let { total_damage, instant_damage, mana, casting_time, recast_time } = s;
    if (invocation === 'arcane_mastery') {
      const n = countInSet(INTEL_CLASSES);
      if (n > 0) {
        const timeCut = 0.2 + 0.1 * (n - 1);
        const manaCut = 0.1 + 0.05 * (n - 1);
        casting_time *= 1 - timeCut;
        recast_time *= 1 - timeCut;
        mana *= 1 - manaCut;
      }
    } else if (invocation === 'empower') {
      const n = countInSet(NON_HYBRID_CASTERS);
      if (n > 0) {
        // why: applied to both total_damage and instant_damage
        // proportionally -- the wiki's own wording is "adds N% to
        // Damage effects" generally, not scoped to only the DoT tick or
        // only an instant hit, so a DoT's own "on cast" burst (if any)
        // gets boosted the same rate its tick stream does.
        const mult = 1 + (0.2 + 0.1 * (n - 1));
        total_damage *= mult;
        instant_damage *= mult;
        mana *= 1.2;
      }
    }
    casting_time = Math.max(casting_time, 0.1);
    mana = Math.max(mana, 1);
    const cycle = s.duration_secs != null ? Math.max(s.duration_secs, casting_time + recast_time) : casting_time + recast_time;
    return {
      ...s,
      total_damage,
      instant_damage,
      mana,
      casting_time,
      recast_time,
      dpm: total_damage / mana,
      dps_with_reuse: total_damage / cycle,
      // why: same rule as the Rust side -- a DoT's "no reuse" rate is
      // only ever its instant component, never the whole tick-stream
      // total (that's `dps_with_reuse`'s job). See `instant_damage`'s
      // own doc in api.ts.
      dps_ignoring_reuse: instant_damage / casting_time,
    };
  }

  const candidates = $derived.by(() =>
    (spells ?? []).filter((s) => usableClasses(s).length > 0).map(applyInvocation),
  );

  // why: within a spell line, keep only the highest-level member the
  // player can actually cast -- a known lower-tier duplicate is never a
  // real suggestion once a higher one is known.
  const deduped = $derived.by(() => {
    const groups = new Map<string, DamageSpellDto>();
    for (const s of candidates) {
      const key = lineKey(s.name);
      const level = Math.max(0, ...usableClasses(s).map((c) => c.level ?? 0));
      const existing = groups.get(key);
      if (!existing || level > Math.max(0, ...usableClasses(existing).map((c) => c.level ?? 0))) {
        groups.set(key, s);
      }
    }
    return [...groups.values()];
  });

  // why: real damage-over-a-window simulation, generalizing the old
  // single best-nuke-vs-worthwhile-DoT and 2-nuke weave-pair heuristics
  // into an actual N-spell timeline -- see simulateRotation's own doc
  // in spellSuggest.ts for the algorithm.
  let rotationWindow = $state<15 | 60>(60);
  const rotationResult = $derived(simulateRotation(deduped, rotationWindow));
  const rotationChips = $derived(collapseSequence(rotationResult.sequence));

  const ranked = $derived.by(() => [...deduped].sort((a, b) => metricOf(b) - metricOf(a)).slice(0, 30));

  function fmt(n: number): string {
    return n.toLocaleString(undefined, { maximumFractionDigits: 1 });
  }

  // why: collapsed by default -- a decent-sized window should show every
  // section's header without scrolling; the calculator itself opens on request.
  let open = $state(false);
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <button type="button" class="flex w-full items-center gap-1.5 text-left" onclick={() => (open = !open)}>
      <span class="w-6 text-[26px] leading-none font-bold text-foreground">{open ? '▾' : '▸'}</span>
      <h2 class="panel-title">DPS auto-suggest</h2>
    </button>
    {#if open}
    <p class="mb-2 mt-1.5 text-[11px] text-muted-foreground">
      Damage/mana math for every damage spell you can currently cast (level {MAX_CHARACTER_LEVEL} cap, same as the picker above).
      Nuke damage is rank-adjusted at +6% per live rank level (I-X), compounding -- the wiki upgrade guide's own rate.
      A DoT's own <i>per-tick</i> damage doesn't scale with rank; only its one-time "on cast" hit (if any) does, though its cast
      time/mana/duration still shrink or grow with rank (wiki-sourced estimate, unverified). A DoT already ticks on its own once
      cast, so its "DPS (no reuse)" column only ever reflects that one-time hit (0 for most real DoTs) -- its real sustained rate
      is the plain "DPS" column instead, which is what the rotation below actually weighs it against.
    </p>

    <div class="mb-2 flex flex-wrap items-start gap-x-3 gap-y-1.5">
      <div class="flex overflow-hidden rounded-sm border border-border text-[10px]">
        {#each [['dpm', 'DPM'], ['dps_reuse', 'DPS'], ['dps_ignore', 'DPS (no reuse)']] as [v, label] (v)}
          <button
            type="button"
            class="px-2 py-0.5 {mode === v ? 'bg-primary/15 text-primary' : 'text-muted-foreground hover:bg-accent'}"
            onclick={() => (mode = v as Mode)}
          >
            {label}
          </button>
        {/each}
      </div>
      <div class="flex overflow-hidden rounded-sm border border-border text-[10px]" title="Only one invocation is ever active in-game -- pick which one to model">
        {#each [['none', 'No invocation'], ['arcane_mastery', 'Arcane Mastery'], ['empower', 'Empower']] as [v, label] (v)}
          <button
            type="button"
            class="px-2 py-0.5 {invocation === v ? 'bg-primary/15 text-primary' : 'text-muted-foreground hover:bg-accent'}"
            onclick={() => (invocation = v as Invocation)}
          >
            {label}
          </button>
        {/each}
      </div>
      <div class="grid grid-flow-col grid-rows-4 gap-x-1.5 gap-y-1">
        {#each ALL_CLASSES as c (c)}
          <button
            type="button"
            class="rounded-sm border px-1.5 py-0.5 text-[10px] {selectedClasses.includes(c)
              ? 'border-primary/40 bg-primary/10 text-primary'
              : 'border-border text-muted-foreground hover:border-primary hover:text-primary'}"
            onclick={() => toggleClass(c)}
          >
            {c}
          </button>
        {/each}
      </div>
      <label class="flex items-center gap-1.5 text-[11px] text-muted-foreground">
        <Checkbox checked={assumeMaxRank} onCheckedChange={(v: boolean) => (assumeMaxRank = v)} />
        assume rank 10
      </label>
    </div>
    {#if invocation === 'arcane_mastery'}
      {@const n = countInSet(INTEL_CLASSES)}
      <p class="mb-2 text-[11px] text-muted-foreground">
        Arcane Mastery: -{n > 0 ? 20 + 10 * (n - 1) : 0}% cast/recovery time, -{n > 0 ? 10 + 5 * (n - 1) : 0}% detrimental mana cost
        {#if n === 0}
          <span class="text-caution">— none of your selected classes qualify (Enchanter/Magician/Necromancer/Wizard), so this has no effect</span>
        {/if}
      </p>
    {:else if invocation === 'empower'}
      {@const n = countInSet(NON_HYBRID_CASTERS)}
      <p class="mb-2 text-[11px] text-muted-foreground">
        Empower: +{n > 0 ? 20 + 10 * (n - 1) : 0}% damage at +{n > 0 ? 20 : 0}% mana cost
        {#if n === 0}
          <span class="text-caution">— none of your selected classes qualify, so this has no effect</span>
        {/if}
      </p>
    {/if}

    {#if assumeMaxRank && assumeMaxRankError}
      <p class="text-[12px] text-destructive">{assumeMaxRankError}</p>
    {:else if !spells}
      <p class="text-[12px] text-muted-foreground">Loading…</p>
    {:else if !deduped.length}
      <p class="text-[11px] text-muted-foreground">No usable damage spells found for the selected class(es) yet.</p>
    {:else}
      <div class="mb-3">
        <div class="mb-1 flex items-center justify-between gap-2">
          <h3 class="text-[10px] uppercase tracking-wide text-muted-foreground">Suggested rotation</h3>
          <div class="flex overflow-hidden rounded-sm border border-border text-[10px]">
            {#each [15, 60] as w (w)}
              <button
                type="button"
                class="px-2 py-0.5 {rotationWindow === w ? 'bg-primary/15 text-primary' : 'text-muted-foreground hover:bg-accent'}"
                onclick={() => (rotationWindow = w as 15 | 60)}
              >
                {w}s
              </button>
            {/each}
          </div>
        </div>
        {#if rotationChips.length}
          <div class="flex flex-wrap items-center gap-1 text-[11px]">
            {#each rotationChips as chip, i (i)}
              {#if i > 0}<span class="text-muted-foreground">→</span>{/if}
              <div class="flex items-center gap-1.5 rounded-sm border border-primary/40 bg-primary/10 px-2 py-1" title={chip.spell.name}>
                {#if chip.spell.icon}
                  <img src={ICON_BASE + encodeURIComponent(chip.spell.icon)} alt="" class="size-4 shrink-0 rounded-[2px] border border-border bg-muted/20" />
                {/if}
                <span class="text-foreground">{chip.spell.name}{#if chip.count > 1}<span class="text-muted-foreground"> ×{chip.count}</span>{/if}</span>
                {#if chip.spell.is_dot}<span class="rounded-sm bg-muted px-1 text-[9px] text-muted-foreground">DoT</span>{/if}
              </div>
            {/each}
          </div>
          <p class="mt-1.5 text-[11px] text-muted-foreground">
            {fmt(rotationResult.totalDamage)} total damage over {rotationWindow}s -- {fmt(rotationResult.avgDps)} average DPS.
          </p>
        {:else}
          <p class="text-[11px] text-muted-foreground">Nothing fits in this window.</p>
        {/if}
      </div>

      <h3 class="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">Ranked by {metricLabel()}</h3>
      <div class="max-h-72 overflow-y-auto rounded-sm border border-border">
        <table class="w-full text-[11px]">
          <thead class="sticky top-0 bg-card">
            <tr class="border-b border-border text-muted-foreground">
              <th class="px-2 py-0.5 text-left font-normal">Spell</th>
              <th class="px-2 py-0.5 text-right font-normal">Rank</th>
              <th class="px-2 py-0.5 text-right font-normal">Dmg</th>
              <th class="px-2 py-0.5 text-right font-normal">Mana</th>
              <th class="px-2 py-0.5 text-right font-normal">DPM</th>
              <th class="px-2 py-0.5 text-right font-normal">DPS</th>
              <th class="px-2 py-0.5 text-right font-normal">DPS (no reuse)</th>
            </tr>
          </thead>
          <tbody>
            {#each ranked as s (s.name)}
              <tr class="border-b border-border/50">
                <td class="px-2 py-0.5">
                  {s.name}{#if s.is_dot}<span class="ml-1 rounded-sm bg-muted px-1 text-[9px] text-muted-foreground">DoT</span>{/if}
                </td>
                <td class="px-2 py-0.5 text-right tabular-nums text-muted-foreground">{s.rank}</td>
                <td class="px-2 py-0.5 text-right tabular-nums">{fmt(s.total_damage)}</td>
                <td class="px-2 py-0.5 text-right tabular-nums">{fmt(s.mana)}</td>
                <td class="px-2 py-0.5 text-right tabular-nums">{fmt(s.dpm)}</td>
                <td class="px-2 py-0.5 text-right tabular-nums">{fmt(s.dps_with_reuse)}</td>
                <td class="px-2 py-0.5 text-right tabular-nums">{fmt(s.dps_ignoring_reuse)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
    {/if}
  </CardContent>
</Card>

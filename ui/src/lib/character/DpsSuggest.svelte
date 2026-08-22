<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { ICON_BASE, ALL_CLASSES, MAX_CHARACTER_LEVEL } from '$lib/character/constants';
  import { activeClasses } from '$lib/stores/character';
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

  let spells = $state<DamageSpellDto[] | null>(null);
  let loadError = $state<string | null>(null);
  $effect(() => {
    api
      .getDamageSpells()
      .then((s) => (spells = s))
      .catch((e) => (loadError = e instanceof Error ? e.message : String(e)));
  });

  // why: usable = at least one class entry (scoped to the selected
  // classes, if any are chosen) at or below the level cap -- the exact
  // same rule the "Suggested spells" picker above already uses, so what
  // shows up here never quietly disagrees with what shows up there.
  function usableClasses(s: DamageSpellDto) {
    const pool = s.classes.filter((c) => c.level == null || c.level <= MAX_CHARACTER_LEVEL);
    return selectedClasses.length ? pool.filter((c) => selectedClasses.includes(c.class)) : pool;
  }

  // why: groups same spell-line variants ("System Shock III"/"IV"/"V")
  // under one key so a rotation never doubles up on two tiers of the
  // same effect -- a pure grouping key, not a rank claim (unlike the
  // reverted spellRank.ts attempt, the stripped numeral is never shown
  // or treated as "this spell's rank", only used to notice two names
  // belong to the same family).
  function lineKey(name: string): string {
    const parts = name.split(' ');
    const tail = parts[parts.length - 1];
    return tail.length > 0 && /^[IVXLCDM]+$/.test(tail) ? parts.slice(0, -1).join(' ') : name;
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
    let { total_damage, mana, casting_time, recast_time } = s;
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
        total_damage *= 1 + (0.2 + 0.1 * (n - 1));
        mana *= 1.2;
      }
    }
    casting_time = Math.max(casting_time, 0.1);
    mana = Math.max(mana, 1);
    const cycle = s.duration_secs != null ? Math.max(s.duration_secs, casting_time + recast_time) : casting_time + recast_time;
    return {
      ...s,
      total_damage,
      mana,
      casting_time,
      recast_time,
      dpm: total_damage / mana,
      dps_with_reuse: total_damage / cycle,
      dps_ignoring_reuse: total_damage / casting_time,
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

  // why: the actual auto-suggest -- best nuke by the chosen metric, plus
  // any DoT whose own upkeep efficiency (damage per second of *casting
  // time* spent, `dps_ignoring_reuse`) beats what that same casting
  // time would earn spamming the best nuke instead. That's the real
  // "worth maintaining" test: refreshing a DoT early wastes duration
  // ("reapplying before duration runs out hurts DPS"), but a DoT that
  // clears this bar is still a strictly better use of that button-press
  // than nuking would have been, reuse timers aside. Capped at 3 DoTs +
  // 1 nuke so this stays a short rotation, not the whole spellbook.
  const rotation = $derived.by(() => {
    const nukes = deduped.filter((s) => !s.is_dot);
    const dots = deduped.filter((s) => s.is_dot);
    const bestNuke = nukes.length ? nukes.reduce((a, b) => (metricOf(b) > metricOf(a) ? b : a)) : null;
    const threshold = bestNuke?.dps_ignoring_reuse ?? 0;
    const worthwhileDots = dots
      .filter((d) => d.dps_ignoring_reuse > threshold)
      .sort((a, b) => b.dps_ignoring_reuse - a.dps_ignoring_reuse)
      .slice(0, 3);
    return bestNuke ? [...worthwhileDots, bestNuke] : worthwhileDots;
  });

  // why: a single nuke's own "DPS (no reuse)" is a fiction -- something
  // has to fill the gap while it's on cooldown. Weaving with a *second*
  // real nuke is how that's actually achieved: cast A, then immediately
  // B while A is on its own recast timer, then back to A -- as long as
  // each spell's own recast finishes before its next turn comes back
  // around, neither one is ever waited on. Cycle length is the larger of
  // (total cast time for one full lap) and (the slowest single spell's
  // own recast, which sets a hard floor no amount of weaving can beat).
  // Checked against the exact case asked about: this character's own
  // Conflagration + Ice Comet, recast 1.5s each vs. 5s cast times each
  // -- recast is never the bottleneck here, so the pair's sustained DPS
  // really does land close to summing their two "no reuse" rates.
  function pairCycleDps(a: DamageSpellDto, b: DamageSpellDto): number {
    const cycle = Math.max(a.casting_time + b.casting_time, a.recast_time, b.recast_time);
    return (a.total_damage + b.total_damage) / cycle;
  }

  const bestWeavePair = $derived.by(() => {
    const nukes = deduped.filter((s) => !s.is_dot);
    if (nukes.length < 2) return null;
    // why: capped to a small top-N pool by solo no-reuse rate -- an
    // O(n^2) pair search over the full ~80-candidate list is wasted
    // work when the best pair is always going to be drawn from the
    // strongest individual spells anyway.
    const pool = [...nukes].sort((a, b) => b.dps_ignoring_reuse - a.dps_ignoring_reuse).slice(0, 8);
    let best: { a: DamageSpellDto; b: DamageSpellDto; dps: number } | null = null;
    for (let i = 0; i < pool.length; i++) {
      for (let j = i + 1; j < pool.length; j++) {
        const dps = pairCycleDps(pool[i], pool[j]);
        if (!best || dps > best.dps) best = { a: pool[i], b: pool[j], dps };
      }
    }
    return best;
  });

  const ranked = $derived.by(() => [...deduped].sort((a, b) => metricOf(b) - metricOf(a)).slice(0, 30));

  function fmt(n: number): string {
    return n.toLocaleString(undefined, { maximumFractionDigits: 1 });
  }
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h2 class="panel-title mb-1.5">DPS auto-suggest</h2>
    <p class="mb-2 text-[11px] text-muted-foreground">
      Damage/mana math for every damage spell you can currently cast (level {MAX_CHARACTER_LEVEL} cap, same as the picker above).
      Nuke damage is rank-adjusted at +10% of base per live rank tier -- measured against your own log, not the wiki's guide page.
      A DoT's own <i>per-tick</i> damage doesn't scale with rank; only its one-time "on cast" hit (if any) does, though its cast
      time/mana/duration still shrink or grow with rank (wiki-sourced estimate, unverified).
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

    {#if loadError}
      <p class="text-[12px] text-destructive">{loadError}</p>
    {:else if !spells}
      <p class="text-[12px] text-muted-foreground">Loading…</p>
    {:else if !rotation.length}
      <p class="text-[11px] text-muted-foreground">No usable damage spells found for the selected class(es) yet.</p>
    {:else}
      <div class="mb-3">
        <h3 class="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">Suggested rotation</h3>
        <div class="flex flex-wrap gap-1.5">
          {#each rotation as s (s.name)}
            <div class="flex items-center gap-1.5 rounded-sm border border-primary/40 bg-primary/10 px-2 py-1 text-[11px]" title={s.name}>
              {#if s.icon}
                <img src={ICON_BASE + encodeURIComponent(s.icon)} alt="" class="size-4 shrink-0 rounded-[2px] border border-border bg-muted/20" />
              {/if}
              <span class="text-foreground">{s.name}</span>
              {#if s.is_dot}<span class="rounded-sm bg-muted px-1 text-[9px] text-muted-foreground">DoT</span>{/if}
              <span class="tabular-nums text-muted-foreground">{fmt(metricOf(s))} {metricLabel()}</span>
            </div>
          {/each}
        </div>
      </div>

      {#if bestWeavePair}
        <div class="mb-3">
          <h3 class="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">Best 2-nuke weave (hides each other's recast)</h3>
          <div class="flex flex-wrap items-center gap-1.5 text-[11px]">
            <div class="flex items-center gap-1.5 rounded-sm border border-border px-2 py-1" title={bestWeavePair.a.name}>
              {#if bestWeavePair.a.icon}<img src={ICON_BASE + encodeURIComponent(bestWeavePair.a.icon)} alt="" class="size-4 shrink-0 rounded-[2px] border border-border bg-muted/20" />{/if}
              <span>{bestWeavePair.a.name}</span>
            </div>
            <span class="text-muted-foreground">+</span>
            <div class="flex items-center gap-1.5 rounded-sm border border-border px-2 py-1" title={bestWeavePair.b.name}>
              {#if bestWeavePair.b.icon}<img src={ICON_BASE + encodeURIComponent(bestWeavePair.b.icon)} alt="" class="size-4 shrink-0 rounded-[2px] border border-border bg-muted/20" />{/if}
              <span>{bestWeavePair.b.name}</span>
            </div>
            <span class="tabular-nums text-muted-foreground">
              {fmt(bestWeavePair.dps)} DPS alternating -- vs {fmt(Math.max(bestWeavePair.a.dps_with_reuse, bestWeavePair.b.dps_with_reuse))} DPS spamming
              either one alone
            </span>
          </div>
        </div>
      {/if}

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
  </CardContent>
</Card>

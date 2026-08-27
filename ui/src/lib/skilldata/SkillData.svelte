<script lang="ts">
  // why: skill estimation table -- AA/focus effect on reuse, duration,
  // and damage, so applying a debuff shows how much it'll do or how
  // long it'll last.
  //
  // No new AA/focus modeling -- no real data source for that yet, and a
  // fabricated table of plausible numbers would be worse than none
  // (trust the log, not a guess). This tab surfaces data that already
  // accounts for AAs/focus/gear without modeling them individually:
  // skilltracker.rs's reuse/recovery timers are learned empirically off
  // real casts (smallest gap observed), not a hardcoded wiki number --
  // a haste item or reuse AA already shows up as a shorter learned
  // timer once used twice. Joined against two other real sources: known
  // duration (Spellbook, wiki-scraped) and known damage/dps (DPS
  // Suggest's rank-adjusted numbers) -- nothing invented.
  import { onMount } from 'svelte';
  import { api, type SkillStatusDto, type SpellbookEntryDto, type DamageSpellDto } from '$lib/tauri/api';
  import { ICON_BASE } from '$lib/character/constants';
  import { fmtTtk } from '$lib/format';
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Input } from '$lib/components/ui/input';
  import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '$lib/components/ui/table';

  let skills = $state<SkillStatusDto[]>([]);
  let spellbook = $state<SpellbookEntryDto[]>([]);
  let damageSpells = $state<DamageSpellDto[]>([]);
  let loaded = $state(false);
  let query = $state('');

  onMount(() => {
    void refresh();
  });

  async function refresh() {
    const [s, sb, ds] = await Promise.all([api.getSkillStatus(), api.getSpellbook(), api.getDamageSpells(true)]);
    skills = s ?? [];
    spellbook = sb ?? [];
    damageSpells = ds ?? [];
    loaded = true;
  }

  type Row = {
    skill: string;
    lastOutcome: 'landed' | 'avoided';
    lastUsedMs: number;
    reuseGapMs: number | null;
    recoveryGapMs: number | null;
    duration: string | null;
    icon: string | null;
    isDot: boolean;
    totalDamage: number | null;
    dpsWithReuse: number | null;
  };

  const rows = $derived.by((): Row[] => {
    const bySpellbookName = new Map(spellbook.map((s) => [s.name, s]));
    const byDamageName = new Map(damageSpells.map((d) => [d.name, d]));
    return skills
      .map((s) => {
        const sb = bySpellbookName.get(s.skill);
        const dmg = byDamageName.get(s.skill);
        return {
          skill: s.skill,
          lastOutcome: s.last_outcome,
          lastUsedMs: s.last_used_ms,
          reuseGapMs: s.reuse_gap_ms,
          recoveryGapMs: s.recovery_gap_ms,
          duration: sb?.duration ?? null,
          icon: sb?.icon ?? dmg?.icon ?? null,
          isDot: dmg?.is_dot ?? false,
          totalDamage: dmg?.total_damage ?? null,
          dpsWithReuse: dmg?.dps_with_reuse ?? null,
        };
      })
      .sort((a, b) => a.skill.localeCompare(b.skill));
  });

  const filtered = $derived(
    query.trim() ? rows.filter((r) => r.skill.toLowerCase().includes(query.trim().toLowerCase())) : rows,
  );
</script>

<div class="flex flex-col gap-3 p-3">
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h2 class="panel-title mb-1.5">skill data</h2>
      <p class="text-[11px] text-muted-foreground">
        Learned reuse/recovery timers for every ability or spell you've actually used this session, plus known
        duration and damage where the app already has real data for it. The timers are measured off your own real
        casts, not a static table -- an AA, a haste item, or a spell rank upgrade already shows up here as a shorter
        learned timer the moment it's been used twice, automatically.
      </p>
      <Input class="mt-2 max-w-64" placeholder="Filter by name…" bind:value={query} />
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="overflow-x-auto px-3 py-2.5">
      {#if !loaded}
        <p class="text-[11px] text-muted-foreground">Loading…</p>
      {:else if !filtered.length}
        <p class="text-[11px] text-muted-foreground">
          {rows.length ? 'Nothing matches that filter.' : "Nothing tracked yet -- use an ability or cast a spell and it'll show up here."}
        </p>
      {:else}
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>skill</TableHead>
              <TableHead>last outcome</TableHead>
              <TableHead>reuse (learned)</TableHead>
              <TableHead>recovery (learned)</TableHead>
              <TableHead>duration</TableHead>
              <TableHead>damage</TableHead>
              <TableHead>dps</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {#each filtered as r (r.skill)}
              <TableRow>
                <TableCell class="flex items-center gap-1.5 font-medium">
                  {#if r.icon}
                    <img src={ICON_BASE + encodeURIComponent(r.icon)} alt="" class="size-4 shrink-0 rounded-[2px]" />
                  {/if}
                  {r.skill}
                </TableCell>
                <TableCell class={r.lastOutcome === 'landed' ? 'text-good' : 'text-caution'}>{r.lastOutcome}</TableCell>
                <TableCell class="font-mono tabular-nums">{r.reuseGapMs !== null ? fmtTtk(r.reuseGapMs) : '—'}</TableCell>
                <TableCell class="font-mono tabular-nums">{r.recoveryGapMs !== null ? fmtTtk(r.recoveryGapMs) : '—'}</TableCell>
                <TableCell>{r.duration ?? '—'}</TableCell>
                <TableCell class="font-mono tabular-nums">
                  {r.totalDamage !== null ? `${Math.round(r.totalDamage)}${r.isDot ? ' (full DoT)' : ''}` : '—'}
                </TableCell>
                <TableCell class="font-mono tabular-nums">{r.dpsWithReuse !== null ? r.dpsWithReuse.toFixed(1) : '—'}</TableCell>
              </TableRow>
            {/each}
          </TableBody>
        </Table>
      {/if}
    </CardContent>
  </Card>
</div>

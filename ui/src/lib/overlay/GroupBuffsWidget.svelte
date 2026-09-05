<script lang="ts">
  // why: Group Buff Tracker -- a verdict first ("All good", else the
  // NAMES of what is missing), then one line per buff kind you could
  // have on you. Only problems carry colour: a covered buff is the
  // expected state and does not need to shout, asked for directly
  // ("dont show green if its good"). Sources include your own detected
  // classes, so a buff you can cast on yourself still reads as missing.
  import type { GroupBuffsDto } from '$lib/tauri/api';
  let {
    data,
    inCombat = false,
    opacity,
    overallOpacity,
  }: { data: GroupBuffsDto | null; inCombat?: boolean; opacity: number; overallOpacity: number } = $props();
  const ABBR: Record<string, string> = {
    Warrior: 'WAR', Cleric: 'CLR', Paladin: 'PAL', Ranger: 'RNG', 'Shadow Knight': 'SHD', Druid: 'DRU',
    Monk: 'MNK', Bard: 'BRD', Rogue: 'ROG', Shaman: 'SHM', Necromancer: 'NEC', Wizard: 'WIZ',
    Magician: 'MAG', Enchanter: 'ENC', Beastlord: 'BST', Berserker: 'BER',
  };
  const abbr = (c: string) => ABBR[c] ?? c.slice(0, 3).toUpperCase();
  const missingRows = $derived(data ? data.rows.filter((r) => !r.active) : []);
  const missing = $derived(missingRows.length);
  // why: name them -- "missing 2" makes you go looking, "missing Clarity,
  // Haste" is the answer itself
  const missingNames = $derived(missingRows.map((r) => r.label).join(', '));
  // why: an innate you can cast and have not is missing the same way a
  // party buff is; a MAYBE never counts against you, which is what makes
  // it a maybe
  const missingInnates = $derived(data ? data.innates.filter((i) => !i.active) : []);
  const maybes = $derived(data ? data.maybes.filter((m) => !m.active) : []);
  // why: a low-tier buff is not coverage -- see BuffRowDto.upgrade
  const upgrades = $derived(data ? data.rows.filter((r) => r.upgrade).length : 0);
  const clean = $derived(missing === 0 && upgrades === 0 && missingInnates.length === 0);
</script>

<div
  class="flex h-full w-full flex-col gap-1 overflow-hidden rounded-sm border border-border/60 px-2 py-1.5 text-[11px]"
  style:background="rgba(20, 24, 30, {opacity})"
  style:opacity={overallOpacity}
  style:text-shadow="0 1px 2px rgba(0, 0, 0, 0.9), 0 0px 4px rgba(0, 0, 0, 0.6)"
>
  {#if inCombat}
    <!-- why: nothing at all mid-fight. Buffs are what you fix BETWEEN
         pulls; a checklist you cannot act on is just occlusion. -->
  {:else if !data}
    <p class="text-muted-foreground">group buffs…</p>
  {:else if !data.rows.length && !data.party.length}
    <p class="text-muted-foreground">group buffs: no party</p>
  {:else}
    <div class="flex items-baseline justify-between">
      <span class="font-medium {clean ? 'text-foreground/70' : 'text-caution'}">
        <!-- why: no rows means nothing is KNOWN, which is not the same as
             nothing being wrong -- saying "All good" there would be a
             claim the data does not support -->
        group buffs: {!data.rows.length && !data.innates.length
          ? 'nothing confirmed yet'
          : clean
            ? 'All good'
            : [
                missing ? `missing ${missingNames}` : '',
                missingInnates.length ? `${missingInnates.length} innate${missingInnates.length === 1 ? '' : 's'}` : '',
                upgrades ? `${upgrades} upgradeable` : '',
              ].filter(Boolean).join(', ')}
      </span>
      <span class="truncate font-mono text-[10px] text-foreground/60" title="your classes">{data.my_classes.map(abbr).join('/')}</span>
    </div>
    {#if missing || upgrades || missingInnates.length || maybes.length}
    <div class="truncate font-mono text-[10px] text-foreground/60" title="party -- confirmed classes count; ? means not confirmed yet">
      {#each data.party as m, i (m.name)}{i ? ' · ' : ''}<span title={m.buffs.length ? `on ${m.name}: ${m.buffs.join(', ')}` : `nothing seen landing on ${m.name}`}>{m.name} {m.classes.length ? m.classes.map(abbr).join('/') : '?'}{m.confirmed ? '' : '?'}{m.buffs.length ? ` +${m.buffs.length}` : ''}</span>{/each}
    </div>
    <div class="flex flex-col gap-0.5">
      {#each data.rows.filter((r) => !r.active || r.upgrade) as r (r.kind)}
        <div class="flex items-baseline justify-between gap-2">
          <span class="text-foreground/80">{r.label}</span>
          {#if r.active && !r.upgrade}
            <span class="truncate text-foreground/60" title="on you">{r.active}</span>
          {:else if r.active}
            <!-- why: up, but a better rank is castable -- name it -->
            <span
              class="truncate text-caution"
              title={`${r.active} is on you; the party can cast ${r.lines[0]?.best_spell ?? 'better'} (${r.lines[0]?.casters.join(', ') ?? ''})`}
            >{r.active} &rarr; {r.lines[0]?.line ?? ''}</span>
          {:else}
            <!-- why: name the lines assumed missing, not just the kind --
                 ranks of a line are one entry, best rank first -->
            <span
              class="truncate text-caution"
              title={r.lines.map((l) => `${l.best_spell} -- ${l.casters.join(', ')}`).join('\n')}
            >{r.lines.map((l) => l.line).join(', ') || 'nobody can cast it'}</span>
          {/if}
        </div>
      {/each}
    </div>
    <!-- why: your own self-casts as a checklist -- no upgrade arrow and no
         caster, "a list of make sure these are on as innates" -->
    {#if missingInnates.length}
      <div class="flex flex-col gap-0.5 border-t border-foreground/15 pt-1">
        <div class="text-[10px] text-muted-foreground">innates</div>
        {#each missingInnates as i (i.line)}
          <div class="flex items-baseline justify-between gap-2 text-[10px]">
            <span class="truncate text-foreground/80">{i.line}</span>
            <span class="truncate text-caution" title="cast it on yourself">{i.best_spell}</span>
          </div>
        {/each}
      </div>
    {/if}
    <!-- why: illusions are a MAYBE -- real stats, but a suggestion, so
         they are quieter and never counted against you -->
    {#if maybes.length}
      <div class="flex flex-col gap-0.5 border-t border-foreground/15 pt-1">
        <div class="text-[10px] text-muted-foreground">maybe</div>
        {#each maybes as m (m.line + m.best_spell)}
          <div class="flex items-baseline justify-between gap-2 text-[10px] text-foreground/55">
            <span class="truncate">{m.line}</span>
            <span class="truncate">{m.best_spell}</span>
          </div>
        {/each}
      </div>
    {/if}
    {/if}
  {/if}
</div>

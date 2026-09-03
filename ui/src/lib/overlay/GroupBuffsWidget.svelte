<script lang="ts">
  // why: Group Buff Tracker -- one word first ("Good" / "Missing N"),
  // then one line per buff kind the party can put on you: green with
  // the active spell, yellow with the best spell and who could cast it.
  // The party line shows who counts and what their classes read as.
  import type { GroupBuffsDto } from '$lib/tauri/api';
  let { data, opacity, overallOpacity }: { data: GroupBuffsDto | null; opacity: number; overallOpacity: number } = $props();
  const ABBR: Record<string, string> = {
    Warrior: 'WAR', Cleric: 'CLR', Paladin: 'PAL', Ranger: 'RNG', 'Shadow Knight': 'SHD', Druid: 'DRU',
    Monk: 'MNK', Bard: 'BRD', Rogue: 'ROG', Shaman: 'SHM', Necromancer: 'NEC', Wizard: 'WIZ',
    Magician: 'MAG', Enchanter: 'ENC', Beastlord: 'BST', Berserker: 'BER',
  };
  const abbr = (c: string) => ABBR[c] ?? c.slice(0, 3).toUpperCase();
  const missing = $derived(data ? data.rows.filter((r) => !r.active).length : 0);
</script>

<div
  class="flex h-full w-full flex-col gap-1 overflow-hidden rounded-sm border border-border/60 px-2 py-1.5 text-[11px]"
  style:background="rgba(20, 24, 30, {opacity})"
  style:opacity={overallOpacity}
  style:text-shadow="0 1px 2px rgba(0, 0, 0, 0.9), 0 0px 4px rgba(0, 0, 0, 0.6)"
>
  {#if !data}
    <p class="text-muted-foreground">group buffs…</p>
  {:else if !data.party.length}
    <p class="text-muted-foreground">group buffs: no party</p>
  {:else}
    <div class="flex items-baseline justify-between">
      <span class="font-medium {missing === 0 ? 'text-good' : 'text-caution'}">
        group buffs: {missing === 0 ? 'Good' : `missing ${missing}`}
      </span>
      <span class="truncate font-mono text-[10px] text-foreground/60" title="your classes">{data.my_classes.map(abbr).join('/')}</span>
    </div>
    <div class="truncate font-mono text-[10px] text-foreground/60" title="party -- confirmed classes count; ? means not confirmed yet">
      {#each data.party as m, i (m.name)}{i ? ' · ' : ''}{m.name} {m.classes.length ? m.classes.map(abbr).join('/') : '?'}{m.confirmed ? '' : '?'}{/each}
    </div>
    <div class="flex flex-col gap-0.5">
      {#each data.rows as r (r.kind)}
        <div class="flex items-baseline justify-between gap-2">
          <span class="text-foreground/80">{r.label}</span>
          {#if r.active}
            <span class="truncate text-good" title="on you">{r.active}</span>
          {:else}
            <span class="truncate text-caution" title="not on you -- who could cast it">{r.best_spell} ({r.casters.join(', ')})</span>
          {/if}
        </div>
      {/each}
      {#if !data.rows.length}
        <p class="text-muted-foreground">no confirmed party classes with buffs for you yet</p>
      {/if}
    </div>
  {/if}
</div>

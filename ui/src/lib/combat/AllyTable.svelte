<script lang="ts">
  import * as Table from '$lib/components/ui/table';
  import { allies, expandedAlly, allySummary, toggleAlly } from '$lib/stores/combat';
  import { trackedSkills, toggleTrackedSkill } from '$lib/stores/settings';
  import TargetIcon from '@lucide/svelte/icons/target';
  // why: the game's own three-letter codes, as /who prints them
  const ABBR: Record<string, string> = {
    Warrior: 'WAR', Cleric: 'CLR', Paladin: 'PAL', Ranger: 'RNG', 'Shadow Knight': 'SHD', Druid: 'DRU',
    Monk: 'MNK', Bard: 'BRD', Rogue: 'ROG', Shaman: 'SHM', Necromancer: 'NEC', Wizard: 'WIZ',
    Magician: 'MAG', Enchanter: 'ENC', Beastlord: 'BST', Berserker: 'BER',
  };
  const abbr = (c: string) => ABBR[c] ?? c.slice(0, 3).toUpperCase();
</script>

{#if $allies.length === 0}
  <p class="py-4 text-[12px] text-muted-foreground">No fights parsed for this selection yet.</p>
{:else}
  <Table.Root>
    <Table.Header>
      <Table.Row>
        <Table.Head>name</Table.Head>
        <Table.Head title="one class model for you and for them: a /who row is ground truth, otherwise evidence per encounter chain. Green once a class clears the bar, yellow while it is still a guess. A chain restarts when they leave, or you zone.">class</Table.Head>
        <Table.Head class="text-right">total</Table.Head>
        <Table.Head class="text-right">%</Table.Head>
        <Table.Head class="text-right">dps</Table.Head>
        <Table.Head class="text-right">hits</Table.Head>
        <Table.Head class="text-right">crit%</Table.Head>
      </Table.Row>
    </Table.Header>
    <Table.Body>
      {#each $allies as a (a.name)}
        <Table.Row
          class="cursor-pointer bg-no-repeat"
          style="background-image: linear-gradient(to right, color-mix(in srgb, var(--color-primary) 14%, transparent) {a.pct}%, transparent {a.pct}%)"
          onclick={() => toggleAlly(a.name)}
        >
          <!-- why: a suggested ally (charm pet / co-occurrence, no permanent
               proof -- see AllyDto.suggested's own doc) reads visibly
               tentative, not equal to a proven groupmate; pet_total > 0
               notes how much of an owner's row came via their pet -->
          <Table.Cell class={a.suggested ? 'text-muted-foreground italic' : a.is_player || a.is_pet ? 'text-primary' : ''}>
            {a.name}{#if a.suggested}<span
                class="ml-1 rounded-sm border border-border px-1 text-[9px] not-italic text-muted-foreground"
                title="Suggested ally -- included via charm or repeated co-occurrence, not proven">suggested</span
              >{/if}{#if a.pet_total > 0 && a.pet_total < a.total}<span
                class="ml-1 text-[10px] text-muted-foreground"
                title="Damage contributed by this ally's pet">(pet {a.pet_total.toLocaleString()})</span
              >{/if}
          </Table.Cell>
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
          <Table.Cell class="text-right tabular-nums">{a.total.toLocaleString()}</Table.Cell>
          <Table.Cell class="text-right tabular-nums">{a.pct.toFixed(1)}%</Table.Cell>
          <Table.Cell class="text-right tabular-nums">{a.dps.toFixed(1)}</Table.Cell>
          <Table.Cell class="text-right tabular-nums">{a.hits.toLocaleString()}</Table.Cell>
          <Table.Cell class="text-right tabular-nums">{a.crit_pct.toFixed(1)}%</Table.Cell>
        </Table.Row>
        {#if $expandedAlly === a.name && $allySummary}
          <Table.Row>
            <Table.Cell colspan={7} class="bg-muted/40 p-0">
              <div class="grid grid-cols-2 gap-3 p-3">
                {#if a.class_source !== 'who' && (a.classes.length < 3 || a.class_prior.length || a.class_conflicts || a.class_chain_end)}
                  <!-- why: Q34 -- what the open slot is stuck between, and the chain's state -->
                  <div class="col-span-2 text-[11px]">
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
                    {/if}
                  </div>
                {/if}
                <div>
                  <h4 class="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">abilities</h4>
                  <table class="w-full text-[11px]">
                    <tbody>
                      {#each $allySummary.abilities as ab (ab.ability)}
                        {@const avoided = ab.missed + ab.blocked + ab.dodged + ab.parried}
                        {@const attempts = ab.hits + avoided}
                        <tr class="group border-b border-border/50">
                          <td class="py-0.5">
                            <span class="inline-flex items-center gap-1">
                              {ab.ability}
                              <!-- why: "track" from wherever an ability shows up;
                                   adds/removes it from the Skill Tracker
                                   overlay's cooldowns section -->
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
                          <td class="py-0.5 text-right tabular-nums">{ab.total.toLocaleString()}</td>
                          <td class="py-0.5 text-right tabular-nums text-muted-foreground"
                            >{ab.hits}x{avoided > 0 ? `/${attempts}` : ''}</td
                          >
                          <td class="py-0.5 text-right tabular-nums text-muted-foreground">avg {ab.avg_hit.toFixed(0)}</td>
                          <td class="py-0.5 text-right tabular-nums text-muted-foreground">
                            {#if ab.crits > 0}crit {ab.avg_crit.toFixed(0)}{/if}
                          </td>
                          <td class="py-0.5 text-right tabular-nums text-bad">
                            {#if avoided > 0}
                              {[
                                ab.missed && `${ab.missed} miss`,
                                ab.blocked && `${ab.blocked} blk`,
                                ab.dodged && `${ab.dodged} dge`,
                                ab.parried && `${ab.parried} par`,
                              ]
                                .filter(Boolean)
                                .join(' ')}
                            {/if}
                          </td>
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                </div>
                <div>
                  <h4 class="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">casts</h4>
                  <table class="w-full text-[11px]">
                    <tbody>
                      {#each $allySummary.casts as c (c.spell)}
                        <tr class="border-b border-border/50">
                          <td class="py-0.5">{c.spell}</td>
                          <td class="py-0.5 text-right tabular-nums">{c.landed}/{c.attempts}</td>
                          <td class="py-0.5 text-right tabular-nums text-muted-foreground">
                            {#if c.resisted}{c.resisted} resisted{/if}
                            {#if c.interrupted}{c.interrupted} interrupted{/if}
                            {#if c.fizzled}{c.fizzled} fizzled{/if}
                          </td>
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                </div>
              </div>
            </Table.Cell>
          </Table.Row>
        {/if}
      {/each}
    </Table.Body>
  </Table.Root>
{/if}

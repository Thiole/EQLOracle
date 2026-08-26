<script lang="ts">
  import * as Table from '$lib/components/ui/table';
  import { allies, expandedAlly, allySummary, toggleAlly } from '$lib/stores/combat';
  import { trackedSkills, toggleTrackedSkill } from '$lib/stores/settings';
  import TargetIcon from '@lucide/svelte/icons/target';
</script>

{#if $allies.length === 0}
  <p class="py-4 text-[12px] text-muted-foreground">No fights parsed for this selection yet.</p>
{:else}
  <Table.Root>
    <Table.Header>
      <Table.Row>
        <Table.Head>name</Table.Head>
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
          <Table.Cell class={a.is_player || a.is_pet ? 'text-primary' : ''}>{a.name}</Table.Cell>
          <Table.Cell class="text-right tabular-nums">{a.total.toLocaleString()}</Table.Cell>
          <Table.Cell class="text-right tabular-nums">{a.pct.toFixed(1)}%</Table.Cell>
          <Table.Cell class="text-right tabular-nums">{a.dps.toFixed(1)}</Table.Cell>
          <Table.Cell class="text-right tabular-nums">{a.hits.toLocaleString()}</Table.Cell>
          <Table.Cell class="text-right tabular-nums">{a.crit_pct.toFixed(1)}%</Table.Cell>
        </Table.Row>
        {#if $expandedAlly === a.name && $allySummary}
          <Table.Row>
            <Table.Cell colspan={6} class="bg-muted/40 p-0">
              <div class="grid grid-cols-2 gap-3 p-3">
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
                              <!-- why: "track" from wherever an ability shows up
                                   -- Spencer's own ask; adds/removes it from the
                                   Skill Tracker overlay's own cooldowns section -->
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

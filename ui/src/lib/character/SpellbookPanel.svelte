<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { spellbook } from '$lib/stores/character';

  const knownCount = $derived($spellbook.filter((s) => s.confidence === 'known').length);
  const possibleCount = $derived($spellbook.length - knownCount);
</script>

<Card>
  <CardContent class="px-3 py-2">
    <h2 class="text-[11px] uppercase tracking-wide text-muted-foreground">Spellbook</h2>
    <p class="mb-2 text-[11px] text-muted-foreground">
      Every spell with at least Possible-tier evidence this session, from the log's own scribe/memorize begin and finish lines. "Known" means a
      finish was confirmed; "possible" means only a begin line landed, never confirmed complete.
    </p>

    {#if !$spellbook.length}
      <p class="text-[12px] text-muted-foreground">Nothing scribed or memorized yet this session.</p>
    {:else}
      <p class="mb-1 text-[12px]">
        <b class="tabular-nums">{knownCount}</b> known this session{#if possibleCount}, <b class="tabular-nums">{possibleCount}</b> possible (began
          scribing or memorizing, never confirmed finished){/if}.
      </p>
      <div class="overflow-x-auto">
        <table class="w-full text-[11px]">
          <thead>
            <tr class="border-b border-border text-muted-foreground">
              <th class="px-2 py-0.5 text-left font-normal">Confidence</th>
              <th class="px-2 py-0.5 text-left font-normal">When</th>
              <th class="px-2 py-0.5 text-left font-normal">Spell</th>
              <th class="px-2 py-0.5 text-left font-normal">Class(es)</th>
              <th class="px-2 py-0.5 text-right font-normal">Mana</th>
              <th class="px-2 py-0.5 text-right font-normal">Cast</th>
              <th class="px-2 py-0.5 text-left font-normal">Effect</th>
            </tr>
          </thead>
          <tbody>
            {#each $spellbook as s (s.name)}
              <tr class="border-b border-border/50">
                <td class="px-2 py-0.5">
                  <span class={s.confidence === 'known' ? 'text-primary' : 'text-muted-foreground'}>{s.confidence}</span>
                </td>
                <td class="px-2 py-0.5 whitespace-nowrap text-muted-foreground">{new Date(s.first_seen_ms).toLocaleString()}</td>
                <td class="px-2 py-0.5">{s.name}</td>
                <td class="px-2 py-0.5 text-muted-foreground">
                  {s.classes.length ? s.classes.map((c) => (c.level != null ? `${c.class} ${c.level}` : c.class)).join(', ') : '—'}
                </td>
                <td class="px-2 py-0.5 text-right tabular-nums">{s.mana ?? '—'}</td>
                <td class="px-2 py-0.5 text-right tabular-nums">{s.casting_time != null ? `${s.casting_time}s` : '—'}</td>
                <td class="max-w-xs truncate px-2 py-0.5 text-muted-foreground" title={s.description ?? undefined}>{s.description ?? '—'}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </CardContent>
</Card>

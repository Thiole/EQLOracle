<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { api, type AaDto, type AaGrantDto } from '$lib/tauri/api';
  import GdField from './GdField.svelte';

  // Every AA lives on one shared wiki page (table rows under section
  // headings, not individual pages) -- aadata.rs never scraped a per-entry
  // url, so this is the one constant link for all of them.
  const AA_WIKI_URL = 'https://eqlwiki.com/Alternate_Advancement';

  let { aa }: { aa: AaDto } = $props();

  let grants = $state<AaGrantDto[] | null>(null);
  let token = 0;
  $effect(() => {
    const name = aa.name;
    grants = null;
    const my = ++token;
    api.getAaLog().then((log) => {
      if (my === token) grants = log.grants.filter((g) => g.name === name);
    });
  });
  const best = $derived(grants?.length ? grants.reduce((a, b) => (b.rank > a.rank ? b : a)) : null);
  const firstGrantAt = $derived(grants?.length ? new Date(grants[0].ts_ms).toLocaleString() : '');
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h2 class="stat-figure text-[18px]">{aa.name}</h2>

    <div class="mt-2 flex flex-col">
      <GdField label="Category" value={aa.category} />
      <GdField label="Ranks" value={aa.ranks} />
      <GdField label="Cost per rank" value={aa.cost_raw} />
    </div>

    {#if aa.description}<p class="mt-2 text-[11px] text-muted-foreground">{aa.description}</p>{/if}
    {#if !aa.certain}
      <p class="mt-1 text-[11px] text-caution">
        The wiki table's rank/cost/level columns didn't line up cleanly for this entry -- treat the numbers above as approximate.
      </p>
    {/if}

    <a class="mt-3 inline-block text-[11px] text-brand-soft hover:text-primary hover:underline" href={AA_WIKI_URL} target="_blank" rel="noopener">
      eqlwiki ↗ (backup / full AA list)
    </a>
  </CardContent>
</Card>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h3 class="panel-title mb-1">your progress</h3>
    {#if grants === null}
      <p class="text-[11px] text-muted-foreground">Loading…</p>
    {:else if !best}
      <p class="text-[11px] text-muted-foreground">Not purchased yet this session.</p>
    {:else}
      <p class="text-[11px]">
        You own rank <b class="text-foreground">{best.rank}{best.max_rank ? ` / ${best.max_rank}` : ''}</b>, first purchased {firstGrantAt}.
      </p>
    {/if}
  </CardContent>
</Card>

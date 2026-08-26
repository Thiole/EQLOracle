<script lang="ts">
  // why: list_mobs' own doc warns this must stay one O(store length) pass,
  // called once here on mount -- not re-fetched per keystroke, filtering
  // below is client-side over the one loaded list.
  import { Input } from '$lib/components/ui/input';
  import { Card, CardContent } from '$lib/components/ui/card';
  import { api, type MobDto } from '$lib/tauri/api';
  import GdLink from '$lib/gamedata/GdLink.svelte';

  let mobs = $state<MobDto[] | null>(null);
  let loadError = $state<string | null>(null);
  let search = $state('');
  let expanded = $state<string | null>(null);

  $effect(() => {
    api
      .listMobs()
      .then((list) => (mobs = list ?? []))
      .catch((e) => (loadError = String(e)));
  });

  const q = $derived(search.trim().toLowerCase());
  const filtered = $derived((mobs ?? []).filter((m) => !q || m.name.toLowerCase().includes(q)));
  const totalKills = $derived((mobs ?? []).reduce((n, m) => n + m.kills, 0));
  const totalPulls = $derived((mobs ?? []).reduce((n, m) => n + m.pulls, 0));

  function toggle(name: string) {
    expanded = expanded === name ? null : name;
  }
</script>

<div class="flex flex-col gap-3 p-3">
  <Card class="rounded-sm">
    <CardContent class="flex items-center justify-between px-3 py-2.5">
      <h2 class="stat-figure text-[18px]">Loot History</h2>
      {#if mobs}
        <span class="text-[11px] text-muted-foreground">
          {totalKills.toLocaleString()} confirmed kill{totalKills === 1 ? '' : 's'} across {mobs.length.toLocaleString()} mob type{mobs.length === 1
            ? ''
            : 's'} ({totalPulls.toLocaleString()} pull{totalPulls === 1 ? '' : 's'} total)
        </span>
      {/if}
    </CardContent>
  </Card>

  {#if loadError}
    <p class="text-[11px] text-bad">Couldn't load mob history: {loadError}</p>
  {:else if !mobs}
    <p class="text-[11px] text-muted-foreground">Loading…</p>
  {:else if !mobs.length}
    <p class="text-[11px] text-muted-foreground">No confirmed pulls yet this session.</p>
  {:else}
    <Input placeholder="filter mobs…" bind:value={search} class="h-8 max-w-64 text-[12px]" />

    <div class="flex flex-col rounded-sm border border-border">
      {#each filtered as m (m.name)}
        {@const open = expanded === m.name}
        <div class="border-b border-border/50 last:border-b-0">
          <button
            type="button"
            class="flex w-full items-baseline gap-x-2 px-2 py-1 text-left text-[11px] hover:bg-muted/40"
            onclick={() => toggle(m.name)}
          >
            <span class="shrink-0 font-medium text-foreground"><GdLink kind="npc" name={m.name} /></span>
            <span class="min-w-0 truncate text-muted-foreground">
              {m.kills.toLocaleString()} kill{m.kills === 1 ? '' : 's'} / {m.pulls.toLocaleString()} pull{m.pulls === 1 ? '' : 's'}
              {#if m.avg_xp_pct !== null}
                · {m.avg_xp_pct.toFixed(2)}% xp avg
              {/if}
              {#if !m.known}
                · not in the wiki catalog — loot is what's actually dropped, not a full table
              {/if}
            </span>
          </button>
          {#if open}
            <div class="border-t border-border/50 bg-muted/10 px-2 py-1.5 text-[11px]">
              {#if !m.loot.length}
                <p class="text-muted-foreground">No drops recorded.</p>
              {:else}
                <table class="w-full text-[11px]">
                  <thead>
                    <tr class="text-left text-muted-foreground"><th class="pb-1 font-normal">item</th><th class="pb-1 font-normal">count</th></tr>
                  </thead>
                  <tbody>
                    {#each m.loot as row (row.item)}
                      <tr>
                        <td class="py-0.5"><GdLink kind="item" name={row.item} /></td>
                        <td class="py-0.5 text-muted-foreground">{row.count > 0 ? row.count.toLocaleString() : '—'}</td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

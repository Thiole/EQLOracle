<script lang="ts">
  // why: browse every real bag/bank/depot/key ring slot -- the "where is
  // my X" locate feature (ItemLocateLabel, wired into Sky Quests already)
  // answers one item at a time; this is the comprehensive view, for
  // scanning the whole inventory or searching it directly. Equip-doll
  // slots deliberately excluded -- Gear/Character Sheet already show those.
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Input } from '$lib/components/ui/input';
  import { Button } from '$lib/components/ui/button';
  import GdLink from '$lib/gamedata/GdLink.svelte';
  import { api, type InventoryContainerDto } from '$lib/tauri/api';

  let containers = $state<InventoryContainerDto[] | null>(null);
  let existingDump = $state<{ file: string; character: string | null } | null | undefined>(undefined);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let query = $state('');

  async function load() {
    error = null;
    loading = true;
    try {
      const [c, dump] = await Promise.all([api.getInventoryBrowser(), api.findExistingInventoryDump()]);
      containers = c;
      existingDump = dump;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void load();
  });

  const q = $derived(query.trim().toLowerCase());

  // why: filters to matching slots per container, hides a container
  // entirely once it has nothing left to show -- searching "efreeti"
  // across 21 containers should read as "here are the 3 that matter",
  // not a wall of unrelated bags with one highlighted row buried in each
  const filtered = $derived.by(() => {
    if (!containers) return null;
    if (!q) return containers;
    return containers
      .map((c) => ({ ...c, slots: c.slots.filter((s) => s.item.toLowerCase().includes(q)) }))
      .filter((c) => c.slots.length > 0 || (c.bag_item?.toLowerCase().includes(q) ?? false));
  });

  const totalSlots = $derived(containers?.reduce((n, c) => n + c.slots.length, 0) ?? 0);
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <div class="mb-1 flex flex-wrap items-center justify-between gap-2">
      <h2 class="panel-title">inventory · bags, bank, depot, key ring</h2>
      <Button size="sm" variant="ghost" class="h-6 text-[11px]" onclick={load} disabled={loading}>
        {loading ? 'refreshing…' : 'refresh'}
      </Button>
    </div>
    <p class="mb-2 text-[11px] text-muted-foreground">
      From your latest <code class="rounded bg-muted px-1 py-0.5">/outputfile inventory</code> dump{#if existingDump?.character}
        , <span class="text-foreground">{existingDump.character}</span>{/if}. Equipped gear lives on the
      <b class="text-foreground">Gear</b> tab instead.
    </p>
    <Input bind:value={query} placeholder="Search your inventory…" class="mb-2 h-7 max-w-sm text-[12px]" />
    {#if error}
      <div class="flex items-center gap-2 text-[12px]">
        <p class="text-destructive">{error}</p>
        <button type="button" class="text-primary underline" onclick={load}>retry</button>
      </div>
    {:else if !containers}
      <p class="text-[12px] text-muted-foreground">Loading…</p>
    {:else if existingDump === null}
      <p class="text-[12px] text-muted-foreground">
        No inventory dump found yet -- run <code class="rounded bg-muted px-1 py-0.5">/outputfile inventory</code> in game, then refresh.
      </p>
    {:else if !containers.length}
      <p class="text-[12px] text-muted-foreground">Nothing in storage -- everything's equipped, or the dump is empty.</p>
    {:else}
      <p class="mb-2 text-[10px] text-muted-foreground">
        {containers.length} container{containers.length === 1 ? '' : 's'}, {totalSlots} item{totalSlots === 1 ? '' : 's'}
        {#if q}&middot; {filtered?.length ?? 0} match{filtered?.length === 1 ? '' : 'es'}{/if}
      </p>
      {#if q && !filtered?.length}
        <p class="text-[12px] text-muted-foreground">Nothing matches "{query}".</p>
      {:else}
        <div class="flex flex-wrap gap-2">
          {#each filtered ?? [] as c (c.label)}
            <Card class="min-w-64 flex-1 basis-64 rounded-sm">
              <CardContent class="px-2.5 py-2">
                <div class="mb-1 flex items-baseline justify-between gap-2">
                  <span class="text-[11px] font-medium text-foreground">{c.label}</span>
                  {#if c.bag_item}<span class="text-[10px] text-muted-foreground">{c.bag_item}</span>{/if}
                </div>
                {#if !c.slots.length}
                  <p class="text-[10px] text-muted-foreground">empty</p>
                {:else}
                  <table class="w-full text-[10px]">
                    <tbody>
                      {#each c.slots as s (s.slot)}
                        <tr class="border-b border-border/50 last:border-0">
                          <td class="w-14 py-0.5 pr-1 text-muted-foreground">{s.slot}</td>
                          <td class="py-0.5"><GdLink kind="item" name={s.item} /></td>
                          <td class="py-0.5 pl-1 text-right text-muted-foreground">{s.tier ? `+${s.tier}` : ''}</td>
                          <td class="w-10 py-0.5 pl-1 text-right tabular-nums text-muted-foreground">{s.count > 1 ? `×${s.count}` : ''}</td>
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                {/if}
              </CardContent>
            </Card>
          {/each}
        </div>
      {/if}
    {/if}
  </CardContent>
</Card>

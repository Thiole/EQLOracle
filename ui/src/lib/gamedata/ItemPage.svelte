<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { api, type ItemDto, type LootEventDto } from '$lib/tauri/api';
  import { ICON_BASE } from '$lib/character/constants';
  import GdLinkList from './GdLinkList.svelte';
  import GdZoneOrMobLink from './GdZoneOrMobLink.svelte';

  let { item }: { item: ItemDto } = $props();

  const wikiUrl = $derived(item.url || `https://eqlwiki.com/${encodeURIComponent(item.name.replace(/ /g, '_'))}`);
  const statRows = $derived(Object.entries(item.stats).sort((a, b) => b[1] - a[1]));

  // why: token-guarded, same shape as the rest of this module's "your
  // history" sections -- click item A, click item B before A's fetch
  // resolves, A's response must not land on B's now-showing page.
  let loot = $state<LootEventDto[] | null>(null);
  let lootError = $state<string | null>(null);
  let token = 0;
  $effect(() => {
    const name = item.name;
    loot = null;
    lootError = null;
    const my = ++token;
    api
      .getItemLootHistory(name)
      .then((events) => {
        if (my === token) loot = events;
      })
      .catch((e) => {
        if (my === token) lootError = String(e);
      });
  });
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <div class="flex gap-3">
      {#if item.icon}
        <img src={ICON_BASE + encodeURIComponent(item.icon)} alt="" class="size-14 shrink-0 rounded-sm border border-border bg-muted/20" />
      {/if}
      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-2">
          <span class="stat-figure text-[18px]">{item.name}</span>
          <a class="text-[11px] text-brand-soft hover:text-primary hover:underline" href={wikiUrl} target="_blank" rel="noopener"
            >eqlwiki ↗ (backup)</a
          >
        </div>
        <div class="mt-0.5 text-[11px] text-muted-foreground">
          {[
            item.classes.length ? item.classes.join(' / ') : 'any class',
            item.slots.join(', '),
            item.era,
            item.wt != null ? `WT ${item.wt}` : null,
            item.size,
          ]
            .filter(Boolean)
            .join(' — ')}
        </div>
      </div>
    </div>

    {#if statRows.length}
      <div class="mt-2 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] tabular-nums text-primary">
        {#each statRows as [k, v] (k)}<span>{k} {v >= 0 ? '+' : ''}{v}</span>{/each}
      </div>
    {/if}

    {#if item.dmg != null && item.delay != null}
      <div class="mt-1.5 text-[11px] tabular-nums text-muted-foreground">
        {item.dmg} / {item.delay} ({(item.dmg / item.delay).toFixed(2)} ratio){item.skill ? ` — ${item.skill}` : ''}
      </div>
    {/if}

    {#if item.tags.length}
      <div class="mt-1.5 flex flex-wrap gap-1">
        {#each item.tags as t (t)}
          <span class="rounded-full border border-border bg-muted/30 px-1.5 py-0.5 text-[10px] text-muted-foreground">{t}</span>
        {/each}
      </div>
    {/if}

    {#if item.zones.length}
      <p class="mt-2 text-[11px]">
        Drops in: {#each item.zones as z, i (z + i)}{#if i > 0}<span>, </span>{/if}<GdZoneOrMobLink name={z} />{/each}
      </p>
    {/if}
    {#if item.mobs.length}
      <p class="mt-1 text-[11px]">From: <GdLinkList kind="npc" names={item.mobs} /></p>
    {/if}
  </CardContent>
</Card>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h3 class="panel-title mb-1">your history with this item</h3>
    {#if lootError}
      <p class="text-[11px] text-bad">Couldn't load loot history: {lootError}</p>
    {:else if loot === null}
      <p class="text-[11px] text-muted-foreground">Loading…</p>
    {:else if !loot.length}
      <p class="text-[11px] text-muted-foreground">Not looted yet this session.</p>
    {:else}
      <div class="flex flex-col gap-0.5">
        {#each [...loot].reverse() as e (e.ts_ms)}
          <div class="text-[11px] text-muted-foreground">
            {new Date(e.ts_ms).toLocaleString()} — {e.qty > 1 ? `${e.qty}x ` : ''}from <b class="text-foreground">{e.mob}</b> in {e.zone ??
              'unknown zone'}
          </div>
        {/each}
      </div>
    {/if}
  </CardContent>
</Card>

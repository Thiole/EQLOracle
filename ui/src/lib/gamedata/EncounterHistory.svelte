<script lang="ts">
  // why: shared by ZonePage ("your parsed encounters here") and NpcPage
  // ("your history with this mob") -- both list_zone_encounters and
  // list_mob_encounters return the identical ZoneEncounterDto shape, so
  // one render path covers both. `showZone` adds each fight's own zone
  // to the row (meaningful on an NPC page -- the same mob can turn up in
  // different zones; redundant on a zone page, left off there).
  import { api, type ZoneEncounterDto, type EncounterDetailDto } from '$lib/tauri/api';
  import { requestJumpToEncounter } from '$lib/stores/combat';
  import { activeModule } from '$lib/stores/shell';

  /** why: matches list_zone_encounters/list_mob_encounters' own backend
   * default -- not passed explicitly over IPC (omitting it there lets the
   * mock fixture key stay the simple no-limit shape every other browsing
   * call already uses), just mirrored here for the "showing N most
   * recent" cap note below. */
  const LIMIT = 30;

  let { kind, id, showZone }: { kind: 'zone' | 'npc'; id: string; showZone: boolean } = $props();

  let encounters = $state<ZoneEncounterDto[] | null>(null);
  let loadError = $state<string | null>(null);
  let expandedId = $state<number | null>(null);
  /** why: absent key = not fetched yet ("Loading…"); present = resolved
   * (a real detail, or a zero-valued fallback on failure -- see below,
   * same "stop showing Loading forever" stance the legacy version took). */
  let details = $state<Record<number, EncounterDetailDto>>({});

  let token = 0;
  $effect(() => {
    const fetchFn = kind === 'zone' ? api.listZoneEncounters : api.listMobEncounters;
    const target = id;
    encounters = null;
    loadError = null;
    details = {};
    expandedId = null;
    const my = ++token;
    fetchFn(target)
      .then((list) => {
        if (my !== token) return;
        list ??= []; // defensive -- invoke<T>()'s type is an assertion, not a guarantee
        encounters = list;
        // Damage totals + drops load one encounter at a time, after the
        // list itself is already on screen -- each row paints in as its
        // own (cheap, windowed) fetch resolves, not all together.
        for (const ze of list) {
          api
            .getEncounterDetail(ze.encounter.id)
            .then((d) => {
              if (my !== token) return;
              details = { ...details, [ze.encounter.id]: d ?? { total_damage: 0, dps: 0, enemy_damage: 0, enemy_dps: 0, drops: [] } };
            })
            .catch(() => {
              if (my !== token) return;
              details = { ...details, [ze.encounter.id]: { total_damage: 0, dps: 0, enemy_damage: 0, enemy_dps: 0, drops: [] } };
            });
        }
      })
      .catch((e) => {
        if (my !== token) return;
        loadError = String(e);
      });
  });

  function fmtTtk(ms: number): string {
    const total = Math.max(0, Math.round(ms / 1000));
    const m = Math.floor(total / 60);
    const s = total % 60;
    return m > 0 ? `${m}m ${s}s` : `${s}s`;
  }
  function toggle(id: number) {
    expandedId = expandedId === id ? null : id;
  }
  function openInCombat(ze: ZoneEncounterDto) {
    requestJumpToEncounter(ze.zone_visit, ze.encounter.id, ze.encounter.target);
    activeModule.set('combat');
  }
</script>

{#if loadError}
  <p class="text-[11px] text-bad">Couldn't load encounters: {loadError}</p>
{:else if !encounters}
  <p class="text-[11px] text-muted-foreground">Loading…</p>
{:else if !encounters.length}
  <p class="text-[11px] text-muted-foreground">No parsed encounters here yet.</p>
{:else}
  <div class="flex flex-col rounded-sm border border-border">
    {#each encounters as ze (ze.encounter.id)}
      {@const e = ze.encounter}
      {@const outcome = e.open ? 'ongoing' : e.slain ? 'kill' : e.wiped ? 'wipe' : 'reset'}
      {@const tierLabel = ze.tier > 0 ? `tier ${ze.tier}` : 'base difficulty'}
      {@const detail = details[e.id]}
      {@const expanded = expandedId === e.id}
      <div class="border-b border-border/50 last:border-b-0">
        <button
          type="button"
          class="flex w-full items-baseline gap-x-2 px-2 py-1 text-left text-[11px] hover:bg-muted/40"
          onclick={() => toggle(e.id)}
        >
          <span class="shrink-0 font-medium text-foreground">{e.target}</span>
          <span class="min-w-0 truncate text-muted-foreground"
            >{new Date(e.start_ms).toLocaleString()} · ttk {fmtTtk(e.duration_ms)} · {tierLabel} · {outcome}{showZone && ze.zone
              ? ` · ${ze.zone}`
              : ''}</span
          >
        </button>
        {#if expanded}
          <div class="border-t border-border/50 bg-muted/10 px-2 py-1.5 text-[11px]">
            {#if detail === undefined}
              <p class="text-muted-foreground">Loading…</p>
            {:else}
              <p>
                {detail.total_damage.toLocaleString()} dmg dealt ({Math.round(detail.dps).toLocaleString()} dps) — {detail.enemy_damage.toLocaleString()}
                dmg taken ({Math.round(detail.enemy_dps).toLocaleString()} dps)
              </p>
              <p class="mt-0.5 text-muted-foreground">
                {#if detail.drops.length}
                  {detail.drops.map((d) => `${d.item}${d.qty > 1 ? ` ×${d.qty}` : ''}`).join(', ')}
                {:else}
                  no drops recorded
                {/if}
              </p>
            {/if}
            <button
              type="button"
              class="mt-1 text-[10px] text-brand-soft hover:text-primary hover:underline"
              onclick={(evt) => {
                evt.stopPropagation();
                openInCombat(ze);
              }}
            >
              open in Combat →
            </button>
          </div>
        {/if}
      </div>
    {/each}
  </div>
  {#if encounters.length >= LIMIT}
    <p class="mt-1 text-[10px] text-muted-foreground">Showing the {LIMIT} most recent — older fights aren't loaded here.</p>
  {/if}
{/if}

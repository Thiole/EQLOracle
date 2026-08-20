<script lang="ts">
  import { Card, CardContent } from '$lib/components/ui/card';
  import { api, type SpellDto, type SpellbookEntryDto } from '$lib/tauri/api';
  import { ICON_BASE } from '$lib/character/constants';
  import { spellEffects } from '$lib/stores/gamedata';
  import { wikiLinkText } from './wikitext';
  import GdField from './GdField.svelte';

  let { spell }: { spell: SpellDto } = $props();

  const wikiUrl = $derived(spell.url || `https://eqlwiki.com/${encodeURIComponent(spell.name.replace(/ /g, '_'))}`);
  const effects = $derived($spellEffects[spell.id]);

  const skillLabel = $derived(spell.skill ? wikiLinkText(spell.skill).replace(/^Skill /, '') : null);

  // Seconds -> a short human string. `null` (nothing parsed) reads as
  // "?", not "0s" or blank -- a real fraction of durations don't parse
  // into a clean number (spelleffect.rs's own doc).
  function fmtSecs(secs: number | null): string {
    if (secs == null) return '?';
    if (secs === 0) return 'instant';
    if (secs < 60) return `${Math.round(secs)}s`;
    if (secs < 3600) return `${(secs / 60).toFixed(secs % 60 === 0 ? 0 : 1)}m`;
    return `${(secs / 3600).toFixed(1)}h`;
  }
  const durationLabel = $derived.by(() => {
    if (!effects) return '?';
    const d = effects.duration;
    if (d.is_permanent) return 'permanent';
    if (d.is_instant) return 'instant';
    if (d.max_secs == null) return '?';
    if (d.min_secs != null && d.min_secs !== d.max_secs) return `${fmtSecs(d.min_secs)}–${fmtSecs(d.max_secs)}`;
    return fmtSecs(d.max_secs);
  });

  let known = $state<SpellbookEntryDto | undefined>(undefined);
  let loadedKnown = $state(false);
  let token = 0;
  $effect(() => {
    const name = spell.name;
    loadedKnown = false;
    const my = ++token;
    api.getSpellbook().then((entries) => {
      if (my !== token) return;
      known = entries.find((e) => e.name === name);
      loadedKnown = true;
    });
  });
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <div class="flex items-center gap-3">
      {#if spell.icon}
        <img src={ICON_BASE + encodeURIComponent(spell.icon)} alt="" class="size-10 shrink-0 rounded-sm border border-border bg-muted/20" />
      {/if}
      <span class="stat-figure text-[18px]">{spell.name}</span>
    </div>

    <div class="mt-2 flex flex-col">
      <GdField label="Class(es)" value={spell.classes.map((c) => (c.level != null ? `${c.class} ${c.level}` : c.class)).join(', ')} />
      <GdField label="Skill" value={skillLabel} />
      <GdField label="Mana" value={spell.mana} />
      <GdField label="Casting time" value={spell.casting_time != null ? `${spell.casting_time}s` : null} />
      <GdField label="Recast time" value={spell.recast_time != null ? `${spell.recast_time}s` : null} />
      <GdField label="Range" value={spell.range} />
      <GdField label="Duration" value={spell.duration} />
      <GdField label="Target" value={spell.target_type} />
      <GdField label="Resist" value={spell.resist} />
      <GdField label="Era" value={spell.era} />
    </div>

    {#if spell.description}<p class="mt-2 text-[11px] text-muted-foreground">{spell.description}</p>{/if}

    <a class="mt-3 inline-block text-[11px] text-brand-soft hover:text-primary hover:underline" href={wikiUrl} target="_blank" rel="noopener">
      eqlwiki ↗ (backup)
    </a>
  </CardContent>
</Card>

{#if effects}
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <h3 class="panel-title mb-1">effects</h3>
      <p class="text-[11px]"><b class="text-foreground">Duration:</b> {durationLabel}</p>
      {#if effects.tags.length}
        <div class="mt-1 flex flex-wrap gap-1">
          {#each effects.tags as t (t)}
            <span class="rounded-full border border-primary/40 bg-primary/10 px-1.5 py-0.5 text-[10px] text-primary">{t}</span>
          {/each}
        </div>
      {/if}
      {#each effects.components as c, i (i)}
        {@const range = c.min_amount != null && c.min_amount !== c.max_amount ? `${c.min_amount}–${c.max_amount}` : c.min_amount}
        {@const unit = c.unit === 'percent' ? '%' : ''}
        <p class="mt-0.5 text-[11px] text-muted-foreground">{c.direction} {c.stat}: {range}{unit}{c.per_tick ? '/tick' : ''}</p>
      {/each}
      {#if effects.description_damage}
        {@const d = effects.description_damage}
        {@const range = d.min_amount !== d.max_amount ? `${d.min_amount}–${d.max_amount}` : d.min_amount}
        <p class="mt-0.5 text-[11px] text-muted-foreground">
          damage (from description): {range}{d.is_over_time ? ' over time' : ''}{d.repetitions ? ` × up to ${d.repetitions}` : ''}
        </p>
      {/if}
      {#if effects.description_heal}
        {@const d = effects.description_heal}
        {@const range = d.min_amount !== d.max_amount ? `${d.min_amount}–${d.max_amount}` : d.min_amount}
        <p class="mt-0.5 text-[11px] text-muted-foreground">heal (from description): {range}{d.is_over_time ? ' over time' : ''}</p>
      {/if}
    </CardContent>
  </Card>
{/if}

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <h3 class="panel-title mb-1">confirmed known</h3>
    {#if !loadedKnown}
      <p class="text-[11px] text-muted-foreground">Loading…</p>
    {:else if !known}
      <p class="text-[11px] text-muted-foreground">No evidence yet this session.</p>
    {:else if known.confidence === 'known'}
      <p class="text-[11px]">
        <span class="rounded-full border border-good/40 bg-good/10 px-1.5 py-0.5 text-[10px] text-good">known</span>
        since {new Date(known.first_seen_ms).toLocaleString()}.
      </p>
    {:else}
      <p class="text-[11px]">
        <span class="rounded-full border border-caution/40 bg-caution/10 px-1.5 py-0.5 text-[10px] text-caution">possible</span>
        — began {new Date(known.first_seen_ms).toLocaleString()}, never confirmed finished.
      </p>
    {/if}
  </CardContent>
</Card>

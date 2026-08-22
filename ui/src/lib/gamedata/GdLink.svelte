<script lang="ts">
  // why: a cross-reference name becomes a real in-app link only when that
  // category's own catalog has a match -- otherwise it's just text, not a
  // dead link dressed up to look clickable.
  import { gdFind, gdOpenPage, type GdKind } from '$lib/stores/gamedata';
  import { displayZoneName } from '$lib/utils';

  let { kind, name }: { kind: GdKind; name: string } = $props();
  const found = $derived(!!gdFind(kind, name));
  // why: lookup/navigation always uses the real `name` -- only the
  // rendered text is display-cleaned, and only for zones (see
  // `displayZoneName`'s own doc for why this can't apply to every kind).
  const label = $derived(kind === 'zone' ? displayZoneName(name) : name);
</script>

{#if found}
  <button type="button" class="text-brand-soft hover:text-primary hover:underline" onclick={() => gdOpenPage(kind, name)}>{label}</button>
{:else}
  <span>{label}</span>
{/if}

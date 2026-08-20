<script lang="ts">
  // why: a cross-reference name becomes a real in-app link only when that
  // category's own catalog has a match -- otherwise it's just text, not a
  // dead link dressed up to look clickable.
  import { gdFind, gdOpenPage, type GdKind } from '$lib/stores/gamedata';

  let { kind, name }: { kind: GdKind; name: string } = $props();
  const found = $derived(!!gdFind(kind, name));
</script>

{#if found}
  <button type="button" class="text-brand-soft hover:text-primary hover:underline" onclick={() => gdOpenPage(kind, name)}>{name}</button>
{:else}
  <span>{name}</span>
{/if}

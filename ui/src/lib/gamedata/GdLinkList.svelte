<script lang="ts">
  import GdLink from './GdLink.svelte';
  import type { GdKind } from '$lib/stores/gamedata';

  let { kind, names }: { kind: GdKind; names: string[] } = $props();
</script>

{#each names as n, i (n + i)}{#if i > 0}<span>, </span>{/if}<GdLink {kind} name={n} />{/each}

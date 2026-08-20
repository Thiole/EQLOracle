<script lang="ts">
  // why: one field row, skipped entirely when there's nothing to show --
  // most wiki scrapes leave half their optional fields blank, and a blank
  // row reads as "the app is broken" where an absent one reads as "the
  // wiki just didn't have this".
  import type { Snippet } from 'svelte';

  let { label, value }: { label: string; value?: string | number | Snippet | null } = $props();
  const empty = $derived(value == null || value === '');
</script>

{#if !empty}
  <div class="flex gap-2 py-0.5 text-[11px]">
    <span class="w-32 shrink-0 text-muted-foreground">{label}</span>
    <span class="min-w-0 flex-1"
      >{#if typeof value === 'function'}{@render value()}{:else}{value}{/if}</span
    >
  </div>
{/if}

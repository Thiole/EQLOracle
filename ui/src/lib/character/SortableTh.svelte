<script lang="ts">
  // why: click-to-sort table header, reused across the AA page's 3 tables
  let {
    label,
    active,
    dir,
    onclick,
    align = 'left',
  }: { label: string; active: boolean; dir: 1 | -1; onclick: () => void; align?: 'left' | 'right' } = $props();
</script>

<th
  class="cursor-pointer px-2 py-0.5 font-normal text-muted-foreground select-none hover:text-foreground {align === 'right' ? 'text-right' : 'text-left'}"
  {onclick}
>
  {label}{#if active}<span class="ml-0.5 text-primary">{dir === 1 ? '▲' : '▼'}</span>{/if}
</th>

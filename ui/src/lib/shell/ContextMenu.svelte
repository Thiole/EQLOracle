<script module lang="ts">
  export interface MenuItem {
    label: string;
    onSelect?: () => void;
    disabled?: boolean;
    children?: MenuItem[];
  }
</script>

<script lang="ts">
  // why: one right-click panel for the whole app -- fixed at the cursor,
  // closed by a click elsewhere, Escape, or picking an item. A group
  // expands in place on click; no hover timing to get wrong on WebKit.
  let {
    x,
    y,
    items,
    onclose,
  }: { x: number; y: number; items: MenuItem[]; onclose: () => void } = $props();

  let panelEl: HTMLDivElement | undefined = $state();
  let openGroup = $state<string | null>(null);

  function pick(item: MenuItem) {
    if (item.disabled) return;
    if (item.children) {
      openGroup = openGroup === item.label ? null : item.label;
      return;
    }
    item.onSelect?.();
    onclose();
  }

  function onDocPointerDown(e: PointerEvent) {
    if (panelEl?.contains(e.target as Node)) return;
    onclose();
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') onclose();
  }

  $effect(() => {
    document.addEventListener('pointerdown', onDocPointerDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onDocPointerDown);
      document.removeEventListener('keydown', onKey);
    };
  });

  // why: kept on screen -- a click near the right or bottom edge opens leftward or upward
  const left = $derived(Math.min(x, Math.max(0, window.innerWidth - 240)));
  const top = $derived(Math.min(y, Math.max(0, window.innerHeight - 320)));
</script>

<div
  bind:this={panelEl}
  role="menu"
  data-testid="context-menu"
  class="fixed z-50 min-w-44 rounded-md border border-border bg-card p-1 text-[12px] shadow-lg"
  style="left: {left}px; top: {top}px"
>
  {#each items as item (item.label)}
    <button
      type="button"
      role="menuitem"
      disabled={item.disabled}
      onclick={() => pick(item)}
      class="flex w-full items-center justify-between rounded-sm px-2 py-1 text-left hover:bg-accent hover:text-accent-foreground disabled:cursor-not-allowed disabled:opacity-50"
    >
      <span>{item.label}</span>
      {#if item.children}<span class="text-muted-foreground">{openGroup === item.label ? '▾' : '▸'}</span>{/if}
    </button>
    {#if item.children && openGroup === item.label}
      <div class="mb-1 ml-3 border-l border-border pl-1">
        {#if !item.children.length}
          <p class="px-2 py-1 text-[11px] text-muted-foreground">nobody detected</p>
        {/if}
        {#each item.children as child (child.label)}
          <button
            type="button"
            role="menuitem"
            disabled={child.disabled}
            onclick={() => pick(child)}
            class="flex w-full rounded-sm px-2 py-1 text-left hover:bg-accent hover:text-accent-foreground disabled:cursor-not-allowed disabled:opacity-50"
          >
            {child.label}
          </button>
        {/each}
      </div>
    {/if}
  {/each}
</div>

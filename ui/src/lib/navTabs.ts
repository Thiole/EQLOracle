// why: the shared Tabs component's own default look (`tabs-list.svelte`'s
// `bg-muted` container, `tabs-trigger.svelte`'s active-pill styling) reads
// as one blended segmented bar -- asked directly to make a tab strip look
// like separate buttons on a nav menu instead, not that bar. These two
// class strings are the override, meant to be passed as the `class` prop
// on `Tabs.List`/`Tabs.Trigger` (Svelte merges a passed `class` in after
// the component's own base classes via `cn()`/tailwind-merge, so this
// only needs to state what should differ, not repeat everything).
// Shared, not copy-pasted per module, so every tab strip that opts into
// this look stays visually identical -- currently Endgame's own strip and
// Game Data's.
export const TAB_LIST_CLASS = 'bg-transparent p-0 gap-1.5';

export const TAB_TRIGGER_CLASS =
  'rounded-md border border-border bg-card px-2.5 py-1 text-foreground/70 shadow-none ' +
  'hover:border-foreground/30 hover:text-foreground ' +
  'data-active:border-primary data-active:bg-primary/10 data-active:text-primary data-active:shadow-none ' +
  'after:hidden';

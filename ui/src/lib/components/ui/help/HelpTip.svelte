<script lang="ts">
  // why: every page and section carried its explanation as permanently
  // visible prose, in the same size and weight as the control it
  // explained -- 25 paragraphs on Settings -> Overlay alone, against 12
  // actual controls. The text is read once and then is noise forever.
  // This is the one place it lives instead: a "?" that opens it on
  // demand, so the page shows controls and the words are one click away.
  //
  // Click, not hover: the content is sentences, and a hover tooltip that
  // vanishes when you move toward it cannot be read. Closes on click
  // outside or Escape -- same pointerdown pattern OverlayQuickMenu uses,
  // rather than a popover primitive, since nothing here needs anchoring
  // logic beyond "under the button, right-aligned".
  import HelpCircleIcon from '@lucide/svelte/icons/circle-help';
  import type { Snippet } from 'svelte';

  // why: `text` for the common case (one paragraph moved off the page)
  // and `children` for the few that need markup -- a caller should not
  // have to wrap a sentence in a snippet to hide it.
  let {
    label = 'What is this?',
    text,
    children,
  }: {
    label?: string;
    text?: string;
    children?: Snippet;
  } = $props();

  let open = $state(false);
  let root: HTMLSpanElement | undefined = $state();
  let btn: HTMLButtonElement | undefined = $state();
  // why: FIXED, not absolute -- an absolute panel is clipped by any
  // overflow ancestor (a Card, <main>'s own scroller) and can be painted
  // under a sibling that owns a stacking context, which is why it did not
  // draw over the sidebar. Fixed escapes both; the cost is measuring the
  // button, which is one read on open.
  let pos = $state({ top: 0, right: 0 });
  function place() {
    if (!btn) return;
    const r = btn.getBoundingClientRect();
    pos = { top: r.bottom + 6, right: Math.max(8, window.innerWidth - r.right) };
  }

  function onDocPointerDown(e: PointerEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false;
  }
  $effect(() => {
    document.addEventListener('pointerdown', onDocPointerDown);
    return () => document.removeEventListener('pointerdown', onDocPointerDown);
  });
</script>

<span bind:this={root} class="relative inline-flex shrink-0">
  <button
    bind:this={btn}
    type="button"
    aria-label={label}
    aria-expanded={open}
    title={label}
    class="rounded-full p-0.5 text-muted-foreground hover:text-foreground {open ? 'text-foreground' : ''}"
    onclick={() => {
      open = !open;
      if (open) place();
    }}
  >
    <HelpCircleIcon class="size-3.5" />
  </button>
  {#if open}
    <!-- why: flat and fully opaque -- the UI ships on WebKitGTK, where
         backdrop-filter and blur are out, and a see-through panel over
         dense text is unreadable regardless. -->
    <div
      role="note"
      style:top="{pos.top}px"
      style:right="{pos.right}px"
      class="fixed z-[200] w-72 rounded-sm border border-border bg-background p-2 text-[11px] leading-relaxed text-muted-foreground shadow-md"
    >
      {#if text}{text}{/if}{#if children}{@render children()}{/if}
    </div>
  {/if}
</span>

<!-- why: a fixed panel keeps its screen position while the page moves
     under it, so scrolling or resizing closes it rather than leaving it
     pointing at nothing -->
<svelte:window
  onkeydown={(e) => {
    if (e.key === 'Escape') open = false;
  }}
  onresize={() => (open = false)}
  onscrollcapture={() => (open = false)}
/>

<script lang="ts">
  // why: "where is my X" -- turns an already-shown "have ×N" style label
  // into a click-to-reveal locate affordance, reusing the real dump data
  // `inventory.rs::locate` now keeps (see its own doc). Only offered when
  // the item is actually owned -- there's nothing to locate otherwise,
  // same reasoning Drop Watch's own bell is hidden for untrackable items.
  import { api, type ItemLocationDto } from '$lib/tauri/api';

  let { item, label, owned }: { item: string; label: string; owned: boolean } = $props();

  let open = $state(false);
  let locations = $state<ItemLocationDto[] | null>(null);
  let loading = $state(false);

  async function toggle() {
    if (!owned) return;
    open = !open;
    if (open && locations === null) {
      loading = true;
      try {
        locations = await api.locateItem(item);
      } finally {
        loading = false;
      }
    }
  }
</script>

{#if owned}
  <button
    type="button"
    class="underline decoration-dotted decoration-current/50 underline-offset-2"
    onclick={toggle}
    title="Where is this?"
  >
    {label}
  </button>
  {#if open}
    <!-- why: an inline span, not a block div -- this renders inside
         other components' own tight single-line chips (SkyQuests/
         SkyClassUnlocks' item chips), a block element there would break
         their inline-flex row layout -->
    <span class="italic opacity-70">
      {#if loading}
        (locating…)
      {:else if !locations?.length}
        (not in the latest inventory dump)
      {:else}
        ({#each locations as l, i (l.label)}{i > 0 ? '; ' : ''}{l.count > 1 ? `${l.count}× ` : ''}{l.label}{/each})
      {/if}
    </span>
  {/if}
{:else}
  {label}
{/if}

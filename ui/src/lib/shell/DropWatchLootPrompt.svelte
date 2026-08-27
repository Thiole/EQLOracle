<script lang="ts">
  // why: own timer per prompt -- each tracked item that came in gets its
  // own countdown, independent of any others pending at the same time
  import { Button } from '$lib/components/ui/button';
  import { resolveLootPrompt, LOOT_PROMPT_TIMEOUT_MS } from '$lib/stores/dropWatchLoot';

  let { item, count, expiresAtMs }: { item: string; count: number; expiresAtMs: number } = $props();

  let remainingMs = $state(LOOT_PROMPT_TIMEOUT_MS);

  $effect(() => {
    remainingMs = Math.max(0, expiresAtMs - Date.now());
    const id = setInterval(() => {
      remainingMs = Math.max(0, expiresAtMs - Date.now());
      // why: the timer running out with no answer means no change -- see
      // resolveLootPrompt's own doc, this is not a "declined" click
      if (remainingMs <= 0) resolveLootPrompt(item, false);
    }, 200);
    return () => clearInterval(id);
  });

  const pct = $derived(Math.round((remainingMs / LOOT_PROMPT_TIMEOUT_MS) * 100));
</script>

<div class="rounded-sm border border-primary/40 bg-card px-3 py-2.5 text-[12px] shadow-lg">
  <p class="mb-1.5">
    You picked up <span class="text-primary">{item}</span>{#if count > 1}
      <span class="text-muted-foreground"> (×{count})</span>
    {/if} -- remove it from Drop Watch?
  </p>
  <div class="mb-2 h-0.5 overflow-hidden rounded-full bg-muted">
    <div class="h-full bg-primary/50 transition-[width] duration-200 linear" style:width="{pct}%"></div>
  </div>
  <div class="flex justify-end gap-2">
    <Button size="sm" variant="ghost" class="h-6 text-[11px]" onclick={() => resolveLootPrompt(item, false)}>
      Keep tracking
    </Button>
    <Button size="sm" class="h-6 text-[11px]" onclick={() => resolveLootPrompt(item, true)}>Remove</Button>
  </div>
</div>

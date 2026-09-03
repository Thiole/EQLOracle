<script lang="ts">
  // why: "a what's new page when a user updates, so they can quickly read
  // what's new since they last updated" -- opens on its own on the first
  // launch after an update with the unread sections; the Info panel can
  // open it with the whole changelog. Renders the changelog's own small
  // markdown subset (### headings, - bullets, paragraphs) itself.
  import { Button } from '$lib/components/ui/button';
  import { whatsNew, closeWhatsNew, ackWhatsNew } from '$lib/stores/whatsnew';

  type Block = { kind: 'h'; text: string } | { kind: 'li'; text: string } | { kind: 'p'; text: string };
  function blocks(body: string): Block[] {
    const out: Block[] = [];
    for (const raw of body.split('\n')) {
      const line = raw.trim();
      if (!line) continue;
      if (line.startsWith('### ')) out.push({ kind: 'h', text: line.slice(4) });
      else if (line.startsWith('- ')) out.push({ kind: 'li', text: line.slice(2) });
      else out.push({ kind: 'p', text: line });
    }
    return out;
  }
  // why: bold and code only -- the two inline marks the changelog uses
  function inline(text: string): string {
    return text
      .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
      .replace(/\*\*(.+?)\*\*/g, '<b>$1</b>')
      .replace(/`([^`]+)`/g, '<code class="rounded-sm bg-muted px-1 font-mono text-[11px]">$1</code>');
  }
</script>

{#if $whatsNew}
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-background/70 p-6" role="dialog" aria-modal="true">
    <div class="flex max-h-[80vh] w-full max-w-2xl flex-col rounded-sm border border-border bg-card shadow-xl">
      <div class="flex items-center justify-between border-b border-border px-4 py-2.5">
        <h2 class="stat-figure text-[16px]">
          {$whatsNew.mode === 'update' ? `What's new in ${$whatsNew.sections[0]?.version ?? ''}` : 'Changelog'}
        </h2>
        {#if $whatsNew.mode === 'update' && $whatsNew.lastSeen}
          <span class="font-mono text-[11px] text-muted-foreground">since {$whatsNew.lastSeen}</span>
        {/if}
      </div>
      <div class="flex-1 overflow-y-auto px-4 py-3 text-[12.5px] leading-relaxed">
        {#each $whatsNew.sections as s (s.version)}
          <div class="mb-4">
            <div class="mb-1 flex items-baseline gap-2">
              <span class="font-mono text-[13px] text-primary">{s.version}</span>
              <span class="font-mono text-[11px] text-muted-foreground">{s.date}</span>
            </div>
            {#each blocks(s.body) as b, i (i)}
              {#if b.kind === 'h'}
                <p class="mt-2 mb-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">{b.text}</p>
              {:else if b.kind === 'li'}
                <p class="ml-3 before:mr-1.5 before:text-muted-foreground before:content-['•']">{@html inline(b.text)}</p>
              {:else}
                <p>{@html inline(b.text)}</p>
              {/if}
            {/each}
          </div>
        {/each}
      </div>
      <div class="flex justify-end gap-2 border-t border-border px-4 py-2">
        {#if $whatsNew.mode === 'update'}
          <Button size="sm" onclick={ackWhatsNew}>got it</Button>
        {:else}
          <Button size="sm" variant="ghost" onclick={closeWhatsNew}>close</Button>
        {/if}
      </div>
    </div>
  </div>
{/if}

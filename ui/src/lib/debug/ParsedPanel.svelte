<script lang="ts">
  // why: every in-memory table behind one regex, newest first, the scan
  // cut at the limit -- the rendered row is what a custom trigger will
  // match against later
  import { onMount } from 'svelte';
  import { Card, CardContent } from '$lib/components/ui/card';
  import * as Select from '$lib/components/ui/select';
  import { api, type SearchDbDto } from '$lib/tauri/api';
  import { refreshOn } from '$lib/tauri/events';

  let table = $state('events');
  let pattern = $state('');
  let result = $state<SearchDbDto | null>(null);
  let error = $state<string | null>(null);
  let busy = false;
  let again = false;

  // why: one call in flight; a request during it runs once it returns
  async function run() {
    if (busy) {
      again = true;
      return;
    }
    busy = true;
    try {
      const r = await api.searchDb(table, pattern);
      if (r) result = r;
      error = r ? null : 'no backend in this session';
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
      if (again) {
        again = false;
        void run();
      }
    }
  }

  // why: 200ms after the last keystroke -- a no-match regex renders the
  // whole store, so never per keystroke
  let timer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    void table;
    void pattern;
    clearTimeout(timer);
    timer = setTimeout(() => void run(), 200);
    return () => clearTimeout(timer);
  });

  // why: live while open -- at most one re-run per 2s of parsed lines
  onMount(() => {
    let last = 0;
    return refreshOn('*', () => {
      const now = Date.now();
      if (now - last < 2000) return;
      last = now;
      void run();
    });
  });

  const summary = $derived.by(() => {
    if (!result) return '';
    const shown = `${result.matched}${result.truncated ? '+' : ''} of ${result.total.toLocaleString()} rows`;
    return result.truncated ? `${shown} · scan cut after ${result.scanned.toLocaleString()}` : shown;
  });
</script>

<Card class="rounded-sm">
  <CardContent class="px-3 py-2.5">
    <div class="mb-2 flex flex-wrap items-center gap-2">
      <h2 class="panel-title">parsed · db search</h2>
      <!-- why: the app's own list, not the native popup -- GTK draws that
           one in its own colours and it never follows the theme -->
      <Select.Root type="single" value={table} onValueChange={(v) => v && (table = v)}>
        <Select.Trigger class="h-6 w-44 font-mono text-[11px]" aria-label="table" data-testid="db-table">
          {table}
        </Select.Trigger>
        <Select.Content>
          {#each result?.tables ?? [table] as t (t)}
            <Select.Item value={t} class="font-mono text-[11px]">{t}</Select.Item>
          {/each}
        </Select.Content>
      </Select.Root>
      <input
        type="text"
        bind:value={pattern}
        placeholder="regex · case-insensitive · matches the whole row"
        spellcheck="false"
        class="h-6 min-w-64 flex-1 rounded-sm border border-border bg-background px-1 font-mono text-[11px]"
        aria-label="regex"
        data-testid="db-regex"
      />
      <span class="text-[11px] tabular-nums text-muted-foreground">{summary}</span>
    </div>
    {#if error}
      <p class="mb-2 font-mono text-[11px] text-bad">{error}</p>
    {/if}
    {#if !result}
      <p class="text-[12px] text-muted-foreground">Loading…</p>
    {:else if !result.rows.length}
      <p class="text-[12px] text-muted-foreground">No rows.</p>
    {:else}
      <div class="max-h-[560px] overflow-auto rounded-sm border border-border">
        <table class="w-full text-[11px]" data-testid="db-rows">
          <thead class="sticky top-0 bg-card">
            <tr class="border-b border-border">
              {#each result.columns as c, i (i)}
                <th class="px-2 py-0.5 text-left font-normal whitespace-nowrap text-muted-foreground">{c}</th>
              {/each}
            </tr>
          </thead>
          <tbody>
            {#each result.rows as row, i (i)}
              <tr class="border-b border-border/50">
                {#each row as cell, j (j)}
                  <td class="px-2 py-0.5 whitespace-nowrap tabular-nums {j === 0 ? 'text-muted-foreground' : ''}">{cell}</td>
                {/each}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </CardContent>
</Card>

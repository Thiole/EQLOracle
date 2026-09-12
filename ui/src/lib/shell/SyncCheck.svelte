<script lang="ts">
  // why: one chip saying whether the app's anchor files can still be
  // trusted. A dump is only usable while the log covers the span since it
  // was written -- if no "Outputfile Complete" for it is in this log,
  // nothing can say what changed, so it needs re-running. Click opens the
  // health check: what is stale and the command that fixes it.
  import { api, type SyncCheckDto, type SyncRowDto } from '$lib/tauri/api';
  import { copyText } from '$lib/clipboard';
  import { listen } from '$lib/tauri/invoke';
  import { status } from '$lib/stores/status';
  import CopyIcon from '@lucide/svelte/icons/copy';

  let open = $state(false);
  let panelEl: HTMLDivElement | undefined = $state();
  let buttonEl: HTMLButtonElement | undefined = $state();
  let check = $state<SyncCheckDto | null>(null);
  let note = $state<Record<string, string>>({});

  const CHIP: Record<string, { text: string; cls: string }> = {
    ok: { text: 'Ok', cls: 'text-good' },
    fix: { text: 'Fix', cls: 'text-caution' },
    missing: { text: 'Missing', cls: 'text-bad' },
    unsure: { text: 'Re-check', cls: 'text-caution' },
  };
  const DOT: Record<string, string> = {
    ok: 'text-good',
    fix: 'text-caution',
    missing: 'text-bad',
    unsure: 'text-caution',
    optional: 'text-muted-foreground',
    inferred: 'text-muted-foreground',
  };
  const MARK: Record<string, string> = {
    ok: '✓',
    fix: '!',
    missing: '✕',
    unsure: '?',
    optional: '?',
    inferred: '✓',
  };

  async function refresh() {
    check = (await api.getSyncCheck().catch(() => null)) ?? null;
  }

  function toggle() {
    open = !open;
    if (open) void refresh();
  }

  async function copy(r: SyncRowDto) {
    if (!r.command) return;
    note[r.kind] = (await copyText(r.command)) ? 'copied -- paste it in game' : 'copy failed';
  }

  function when(ms: number | null): string {
    if (ms == null) return '';
    const d = new Date(ms);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleString();
  }

  function onDocPointerDown(e: PointerEvent) {
    if (!open) return;
    const t = e.target as Node;
    if (panelEl?.contains(t) || buttonEl?.contains(t)) return;
    open = false;
  }

  // why: the toolbar mounts before the tail worker has named a file, and
  // a verdict computed then reports the log itself as missing -- the
  // worst primary row, so the whole chip went red until a click re-ran
  // it. Nothing is knowable until the tail has settled.
  const tail = $derived($status?.status);
  const ready = $derived(!!tail && !tail.backfilling);

  $effect(() => {
    // why: tracked on purpose -- re-ask whenever the tail's own state
    // changes, which is the moment the answer becomes knowable
    void tail?.file;
    void tail?.watching;
    void tail?.backfilling;
    void refresh();
  });

  $effect(() => {
    // why: pushed, not polled -- the backend emits the whole state the
    // moment the log carries an Outputfile Complete line, so this costs
    // nothing per tick and lands whether or not the panel is open
    let stop: (() => void) | undefined;
    void listen<SyncCheckDto>('sync-check', (e) => {
      check = e.payload;
    }).then((un) => {
      stop = un;
    });
    document.addEventListener('pointerdown', onDocPointerDown);
    return () => {
      stop?.();
      document.removeEventListener('pointerdown', onDocPointerDown);
    };
  });

  // why: null whenever the backend has nothing to say (mock IPC included)
  // -- the chip stays neutral rather than claiming a state it can't know
  const chip = $derived(ready && check ? (CHIP[check.overall] ?? CHIP.fix) : null);
  const primary = $derived(check?.rows.filter((r) => r.primary) ?? []);
  const secondary = $derived(check?.rows.filter((r) => !r.primary) ?? []);
</script>

<div class="relative">
  <button
    bind:this={buttonEl}
    type="button"
    class="flex items-center gap-1.5 text-[11px] text-muted-foreground hover:text-foreground"
    onclick={toggle}
    aria-expanded={open}
    data-testid="sync-check-chip"
  >
    <span>Sync Check:</span>
    <span class={chip ? chip.cls : 'text-muted-foreground'}>{chip ? chip.text : '--'}</span>
  </button>

  {#if open}
    <div
      bind:this={panelEl}
      class="absolute right-0 top-full z-50 mt-1 w-[min(30rem,calc(100vw-2rem))] rounded-sm border border-border bg-card p-3 shadow-md"
      data-testid="sync-check-panel"
    >
      {#if !ready}
        <p class="text-[11px] text-muted-foreground">Still reading the log -- nothing to report yet.</p>
      {:else if !check}
        <p class="text-[11px] text-muted-foreground">No sync information available.</p>
      {:else}
        <p class="mb-2 text-[11px] text-muted-foreground">
          A dump stays usable while the log still covers what happened since it was written.
          Anything the log can't account for is listed as needing a re-run.
        </p>
        {#each [{ title: 'Needed', rows: primary }, { title: 'Not required', rows: secondary }] as group (group.title)}
          {#if group.rows.length}
            <p class="mb-1 mt-2 text-[10px] uppercase tracking-wide text-muted-foreground">{group.title}</p>
            <ul class="flex flex-col gap-1.5">
              {#each group.rows as r (r.kind)}
                <li class="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
                  <span class="w-3 shrink-0 font-mono {DOT[r.status] ?? 'text-muted-foreground'}">{MARK[r.status] ?? '?'}</span>
                  <span class="min-w-24 shrink-0 text-[12px]">{r.label}</span>
                  <span class="flex-1 text-[11px] text-muted-foreground">{r.detail}</span>
                  {#if r.command && r.status !== 'ok' && r.status !== 'inferred'}
                    <button
                      type="button"
                      class="inline-flex shrink-0 items-center gap-1 rounded-sm border border-border px-1.5 py-0.5 font-mono text-[11px] hover:bg-accent hover:text-foreground"
                      title="Copy {r.command}"
                      onclick={() => copy(r)}
                    >
                      <CopyIcon class="size-3" />
                      {r.command}
                    </button>
                  {/if}
                  {#if note[r.kind]}
                    <span class="w-full pl-5 text-[10px] text-muted-foreground">{note[r.kind]}</span>
                  {/if}
                  {#if r.file}
                    <span class="w-full pl-5 font-mono text-[10px] text-muted-foreground">
                      {r.file}{r.modified_ms ? ` -- written ${when(r.modified_ms)}` : ''}
                    </span>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
        {/each}
        <button
          type="button"
          class="mt-3 text-[11px] text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
          onclick={refresh}
        >
          re-check
        </button>
      {/if}
    </div>
  {/if}
</div>

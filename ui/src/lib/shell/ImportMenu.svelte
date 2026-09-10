<script lang="ts">
  // why: the three `/outputfile` dumps the app reads, in one place -- copy
  // the command for the game, then import the file it wrote. A fresh dump
  // waits here until asked for; nothing pops up on launch.
  import { api, type OutputfileDto } from '$lib/tauri/api';
  import { copyText } from '$lib/clipboard';
  import { loadInventoryDump, reloadSpellbook } from '$lib/stores/character';

  let open = $state(false);
  let panelEl: HTMLDivElement | undefined = $state();
  let buttonEl: HTMLButtonElement | undefined = $state();
  let files = $state<OutputfileDto[] | null>(null);
  let note = $state<Record<string, string>>({});

  const LABELS: Record<string, string> = { inventory: 'Inventory', achievements: 'Achievements', spellbook: 'Spellbook' };

  async function refresh() {
    files = (await api.listOutputfiles().catch(() => null)) ?? [];
  }

  function toggle() {
    open = !open;
    if (open) void refresh();
  }

  async function copy(f: OutputfileDto) {
    note[f.kind] = (await copyText(f.command)) ? 'copied -- paste it in game' : 'copy failed';
  }

  async function doImport(f: OutputfileDto) {
    note[f.kind] = 'importing…';
    try {
      if (f.kind === 'inventory') {
        if (!f.file) throw new Error('no dump found');
        await loadInventoryDump(f.file);
        note[f.kind] = 'loaded into Gear';
      } else if (f.kind === 'achievements') {
        note[f.kind] = `${await api.importAchievements()} complete`;
      } else {
        const n = await api.importSpellbook();
        await reloadSpellbook();
        note[f.kind] = `${n} spells known`;
      }
    } catch (e) {
      note[f.kind] = e instanceof Error ? e.message : String(e);
    }
  }

  function onDocPointerDown(e: PointerEvent) {
    if (!open) return;
    const t = e.target as Node;
    if (panelEl?.contains(t) || buttonEl?.contains(t)) return;
    open = false;
  }

  $effect(() => {
    document.addEventListener('pointerdown', onDocPointerDown);
    return () => document.removeEventListener('pointerdown', onDocPointerDown);
  });

  const when = (ms: number | null) => (ms ? new Date(ms).toLocaleString() : 'not found');
</script>

<div class="relative">
  <button
    type="button"
    bind:this={buttonEl}
    onclick={toggle}
    class="rounded-md border border-border px-2 py-1 text-[11px] font-medium text-muted-foreground transition-colors hover:text-foreground"
  >
    Import
  </button>

  {#if open}
    <div
      bind:this={panelEl}
      class="absolute right-0 top-full z-50 mt-1 w-80 rounded-md border border-border bg-card p-2.5 text-[12px] shadow-lg"
      data-testid="import-menu"
    >
      {#if !files}
        <p class="text-[11px] text-muted-foreground">Looking beside Logs…</p>
      {:else if !files.length}
        <p class="text-[11px] text-muted-foreground">No backend in this session.</p>
      {:else}
        <div class="flex flex-col gap-2">
          {#each files as f (f.kind)}
            <div>
              <div class="flex items-center justify-between gap-2">
                <span class="font-medium text-foreground">{LABELS[f.kind] ?? f.kind}</span>
                <div class="flex shrink-0 gap-1">
                  <button
                    type="button"
                    title={f.command}
                    onclick={() => void copy(f)}
                    class="rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground hover:text-foreground"
                  >
                    copy command
                  </button>
                  <button
                    type="button"
                    disabled={!f.file}
                    onclick={() => void doImport(f)}
                    class="rounded border border-border px-1.5 py-0.5 text-[10px] text-muted-foreground hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    import
                  </button>
                </div>
              </div>
              <p class="text-[11px] text-muted-foreground">{f.file ?? 'no file yet'} · {when(f.modified_ms)}</p>
              {#if note[f.kind]}
                <p class="text-[11px]">{note[f.kind]}</p>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<script lang="ts">
  // why: turns "overlay isn't showing" into pasteable OS facts -- flags
  // decoded, the known-bad states called out, one click to copy the raw
  // JSON into a bug report
  import { onMount } from 'svelte';
  import { api, type OverlayDiagnosticsDto } from '$lib/tauri/api';
  import { Card, CardContent } from '$lib/components/ui/card';
  import { Button } from '$lib/components/ui/button';
  import { copyText } from '$lib/clipboard';

  let diag = $state<OverlayDiagnosticsDto | null>(null);
  let error = $state<string | null>(null);
  let loaded = $state(false);
  let copied = $state(false);

  onMount(() => {
    void refresh();
  });

  // why: the error IS the report when the call fails -- shown and
  // copyable, never a silently greyed button ("copy json is greyed out")
  async function refresh() {
    loaded = false;
    error = null;
    try {
      diag = (await api.getOverlayDiagnostics()) ?? null;
    } catch (e) {
      diag = null;
      error = String(e);
    }
    loaded = true;
    copied = false;
  }

  async function copyJson() {
    const payload = diag ? JSON.stringify(diag, null, 2) : (error ?? '');
    if (!payload) return;
    copied = await copyText(payload);
  }

  // why: the exact invisible-layered-window state the Windows reports
  // describe -- LAYERED set but attributes never applied
  function invisibleLayered(o: { win32: { ex_flags: string[]; layered_alpha: number | null } | null }): boolean {
    return !!o.win32 && o.win32.ex_flags.includes('LAYERED') && o.win32.layered_alpha === null;
  }

  function offEveryMonitor(o: { outer_x: number | null; outer_y: number | null; width: number | null; height: number | null }): boolean {
    if (!diag || o.outer_x == null || o.outer_y == null || o.width == null || o.height == null) return false;
    return !diag.monitors.some(
      (m) =>
        o.outer_x! < m.x + m.width && o.outer_x! + o.width! > m.x && o.outer_y! < m.y + m.height && o.outer_y! + o.height! > m.y,
    );
  }
</script>

<div class="flex flex-col gap-3">
  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      <div class="flex items-center justify-between gap-2">
        <h2 class="panel-title">overlay diagnostics</h2>
        <div class="flex gap-1.5">
          <Button variant="outline" size="sm" class="h-6 px-2 text-[11px]" onclick={refresh}>refresh</Button>
          <Button variant="outline" size="sm" class="h-6 px-2 text-[11px]" onclick={copyJson} disabled={!diag && !error}>
            {copied ? 'copied' : 'copy JSON'}
          </Button>
        </div>
      </div>
      <p class="mt-1 text-[11px] text-muted-foreground">
        What the OS itself reports for each open overlay window. For a "not showing" report: enable the overlay
        widget first, hit refresh, then copy JSON and paste it in the bug report.
      </p>
      {#if diag}
        <p class="mt-1.5 text-[11px]">
          <span class="text-muted-foreground">v{diag.version} · {diag.platform} · capability:</span>
          {diag.capability.capability}{diag.capability.reason ? ` (${diag.capability.reason})` : ''}
          <span class="text-muted-foreground">
            · {diag.monitors.length} monitor{diag.monitors.length === 1 ? '' : 's'}:
            {diag.monitors.map((m) => `${m.width}x${m.height}@${m.x},${m.y} x${m.scale}`).join(' | ')}
          </span>
        </p>
      {/if}
    </CardContent>
  </Card>

  <Card class="rounded-sm">
    <CardContent class="px-3 py-2.5">
      {#if !loaded}
        <p class="text-[11px] text-muted-foreground">Loading…</p>
      {:else if diag?.stalled}
        <p class="text-[11px] text-caution">{diag.stalled}</p>
      {:else if error}
        <p class="text-[11px] text-caution">Diagnostics call failed -- this error IS the bug report, copy it:</p>
        <p class="mt-1 font-mono text-[11px]">{error}</p>
      {:else if !diag}
        <p class="text-[11px] text-muted-foreground">Diagnostics unavailable (no backend in this session).</p>
      {:else if !diag.overlays.length}
        <p class="text-[11px] text-muted-foreground">
          No overlay windows open. Enable a widget in Overlay settings, then refresh.
        </p>
      {:else}
        <div class="flex flex-col gap-2">
          {#each diag.overlays as o (o.label)}
            <div class="rounded-sm border border-border px-2.5 py-1.5 text-[11px]">
              <div class="flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
                <span class="font-medium">{o.label}</span>
                <span class="text-muted-foreground">
                  tauri visible: {o.tauri_visible ?? '?'} · pos: {o.outer_x ?? '?'},{o.outer_y ?? '?'} · size:
                  {o.width ?? '?'}×{o.height ?? '?'}
                </span>
              </div>
              {#if o.win32}
                <div class="mt-0.5 flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
                  <span class="text-muted-foreground">win32:</span>
                  <span class="font-mono">{o.win32.ex_flags.join(' ') || '(no flags)'}</span>
                  <span class="text-muted-foreground">
                    visible={o.win32.visible} iconic={o.win32.iconic} alpha={o.win32.layered_alpha ?? 'UNSET'}
                    cloaked={o.win32.cloaked ?? '?'} rect={o.win32.rect.join(',')}
                  </span>
                </div>
              {/if}
              {#if invisibleLayered(o)}
                <p class="mt-0.5 text-caution">
                  LAYERED without attributes -- Windows will not render this window at all. This is the bug.
                </p>
              {/if}
              {#if offEveryMonitor(o)}
                <p class="mt-0.5 text-caution">Window sits outside every monitor -- it exists but is off-screen.</p>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
      {#if diag?.enable_trace.length}
        <p class="mt-2 text-[11px] text-muted-foreground">enable trace (newest last):</p>
        <ul class="mt-0.5 font-mono text-[11px]">
          {#each diag.enable_trace.slice(-30) as t, i (i)}
            <li>{t}</li>
          {/each}
        </ul>
      {/if}
    </CardContent>
  </Card>
</div>

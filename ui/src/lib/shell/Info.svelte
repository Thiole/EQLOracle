<script lang="ts">
  import { onMount } from 'svelte';
  import { Card, CardContent } from '$lib/components/ui/card';
  import { api } from '$lib/tauri/api';
  import { updateChannel, loadPreferences } from '$lib/stores/settings';
  import { openChangelog } from '$lib/stores/whatsnew';
  import { DISCORD_INVITE, DISCORD_PATH } from '$lib/links';

  // why: Info menu shows current version. Backend command (see
  // commands::get_app_version's doc for why not a raw
  // @tauri-apps/api/app::getVersion() call), no network round trip --
  // just what's actually installed. updateChannel is shown alongside
  // it -- "0.1.34" alone means nothing without knowing it's a Beta
  // build's rolling build number, not a real semver release (see
  // 3-release.yml's doc on why only the testing channel gets that).
  let version = $state<string | null>(null);
  onMount(() => {
    void loadPreferences();
    void api.getAppVersion().then((v) => (version = v));
  });
</script>

<div class="flex flex-col gap-3 p-3">
  <Card class="rounded-sm">
    <CardContent class="flex flex-col gap-3 px-3 py-3 text-[12px]">
      <div>
        <h2 class="panel-title mb-1">EQL Oracle</h2>
        <p class="text-muted-foreground">Thanks for using it -- more will land here over time.</p>
      </div>

      <div>
        <h3 class="mb-0.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Version</h3>
        <p class="text-muted-foreground">
          <span class="text-foreground">{version ?? '…'}</span>
          <span class="text-muted-foreground/70">({$updateChannel === 'beta' ? 'testing channel' : 'public channel'})</span>
          <button type="button" class="ml-2 text-[11px] text-brand-soft hover:text-primary hover:underline" onclick={openChangelog}>what's new →</button>
        </p>
      </div>

      <div>
        <h3 class="mb-0.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Source</h3>
        <p class="text-muted-foreground">github.com/Thiole/EQLOracle · eqloracle.com</p>
      </div>

      <div>
        <h3 class="mb-0.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Community</h3>
        <a
          class="inline-flex items-center gap-1.5 text-brand-soft hover:text-primary hover:underline"
          href={DISCORD_INVITE}
          target="_blank"
          rel="noopener"
        >
          <svg class="h-3.5 w-3.5 shrink-0" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d={DISCORD_PATH} /></svg>
          Join the Discord ↗
        </a>
        <p class="text-muted-foreground">Releases, testing builds, help, bug reports.</p>
      </div>

      <div>
        <h3 class="mb-0.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Contact</h3>
        <p class="text-muted-foreground">Discord: <span class="text-foreground">thiole</span></p>
        <p class="text-muted-foreground">In-game: <span class="text-foreground">Manipulator</span> on <span class="text-foreground">Rivervale</span></p>
      </div>

      <p class="text-[11px] italic text-muted-foreground">This page isn't filled out yet -- more to come.</p>
    </CardContent>
  </Card>
</div>

<script lang="ts">
  import { onMount } from 'svelte';
  import { Card, CardContent } from '$lib/components/ui/card';
  import { api } from '$lib/tauri/api';
  import { updateChannel, loadPreferences } from '$lib/stores/settings';

  // why: Spencer's own ask -- "i menu should show current version
  // information". A real backend command (see commands::get_app_version's
  // own doc for why not a raw @tauri-apps/api/app::getVersion() call),
  // no network round trip -- unlike checkForUpdate, this is just what's
  // actually installed right now. updateChannel is shown alongside it --
  // a bare "0.1.34" means nothing without knowing it's a Beta build's
  // own rolling build-number scheme, not a real semver release (see
  // 3-release.yml's own doc on why only the testing channel gets that).
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
        </p>
      </div>

      <div>
        <h3 class="mb-0.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Source</h3>
        <p class="text-muted-foreground">GitHub: <span class="italic">link coming soon</span></p>
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

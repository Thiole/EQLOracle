<script lang="ts">
  import { Button } from '$lib/components/ui/button';
  import { Checkbox } from '$lib/components/ui/checkbox';
  import { Card, CardHeader, CardTitle, CardContent } from '$lib/components/ui/card';
  import { api } from '$lib/tauri/api';
  import { status, refreshStatus } from '$lib/stores/status';
  import { saveProfile, setSaveProfile, loadPreferences } from '$lib/stores/settings';

  $effect(() => {
    void loadPreferences();
  });

  let picking = $state(false);
  let error = $state<string | null>(null);

  async function chooseFolder() {
    picking = true;
    error = null;
    try {
      const path = await api.pickLogDirectory();
      if (!path) return; // user cancelled -- not an error
      const result = await api.setLogDirectory(path);
      status.set(result);
    } catch (e) {
      error = String(e);
    } finally {
      picking = false;
    }
  }
</script>

<div class="flex min-h-screen items-center justify-center p-8">
  <Card class="max-w-md">
    <CardHeader>
      <CardTitle>Find your install folder</CardTitle>
    </CardHeader>
    <CardContent class="space-y-4 text-sm text-muted-foreground">
      <p>
        Point EQL Oracle at your EverQuest Legends install folder -- the one that directly contains
        <code class="rounded bg-muted px-1 py-0.5 text-foreground">Logs</code>
        (not the <code class="rounded bg-muted px-1 py-0.5 text-foreground">Logs</code> folder itself). It watches
        whichever <code class="rounded bg-muted px-1 py-0.5 text-foreground">eqlog_&lt;Character&gt;_&lt;Server&gt;.txt</code>
        file was written to most recently, replays what it's already said, then keeps parsing live as the game writes
        more -- and the install folder itself is also where
        <code class="rounded bg-muted px-1 py-0.5 text-foreground">/outputfile inventory</code>
        writes its dump, which needs to be reachable from the same place.
      </p>
      <label class="flex items-center gap-1.5 text-[12px] text-foreground">
        <Checkbox checked={$saveProfile} onCheckedChange={(v: boolean) => setSaveProfile(v)} />
        save your profile across launches
      </label>
      <p class="text-[11px]">
        Off (default): every launch replays the whole log and figures out your classes fresh from what it actually
        sees. On: also remembers your last-confirmed classes between launches, as a fallback until a new session's
        own replay reconfirms them itself. Change this anytime in Settings.
      </p>
      <Button onclick={chooseFolder} disabled={picking}>{picking ? 'Choosing…' : 'Choose folder…'}</Button>
      {#if error}
        <p class="text-destructive">{error}</p>
      {/if}
    </CardContent>
  </Card>
</div>

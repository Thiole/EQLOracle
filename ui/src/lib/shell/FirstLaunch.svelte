<script lang="ts">
  import { Button } from '$lib/components/ui/button';
  import { Card, CardHeader, CardTitle, CardContent } from '$lib/components/ui/card';
  import { api } from '$lib/tauri/api';
  import { status, refreshStatus } from '$lib/stores/status';

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
        Point EQLP Companion at your EverQuest Legends install folder -- the one that directly contains
        <code class="rounded bg-muted px-1 py-0.5 text-foreground">Logs</code>
        (not the <code class="rounded bg-muted px-1 py-0.5 text-foreground">Logs</code> folder itself). It watches
        whichever <code class="rounded bg-muted px-1 py-0.5 text-foreground">eqlog_&lt;Character&gt;_&lt;Server&gt;.txt</code>
        file was written to most recently, replays what it's already said, then keeps parsing live as the game writes
        more -- and the install folder itself is also where
        <code class="rounded bg-muted px-1 py-0.5 text-foreground">/outputfile inventory</code>
        writes its dump, which needs to be reachable from the same place.
      </p>
      <Button onclick={chooseFolder} disabled={picking}>{picking ? 'Choosing…' : 'Choose folder…'}</Button>
      {#if error}
        <p class="text-destructive">{error}</p>
      {/if}
    </CardContent>
  </Card>
</div>

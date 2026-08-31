<script lang="ts">
  import * as Select from '$lib/components/ui/select';
  import { Button } from '$lib/components/ui/button';
  import { Checkbox } from '$lib/components/ui/checkbox';
  import { Card, CardHeader, CardTitle, CardContent } from '$lib/components/ui/card';
  import { api } from '$lib/tauri/api';
  import { status, refreshStatus } from '$lib/stores/status';
  import { saveProfile, setSaveProfile, theme, setTheme, loadPreferences } from '$lib/stores/settings';
  import { THEME_CATEGORIES, THEME_SWATCHES, themeName } from '$lib/settings/themes';

  $effect(() => {
    void loadPreferences();
  });

  let picking = $state(false);
  let error = $state<string | null>(null);
  let manualPath = $state('');

  async function applyPath(path: string) {
    error = null;
    try {
      const result = await api.setLogDirectory(path);
      status.set(result);
    } catch (e) {
      error = String(e);
    }
  }

  async function chooseFolder() {
    picking = true;
    error = null;
    try {
      const path = await api.pickLogDirectory();
      if (!path) return; // user cancelled -- not an error
      await applyPath(path);
    } catch (e) {
      error = String(e);
    } finally {
      picking = false;
    }
  }
</script>

{#snippet themeSwatch(slug: string)}
  <span class="flex shrink-0 gap-[3px]">
    {#each THEME_SWATCHES[slug] ?? [] as color, i (i)}
      <span class="size-2.5 rounded-full border border-black/20" style="background-color: {color}"></span>
    {/each}
  </span>
{/snippet}

<!-- min-h-full, not min-h-screen: rendered below the always-present
     Toolbar row now, so "screen" would force a needless scrollbar -->
<div class="flex min-h-full items-center justify-center p-8">
  <Card class="max-w-md">
    <CardHeader>
      <CardTitle>Find your install folder</CardTitle>
    </CardHeader>
    <CardContent class="space-y-4 text-sm text-muted-foreground">
      <label class="flex items-center gap-2 text-[12px] text-foreground">
        <span class="w-16 shrink-0 text-muted-foreground">theme</span>
        <Select.Root type="single" value={$theme} onValueChange={(v) => v && setTheme(v)}>
          <Select.Trigger class="h-7 flex-1 text-[12px]">
            <span class="flex items-center gap-2">
              {@render themeSwatch($theme)}
              {themeName($theme)}
            </span>
          </Select.Trigger>
          <Select.Content>
            {#each THEME_CATEGORIES as cat (cat.label)}
              <Select.Group>
                <Select.GroupHeading class="text-[10px] tracking-[0.1em] text-muted-foreground uppercase">{cat.label}</Select.GroupHeading>
                {#each cat.themes as t (t.slug)}
                  <Select.Item value={t.slug}>
                    <span class="flex items-center gap-2">
                      {@render themeSwatch(t.slug)}
                      {t.name}
                    </span>
                  </Select.Item>
                {/each}
              </Select.Group>
            {/each}
          </Select.Content>
        </Select.Root>
      </label>
      <p class="text-[11px]">Try on a look before you get started -- this applies live and carries over into the app once you're in.</p>

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
      <!-- why: real Windows first-launch report -- the native dialog can
           open behind the window (or its callback never resolve), which
           reads as "the button does nothing" and hard-blocks setup with
           no other way in. Pasting the path is the way in that cannot
           break; picking Logs itself is auto-repaired backend-side. -->
      <div class="flex items-center gap-2">
        <input
          class="h-8 flex-1 rounded-md border border-border bg-background px-2 text-[12px] text-foreground"
          placeholder="…or paste the install folder path here"
          bind:value={manualPath}
        />
        <Button variant="outline" disabled={!manualPath.trim()} onclick={() => void applyPath(manualPath.trim())}>Use</Button>
      </div>
      {#if error}
        <p class="text-destructive">{error}</p>
      {/if}
    </CardContent>
  </Card>
</div>

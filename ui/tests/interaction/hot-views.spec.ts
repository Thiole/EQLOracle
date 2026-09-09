import { test, expect } from '@playwright/test';

// why: a module is a view of hot store state -- a parse-tick must reach
// the Group Buffs card without leaving and re-entering Overview. The
// reported bug: "if i buff it doesn't load until i reload the tab".
test('group buffs card updates on a parse-tick without a remount', async ({ page }) => {
  await page.goto('/');
  await page.locator('[data-module="overview"]').click();
  const card = page.locator('[data-slot="card"]', { hasText: 'Group Buffs' });
  await expect(card).toBeVisible();
  await expect(card).not.toContainText('Loading…');
  await expect(card).not.toContainText('Hot Test Line');

  // why: the backend "changes" -- the next fetch carries a new missing line
  await page.evaluate(async () => {
    const m = await import('/src/lib/tauri/api.ts');
    const orig = m.api.getGroupBuffs;
    m.api.getGroupBuffs = async () => {
      const d = (await orig()) ?? { good: true, upgrades: 0, my_classes: [], party: [], rows: [], innates: [] };
      return { ...d, innates: [...d.innates, { line: 'Hot Test Line', label: 'hot', best_spell: 'x', best_level: 1, active: null }] };
    };
  });
  await page.evaluate(() =>
    (window as unknown as { __mockEmit: (e: string, p: unknown) => void }).__mockEmit('parse-tick', {
      status: { log_dir: null, file: null, character: null, server: null, watching: true, tail_status: 'grew', backfilling: false, pets_attributed: 0 },
      counts: { total: 1, matched: 1, unmatched: 0, headerless: 0, blank: 0, by_kind: { spell: 1 } },
      recent: [{ kind: 'spell', rule_id: 'test', text: 'test' }],
    }),
  );
  await expect(card).toContainText('Hot Test Line', { timeout: 3000 });
});

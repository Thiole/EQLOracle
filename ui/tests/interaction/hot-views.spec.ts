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

// why: reported real -- launching into Combat showed the fight tree at the
// full width of the window with the data pushed below it. A default
// 1040px window at 110% zoom is 945 CSS px, which fell under Tailwind's
// lg (1024) and collapsed the two panes into one column, tree first.
test.describe('combat panes', () => {
  const tree = '[data-testid="fight-tree"]';
  const details = '[data-testid="combat-details"]';

  test('both panes sit side by side at 945 CSS px, tree on the left', async ({ page }) => {
    await page.setViewportSize({ width: 945, height: 700 });
    await page.goto('/');
    await page.locator('[data-module="combat"]').click();
    await page.waitForSelector(tree);
    const treeBox = await page.locator(tree).boundingBox();
    expect(treeBox, 'the tree renders').not.toBeNull();
    // why: the whole complaint in one number -- it must not own the window,
    // and it must not spill past the 340px column it was given either
    expect(treeBox!.width).toBeLessThanOrEqual(340);
    const detailsBox = await page.locator(details).boundingBox();
    expect(detailsBox, 'the data pane renders').not.toBeNull();
    expect(detailsBox!.x).toBeGreaterThan(treeBox!.x + treeBox!.width - 1);
    expect(detailsBox!.y).toBeLessThan(treeBox!.y + 200);
  });

  test('under the breakpoint the data comes first and the tree is a rail', async ({ page }) => {
    await page.setViewportSize({ width: 760, height: 640 });
    await page.goto('/');
    await page.locator('[data-module="combat"]').click();
    const rail = page.locator('[data-testid="fight-tree-rail"]');
    await page.waitForSelector('[data-testid="fight-tree-rail"]');
    const collapsed = await rail.boundingBox();
    expect(collapsed, 'the rail renders').not.toBeNull();
    // why: a labelled spine, nothing more, until asked for
    expect(collapsed!.width).toBeLessThan(60);
    expect(collapsed!.x).toBe(0);
    // why: the data now owns the width and starts at the top
    const detailsBox = await page.locator(details).boundingBox();
    expect(detailsBox!.y).toBeLessThan(collapsed!.y + 120);
    // why: the whole content area minus the spine -- the data owns it now
    expect(detailsBox!.width).toBeGreaterThan(500);

    // why: hover slides it out over the page
    await rail.hover();
    await expect
      .poll(async () => (await rail.boundingBox())!.width, { timeout: 2000 })
      .toBeGreaterThan(300);
    // why: and it goes back when the pointer leaves
    await page.mouse.move(700, 400);
    await expect
      .poll(async () => (await rail.boundingBox())!.width, { timeout: 2000 })
      .toBeLessThan(60);
  });
});

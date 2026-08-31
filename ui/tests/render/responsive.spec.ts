import { test, expect, type Page } from '@playwright/test';

// Every module × a realistic window-size matrix. Three invariants per
// cell, all structural (no screenshot baselines -- those live in
// layout.spec.ts's own fixme until per-engine baselines are curated):
//   1. the module mounts something (main isn't empty),
//   2. no document-level horizontal overflow,
//   3. no uncaught page error while mounting or resizing.
// Runs against the mock harness, where most modules render their empty/
// null states -- that's a feature: an empty state that crashes or
// overflows is exactly the first-run experience a new user gets.

const MODULES = [
  'overview',
  'combat',
  'social',
  'character',
  'endgame',
  'tradeskill',
  'gamedata',
  'maps',
  'overlay',
  'info',
  'debug',
  'settings',
] as const;

// 760x480 is tauri.conf.json's own minWidth/minHeight -- the real floor.
// The rest are the common desktop sizes a Windows player actually runs.
const VIEWPORTS: Array<[number, number]> = [
  [760, 480],
  [1024, 640],
  [1366, 768],
  [1920, 1080],
  [2560, 1440],
];

async function horizontalOverflow(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const bad: string[] = [];
    const doc = document.documentElement;
    if (doc.scrollWidth > doc.clientWidth + 1) {
      bad.push(`document: scrollWidth ${doc.scrollWidth} > clientWidth ${doc.clientWidth}`);
    }
    return bad;
  });
}

for (const [w, h] of VIEWPORTS) {
  test(`every module renders without overflow or errors at ${w}x${h}`, async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));

    await page.setViewportSize({ width: w, height: h });
    await page.goto('/');
    await page.waitForSelector('[data-app-root]');
    // Configured state comes from the fixture snapshot; the sidebar is
    // the readiness signal for the full shell.
    await page.waitForSelector('[data-slot="sidebar"]');

    for (const mod of MODULES) {
      await page.click(`[data-module="${mod}"]`);
      // One frame for the module to mount; anything slower than this to
      // first paint is itself a bug worth failing on.
      await page.waitForTimeout(150);

      const mounted = await page.evaluate(() => {
        const main = document.querySelector('main');
        return !!main && main.children.length > 0;
      });
      expect(mounted, `${mod} at ${w}x${h}: <main> mounted nothing`).toBe(true);

      const overflow = await horizontalOverflow(page);
      expect(overflow, `${mod} at ${w}x${h}: ${overflow.join('; ')}`).toEqual([]);

      expect(errors, `${mod} at ${w}x${h}: ${errors.join('; ')}`).toEqual([]);
    }
  });
}

// One mid-size pass that saves a full-page screenshot per module --
// review artifacts, not baselines: they land in test-results/ for a
// human (or agent) to actually look at, and never gate the suite.
test('capture per-module screenshots for review', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.goto('/');
  await page.waitForSelector('[data-slot="sidebar"]');
  for (const mod of MODULES) {
    await page.click(`[data-module="${mod}"]`);
    await page.waitForTimeout(200);
    await page.screenshot({
      path: testInfo.outputPath(`module-${mod}.png`),
      fullPage: false,
    });
  }
});

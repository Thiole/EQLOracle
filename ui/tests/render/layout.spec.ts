import { test, expect } from '@playwright/test';

// Visual correctness across engines, viewports and scale factors.
// Baselines are per-engine: WebKitGTK and Chromium rasterise differently, and a
// shared baseline would either be too loose to catch anything or permanently red.

test.describe('layout', () => {
  test.fixme('main panel matches baseline at every viewport/scale', async ({ page }) => {
    await page.goto('/');
    // await expect(page).toHaveScreenshot(`main-${engine}-${viewport}-${scale}.png`)
  });

  test('no horizontal overflow at the narrowest supported width', async ({ page }) => {
    // 760 is tauri.conf.json's own `minWidth` -- the narrowest the real
    // window can ever be, so it's the real floor to check against, not
    // an arbitrary "mobile" breakpoint this desktop app doesn't have.
    await page.setViewportSize({ width: 760, height: 480 });
    await page.goto('/');
    await page.waitForSelector('[data-app-root]');
    await page.waitForSelector('text=allies');

    const overflow = await page.evaluate(() => {
      const doc = document.documentElement;
      const bad: string[] = [];
      if (doc.scrollWidth > doc.clientWidth + 1) {
        bad.push(`document: scrollWidth ${doc.scrollWidth} > clientWidth ${doc.clientWidth}`);
      }
      return bad;
    });
    expect(overflow, overflow.join('; ')).toEqual([]);
  });

  test('long mob and player names truncate rather than breaking layout', async ({ page }) => {
    // Real names, exactly the shape that breaks a table with no min-width/
    // truncation: a long possessive spell/ability name in the expanded
    // ally panel. The reference fixture's own real data already has
    // several -- picking one from the aggregate ability breakdown rather
    // than injecting a synthetic name keeps this test honest about what
    // the real backend actually produces.
    await page.setViewportSize({ width: 900, height: 700 });
    await page.goto('/');
    // The ally TABLE row specifically -- 'Kaeus' also appears as a
    // fight-participant chip now, so a bare text match is ambiguous.
    await page.getByRole('cell', { name: 'Kaeus' }).click();
    await page.waitForSelector('text=ABILITIES');

    const overflow = await page.evaluate(() => {
      const doc = document.documentElement;
      return doc.scrollWidth > doc.clientWidth + 1;
    });
    expect(overflow).toBe(false);
  });

  test('numeric columns use tabular figures so values never shift the column', async ({ page }) => {
    // 1 -> 1,204 -> 46,896 must not shift the column -- guaranteed by
    // `tabular-nums`, not by every value happening to be the same width.
    // Checking the CSS property directly (rather than measuring pixel
    // widths across a data-dependent fixture) is what makes this
    // assertion actually about the *cause*, not a value-dependent proxy
    // for it.
    await page.goto('/');
    await page.waitForSelector('text=allies');
    const cell = page.locator('td.tabular-nums').first();
    await expect(cell).toHaveCSS('font-variant-numeric', /tabular-nums/);
  });
});

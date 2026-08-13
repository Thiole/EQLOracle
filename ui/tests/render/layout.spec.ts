import { test } from '@playwright/test';

// Visual correctness across engines, viewports and scale factors.
// Baselines are per-engine: WebKitGTK and Chromium rasterise differently, and a
// shared baseline would either be too loose to catch anything or permanently red.

test.describe('layout', () => {
  test.fixme('main panel matches baseline at every viewport/scale', async ({ page }) => {
    await page.goto('/');
    // await expect(page).toHaveScreenshot(`main-${engine}-${viewport}-${scale}.png`)
  });

  test.fixme('no horizontal overflow at the narrowest supported width', async () => {
    // scrollWidth <= clientWidth on document and every panel.
  });

  test.fixme('long mob and player names truncate rather than breaking layout', async () => {
    // Real names from the fixture: "Innoruuk, the Prince of Hate",
    // "Garrison's Mighty Mana Shock X". These are the ones that break tables.
  });

  test.fixme('numeric columns stay aligned as values change width', async () => {
    // 1 -> 1,204 -> 46,896 must not shift the column.
  });
});

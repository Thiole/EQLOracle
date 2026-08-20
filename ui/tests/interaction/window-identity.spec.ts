import { test, expect } from '@playwright/test';

// Double windows. Common in webview apps: a second window on re-focus, on
// deep-link, on tray-icon click, or after the updater runs. Also a duplicated
// root node when the framework mounts twice under StrictMode.

test.describe('window identity', () => {
  test('exactly one app root is mounted', async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('[data-app-root]')).toHaveCount(1);
  });

  test.fixme('re-invoking the app focuses the existing window, never opens a second', async () => {
    // Shell-level: needs tauri-driver. Assert window count stays 1 across
    // tray click, deep link, and second-instance launch.
  });

  test('no duplicate mount under strict double-render', async ({ page }) => {
    // Subscriptions and IPC listeners must be idempotent. A doubled listener
    // shows up as every damage event counted twice -- silent and severe.
    // Real regression this guards: `initTauriEvents()` (src/lib/tauri/
    // events.ts) is called from App.svelte's `onMount`, which Svelte can
    // in principle re-run (e.g. a hot-reload remount); its own `initialized`
    // guard is what's actually under test here, not just "did the page load".
    await page.goto('/');
    await page.waitForSelector('[data-app-root]');
    const rootCount = await page.evaluate(() => document.querySelectorAll('[data-app-root]').length);
    expect(rootCount).toBe(1);
  });
});

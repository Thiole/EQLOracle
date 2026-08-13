import { test } from '@playwright/test';

// Double windows. Common in webview apps: a second window on re-focus, on
// deep-link, on tray-icon click, or after the updater runs. Also a duplicated
// root node when the framework mounts twice under StrictMode.

test.describe('window identity', () => {
  test.fixme('exactly one app root is mounted', async ({ page }) => {
    await page.goto('/');
    // expect(await page.locator('[data-app-root]').count()).toBe(1)
  });

  test.fixme('re-invoking the app focuses the existing window, never opens a second', async () => {
    // Shell-level: needs tauri-driver. Assert window count stays 1 across
    // tray click, deep link, and second-instance launch.
  });

  test.fixme('no duplicate mount under strict double-render', async () => {
    // Subscriptions and IPC listeners must be idempotent. A doubled listener
    // shows up as every damage event counted twice -- silent and severe.
  });
});

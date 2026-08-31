import { test, expect } from '@playwright/test';

// The Windows-frameless custom title bar (Toolbar.svelte). The mock
// harness reports custom_titlebar: true (mock.ts's get_ui_shell) so this
// chrome is exercised here even though Linux dev builds run decorated.
// What the harness can't see -- real dragging, real minimize -- is
// shell-level (tauri-driver, tests/README.md's own split).

test.describe('custom title bar', () => {
  test('window controls render with accessible names', async ({ page }) => {
    await page.goto('/');
    await page.waitForSelector('[data-testid="window-controls"]');
    for (const name of ['Minimize', 'Maximize', 'Close']) {
      await expect(page.getByRole('button', { name })).toBeVisible();
    }
  });

  test('a drag region exists and carries the tauri attribute', async ({ page }) => {
    await page.goto('/');
    const region = page.locator('[data-testid="titlebar-drag-region"]');
    await expect(region).toHaveAttribute('data-tauri-drag-region', '');
  });

  test('title bar renders before the app is configured', async ({ page }) => {
    // A frameless first-launch window without controls would be
    // unclosable -- the toolbar must not be gated on configured state.
    // The mock fixture always reports configured, so assert the
    // structural guarantee instead: the toolbar is OUTSIDE the
    // configured-only branch (it precedes the sidebar in the DOM and
    // renders even while status is null on first paint).
    await page.goto('/');
    await page.waitForSelector('header');
    const headerFirst = await page.evaluate(() => {
      const root = document.querySelector('[data-app-root]');
      return root?.firstElementChild?.tagName.toLowerCase();
    });
    expect(headerFirst).toBe('header');
  });

  test('clicking window controls never throws in the harness', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(e.message));
    await page.goto('/');
    await page.waitForSelector('[data-testid="window-controls"]');
    await page.getByRole('button', { name: 'Minimize' }).click();
    await page.getByRole('button', { name: 'Maximize' }).click();
    // Close is mock-guarded too, but click it last anyway.
    await page.getByRole('button', { name: 'Close' }).click();
    expect(errors).toEqual([]);
  });
});

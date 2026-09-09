import { test, expect } from '@playwright/test';

// why: Debug > Parsed browses every in-memory table -- the events table
// renders from the fixture, and switching tables swaps the columns
test('debug db search browses events and switches tables', async ({ page }) => {
  await page.goto('/');
  await page.locator('[data-module="debug"]').click();
  const rows = page.getByTestId('db-rows');
  await expect(rows).toBeVisible();
  await expect(rows.locator('thead')).toContainText('actor');
  expect(await rows.locator('tbody tr').count()).toBeGreaterThan(0);

  await page.getByTestId('db-table').selectOption('encounters');
  await expect(rows.locator('thead')).toContainText('target', { timeout: 3000 });
  await expect(rows.locator('thead')).not.toContainText('actor');
});

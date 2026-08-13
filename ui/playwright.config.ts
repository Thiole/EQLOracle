import { defineConfig, devices } from '@playwright/test';

const [w, h] = (process.env.EQLP_VIEWPORT ?? '1280x800').split('x').map(Number);
const scale = Number(process.env.EQLP_SCALE ?? '1');

export default defineConfig({
  testDir: './tests',
  forbidOnly: !!process.env.CI,
  retries: 0, // A flaky UI test is a bug report, not something to paper over.
  reporter: [['html', { outputFolder: 'playwright-report' }], ['list']],
  use: {
    baseURL: 'http://localhost:5173',
    viewport: { width: w, height: h },
    deviceScaleFactor: scale,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  expect: {
    // Font rasterisation differs between engines; geometry does not.
    // Keep this tight — a loose threshold hides real layout drift.
    toHaveScreenshot: { maxDiffPixelRatio: 0.01 },
  },
  projects: [
    // Blocking: Linux ships WebKitGTK, so this is the engine that matters.
    { name: 'webkit', use: { ...devices['Desktop Safari'] } },
    // Advisory: catches Chromium-only regressions early, never gates.
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
  ],
  webServer: {
    command: 'npm run dev:mock',
    url: 'http://localhost:5173',
    reuseExistingServer: !process.env.CI,
  },
});

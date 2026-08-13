import { test, expect } from '@playwright/test';

// Cursor/hit-target misalignment. The symptom users report is "I clicked the
// button and nothing happened" or "I had to click slightly above it".
//
// Root causes seen in the wild: CSS transforms on an ancestor, deviceScaleFactor
// rounding, a transparent overlay capturing pointer events, and scroll offset
// not accounted for in a custom drag handler.
//
// The check: for every interactive element, the point at its visual centre must
// resolve to that element via elementFromPoint. Coordinates are ground truth;
// what the DOM thinks is on top is the thing under test.

test.describe('hit testing', () => {
  test.fixme('every interactive element receives clicks at its visual centre', async ({ page }) => {
    await page.goto('/');
    // for each [role=button],[role=tab],input,select,a:
    //   box = await el.boundingBox()
    //   hit = await page.evaluate(([x,y]) => document.elementFromPoint(x,y), centre(box))
    //   expect(hit).toBe(el)  // not an ancestor, not an overlay
  });

  test.fixme('hit targets survive deviceScaleFactor 1.25 and 2', async () => {
    // Fractional scaling is where rounding errors surface. Same assertion,
    // driven by the EQLP_SCALE matrix axis.
  });

  test.fixme('no invisible element intercepts pointer events', async () => {
    // A zero-opacity or zero-size overlay with pointer-events:auto is the
    // classic cause of dead clicks. Assert nothing above the fold has
    // pointer-events enabled while being visually absent.
  });

  test.fixme('drag on a resizable panel tracks the cursor without drift', async () => {
    // Move in N steps, assert the panel edge follows within 1px at every step.
    // Accumulated drift means the handler is using the wrong coordinate space.
  });
});

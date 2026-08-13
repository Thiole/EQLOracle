import { test } from '@playwright/test';

// Overlap and stacking. "Panels on top of each other" is the single most common
// complaint about webview meters.

test.describe('z-order and overlap', () => {
  test.fixme('no two non-modal panels have intersecting bounding boxes', async () => {
    // Pairwise rect intersection over all [data-panel]. Cheap and catches a lot.
  });

  test.fixme('modal and dropdown layers render above all panels', async () => {
    // Assert via elementFromPoint at the overlap, not via computed z-index --
    // stacking contexts make z-index a liar.
  });

  test.fixme('tooltip never renders offscreen', async () => {
    // Trigger near each viewport edge; assert the rect stays inside.
  });

  test.fixme('scrolled content does not paint over sticky headers', async () => {});
});

import { test } from '@playwright/test';

// Field-level correctness against a known fixture. The UI must show what the
// parser produced -- no rounding drift, no stale values, no silent blanks.

test.describe('field values', () => {
  test.fixme('displayed DPS matches the session crate for the same fixture+timestamp', async () => {
    // Golden: run eqlp-session over fixture at t, compare against the DOM.
    // This is the test that catches the UI quietly disagreeing with the engine.
  });

  test.fixme('unattributable damage is shown as contested, never split silently', async () => {
    // Uses the dual-charm abhorrent window. See docs -- attribution is
    // impossible there, and the UI must say so rather than pick a name.
  });

  test.fixme('unknown TTK renders as unknown, not as zero or infinity', async () => {});

  test.fixme('stale values clear when an encounter ends', async () => {
    // A fight that goes quiet must not keep showing its last DPS.
  });
});

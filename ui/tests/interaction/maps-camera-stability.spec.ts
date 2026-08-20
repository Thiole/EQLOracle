import { test, expect } from '@playwright/test';
import type { MapFileDto } from '../../src/lib/tauri/api';

// Real regression this guards: MapViewer.svelte's scene-build effect used
// to depend (directly or transitively) on things a real `parse-tick`
// updates every few seconds -- `lastLocation` (read reactively) and, once
// the NPC overlay landed, `npcMarkers` too. Either one changing tore down
// and rebuilt the whole Three.js scene, throwing the camera back to its
// default framing and making the map unusable to actually pan/zoom
// around in while playing live. Fixed by splitting the scene build into
// its own effect (deps: map/zone only) with two small side effects for
// the "you are here" marker and the NPC overlay that move/rebuild just
// their own mesh in place.
//
// Exercised against the isolated `MapViewerHarness` fixture, not the
// whole app+Tauri-mock plumbing: MapViewer takes its geometry as plain
// props and reads one store, so it's cheap to drive directly the same
// way a real parse-tick would.

const squareRoom: MapFileDto = {
  lines: [
    { a: [-100, -100, 0], b: [100, -100, 0], color: [128, 128, 128] },
    { a: [100, -100, 0], b: [100, 100, 0], color: [128, 128, 128] },
    { a: [100, 100, 0], b: [-100, 100, 0], color: [128, 128, 128] },
    { a: [-100, 100, 0], b: [-100, -100, 0], color: [128, 128, 128] },
  ],
  markers: [{ pos: [90, 0, 0], color: [0, 255, 0], size: 3, label: 'to_West_Commonlands' }],
};

const otherRoom: MapFileDto = {
  lines: [
    { a: [-500, -500, 200], b: [500, -500, 200], color: [200, 40, 40] },
    { a: [500, -500, 200], b: [500, 500, 200], color: [200, 40, 40] },
  ],
  markers: [],
};

declare global {
  interface Window {
    __harness: {
      setMap: (m: MapFileDto) => void;
      setZone: (z: string) => void;
      setNpcMarkers: (n: unknown[]) => void;
      setZoneContext: (c: unknown) => void;
      setLastLocation: (loc: unknown) => void;
    };
  }
}

test.describe('Maps camera stability', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/tests/fixtures/mapviewer-harness.html');
    await page.waitForSelector('canvas');
    await page.evaluate((m) => window.__harness.setMap(m), squareRoom);
    // let the scene build and the render loop settle
    await page.waitForTimeout(300);
  });

  async function orbit(page: import('@playwright/test').Page) {
    const box = await page.locator('canvas').boundingBox();
    if (!box) throw new Error('canvas not found');
    const cx = box.x + box.width / 2;
    const cy = box.y + box.height / 2;
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await page.mouse.move(cx + 120, cy + 60, { steps: 8 });
    await page.mouse.up();
    // why: OrbitControls' damping decays the post-drag residual velocity
    // exponentially (dampingFactor 0.08/frame) -- it never truly stops,
    // just gets small. Long enough that it's below float/pixel precision,
    // not the arbitrary "feels done" 300ms a human would use.
    await page.waitForTimeout(3000);
  }

  test('a real parse-tick style lastLocation/zoneContext update does not reset the camera', async ({ page }) => {
    await orbit(page);
    const before = await page.locator('canvas').screenshot();

    for (let i = 0; i < 3; i++) {
      await page.evaluate(
        (i) =>
          window.__harness.setLastLocation({
            ts_ms: Date.now() + i,
            x: i * 10,
            y: i * 10,
            z: 0,
            zone: 'Unrelated Zone', // deliberately non-matching -- exercises the store update path either way
            map_zones: [],
          }),
        i,
      );
      await page.evaluate(
        (i) => window.__harness.setZoneContext({ current: 'Testzone', previous: `Prior Zone ${i}`, teleport_landing: null, current_map_zones: [] }),
        i,
      );
      await page.waitForTimeout(150);
    }

    const after = await page.locator('canvas').screenshot();
    expect(after.equals(before), 'camera view must be pixel-identical after simulated parse-ticks').toBe(true);
  });

  test('toggling the NPC overlay does not reset the camera', async ({ page }) => {
    await orbit(page);
    const before = await page.locator('canvas').screenshot();

    await page.evaluate(() => window.__harness.setNpcMarkers([{ name: 'Test Mob', x: 10, y: 10, z: null }]));
    await page.waitForTimeout(200);
    await page.evaluate(() => window.__harness.setNpcMarkers([]));
    await page.waitForTimeout(200);

    const after = await page.locator('canvas').screenshot();
    expect(after.equals(before), 'camera view must be pixel-identical after an NPC-overlay toggle').toBe(true);
  });

  test('a genuinely new /loc reading for the matching zone DOES soft-pan the camera', async ({ page }) => {
    // why: the counterpart to the stability tests above -- proving the new
    // "soft move to center on my /loc" feature actually fires, not just
    // that it correctly stays silent for non-matching/duplicate updates.
    const before = await page.locator('canvas').screenshot();

    await page.evaluate(() =>
      window.__harness.setLastLocation({
        ts_ms: Date.now(),
        x: 5000,
        y: -5000,
        z: 100,
        zone: 'testzone',
        map_zones: ['testzone'], // real resolution matches the currently-loaded map
      }),
    );
    // pan duration (700ms) plus damping settle
    await page.waitForTimeout(1200);

    const after = await page.locator('canvas').screenshot();
    expect(after.equals(before), 'a real matching /loc reading should visibly pan the camera').toBe(false);
  });

  test('a duplicate /loc reading (same ts_ms) does not re-pan', async ({ page }) => {
    const loc = { ts_ms: 999_000, x: 5000, y: -5000, z: 100, zone: 'testzone', map_zones: ['testzone'] };
    await page.evaluate((l) => window.__harness.setLastLocation(l), loc);
    await page.waitForTimeout(1200); // let the real pan finish and settle

    const settled = await page.locator('canvas').screenshot();
    // why: same ts_ms, same everything -- re-sending it (a real parse-tick
    // re-fetching the *same* last known /loc) must not restart the pan
    // animation or otherwise touch the camera a second time.
    await page.evaluate((l) => window.__harness.setLastLocation({ ...l }), loc);
    await page.waitForTimeout(300);

    const after = await page.locator('canvas').screenshot();
    expect(after.equals(settled), 'a duplicate reading (same ts_ms) must not move the camera again').toBe(true);
  });

  test('a fresh entrance guess (no real /loc yet) ALSO soft-pans the camera', async ({ page }) => {
    // why: the real bug report this guards -- the guess marker (used when
    // no confirmed /loc exists yet, e.g. right after zoning) got a
    // visibility upgrade and a camera pan alongside the confirmed one;
    // this is the one that's easy to miss entirely in a large zone at
    // default zoomed-out framing, which is exactly what happened.
    const before = await page.locator('canvas').screenshot();

    await page.evaluate(() =>
      window.__harness.setZoneContext({
        current: 'Testzone',
        previous: 'West Commonlands', // matches squareRoom's own "to_West_Commonlands" marker
        teleport_landing: null,
        current_map_zones: ['testzone'],
      }),
    );
    await page.waitForTimeout(1200); // pan duration (700ms) plus damping settle

    const after = await page.locator('canvas').screenshot();
    expect(after.equals(before), 'a resolved entrance guess should visibly pan the camera too').toBe(false);
  });

  test('re-confirming the same entrance guess does not re-pan', async ({ page }) => {
    const ctx = {
      current: 'Testzone',
      previous: 'West Commonlands',
      teleport_landing: null,
      current_map_zones: ['testzone'],
    };
    await page.evaluate((c) => window.__harness.setZoneContext(c), ctx);
    await page.waitForTimeout(1200);

    const settled = await page.locator('canvas').screenshot();
    // why: a later tick re-confirming the exact same guess (same previous
    // zone, same matched marker) must not restart the pan animation.
    await page.evaluate((c) => window.__harness.setZoneContext({ ...c }), ctx);
    await page.waitForTimeout(300);

    const after = await page.locator('canvas').screenshot();
    expect(after.equals(settled), 'the same guess recomputed must not move the camera again').toBe(true);
  });

  test('sanity check: switching to a different zone/map DOES change the view', async ({ page }) => {
    await orbit(page);
    const before = await page.locator('canvas').screenshot();

    await page.evaluate((m) => window.__harness.setMap(m), otherRoom);
    await page.evaluate((z) => window.__harness.setZone(z), 'otherzone');
    await page.waitForTimeout(300);

    const after = await page.locator('canvas').screenshot();
    expect(after.equals(before), 'a real zone switch should reframe the camera, proving the test is not vacuous').toBe(
      false,
    );
  });
});

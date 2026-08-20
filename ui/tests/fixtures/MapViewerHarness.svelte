<script lang="ts">
  // why: a real Playwright regression harness for MapViewer's own
  // reactivity contract, not the whole app/Tauri-mock plumbing --
  // MapViewer takes its geometry as plain props and reads `lastLocation`
  // from a store, so it can be exercised standalone. Exposes exactly the
  // knobs a real `parse-tick` jiggles (lastLocation, zoneContext,
  // npcMarkers) so a test can drive them the same way the real app does
  // and assert the camera never moves. See
  // ui/tests/interaction/maps-camera-stability.spec.ts.
  import MapViewer from '../../src/lib/maps/MapViewer.svelte';
  import { lastLocation } from '../../src/lib/stores/maps';
  import type { MapFileDto, NpcMarkerDto, ZoneContextDto } from '../../src/lib/tauri/api';

  let map = $state<MapFileDto>({ lines: [], markers: [] });
  let zone = $state('testzone');
  let npcMarkers = $state<NpcMarkerDto[]>([]);
  let zoneContext = $state<ZoneContextDto | null>(null);

  (window as unknown as { __harness: unknown }).__harness = {
    setMap: (m: MapFileDto) => (map = m),
    setZone: (z: string) => (zone = z),
    setNpcMarkers: (n: NpcMarkerDto[]) => (npcMarkers = n),
    setZoneContext: (c: ZoneContextDto | null) => (zoneContext = c),
    setLastLocation: (loc: unknown) => lastLocation.set(loc as never),
  };
</script>

<div style="width:600px;height:400px">
  <MapViewer {map} {zone} {npcMarkers} {zoneContext} />
</div>

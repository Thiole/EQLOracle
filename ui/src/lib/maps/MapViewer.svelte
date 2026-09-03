<script lang="ts">
  // why: real 3D wireframe geometry (wall segments + labeled markers),
  // rendered with Three.js -- one LineSegments draw call for the walls
  // regardless of segment count (tens of thousands in the biggest real
  // zones), OrbitControls for camera. This is a blueprint/wireframe view
  // of the map file's own line-art data, not the game's textured mesh --
  // see the plan's own note on what this format actually contains.
  //
  // Three effects, deliberately split, not one: a `parse-tick` arrives
  // every few seconds while the game is live, updating `lastLocation` and
  // `zoneContext` -- if the scene-build effect below depended on either,
  // it would tear down and rebuild the whole scene (camera, controls,
  // walls) on every tick, throwing the user's pan/zoom back to the
  // default view every time. So the expensive rebuild only ever depends
  // on `map`/`zone`/the canvas -- the "you are here" marker and the NPC
  // overlay each get their own small effect that moves/rebuilds just
  // their own mesh in place, camera and controls left completely alone.
  import { untrack } from 'svelte';
  import { get } from 'svelte/store';
  import * as THREE from 'three';
  import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
  import type { MapFileDto, MapMarkerDto, NpcMarkerDto, PathDto, ZoneContextDto } from '$lib/tauri/api';
  import { lastLocation } from '$lib/stores/maps';
  import { zoneMatches, looksLikeEntranceFor } from './zoneMatch';

  let webglError = $state<string | null>(null);

  let {
    map,
    zone,
    npcMarkers = [],
    zoneContext = null,
    path = null,
    onMarkerClick = null,
  }: {
    map: MapFileDto;
    zone: string;
    npcMarkers?: NpcMarkerDto[];
    zoneContext?: ZoneContextDto | null;
    /** why: a computed walking route to draw, from `Maps.svelte`'s own
     * `findWalkPath` query -- `null` when no route is currently shown.
     * Waypoints are already in the map file's own coordinate order (same
     * as `MapMarkerDto.pos`), not `/loc`-space -- no transform needed
     * here, unlike the here-marker/teleport-landing paths. */
    path?: PathDto | null;
    /** why: opt-in -- `null` (the default) means clicking a marker does
     * nothing but hover-highlight it, same as before this existed. Set by
     * `Maps.svelte` only while its own "walk here" mode is active, so
     * ordinary browsing never triggers a pathfinding query by accident. */
    onMarkerClick?: ((marker: MapMarkerDto) => void) | null;
  } = $props();

  let canvas: HTMLCanvasElement;
  let container: HTMLDivElement;

  /** why: the "you are here" marker (cross or sphere) is real, static
   * scene geometry, not a screen-space sprite -- so at a large outdoor
   * zone's default zoomed-out framing it can shrink to a few pixels
   * despite the base sizes above already being bigger than they used to
   * be. Rather than a hard-coded max size, `tick()` grows the mesh's
   * scale linearly with its distance from the camera once that distance
   * passes this reference value, so it holds a roughly constant apparent
   * size on screen (the same effect `sizeAttenuation: false` gives the
   * point-cloud markers) -- and only ever grows, never shrinks the base
   * size below what's set above, satisfied by clamping scale to a
   * minimum of 1. Not derived from anything more precise than "a
   * comfortable close-up viewing distance in this coordinate space". */
  const HERE_MESH_SCALE_REFERENCE_DIST = 400;

  // why: two distinct sources rendered in two distinct colors on purpose
  // (see the NPC points' own cyan tint below) -- the map file's own
  // hand-plotted markers and the wiki NPC catalog's coordinate-parsed
  // spawns are different data with different confidence levels, and
  // mixing them into one color would hide that. The hover tooltip tags
  // its source explicitly for the same reason.
  let hovered = $state<{ label: string; source: 'map' | 'npc' } | null>(null);
  let hoverPos = $state<{ x: number; y: number }>({ x: 0, y: 0 });
  /** why: drives the small caption near the bottom instructions -- a real
   * `/loc` reading or a teleport landing (both `confirmed`) and the
   * weaker previous-zone entrance guess (`guess`) are never shown as the
   * same confidence level, see `placeHereMesh`'s own doc. */
  let hereStatus = $state<{ kind: 'confirmed' | 'guess'; note: string } | null>(null);

  // ---- mutable, non-reactive scene state, shared across the three
  // effects below. Plain `let`s, not `$state` -- these are Three.js's own
  // objects, mutated imperatively inside effect bodies; wrapping them in
  // runes would just make Svelte track writes nothing reads reactively.
  let scene: THREE.Scene | null = null;
  // why: `THREE.Object3D`, not `THREE.Mesh` -- the confirmed marker is a
  // `Group` (two crossed bars, see `makeHereCross`), the guess marker is
  // still a plain sphere `Mesh`; both need to fit here.
  let hereMesh: THREE.Object3D | null = null;
  let npcPointsMesh: THREE.Points | null = null;
  /** why: the drawn route line -- its own mesh, own small effect below,
   * same "don't touch the expensive scene-build effect" reasoning the
   * NPC overlay already follows. `null` whenever no route is shown. */
  let pathLineMeshes: THREE.Line[] = [];
  /** why: `onPointerMove`'s hover raycast needs the *current* NPC marker
   * list to label a hit -- kept fresh by the NPC effect below rather than
   * closed over at scene-build time, since scene build no longer reruns
   * when `npcMarkers` changes (see this file's own top doc). Starts empty
   * and gets its real initial value from effect 1's own untracked
   * snapshot, not read directly here -- that would just be another
   * (unwanted) tracked dependency of whichever effect ran first. */
  let liveNpcMarkers: NpcMarkerDto[] = [];
  /** why: `panCameraTo`'s own target -- assigned once effect 1 has created
   * them, cleared on teardown. Kept separate from effect 1's local
   * `camera`/`controls` consts (which stay non-null-typed for everything
   * *inside* that effect) purely so code outside it has something to
   * reach for without a null-assertion at every use. */
  let liveCamera: THREE.PerspectiveCamera | null = null;
  let liveControls: OrbitControls | null = null;
  let panRaf = 0;
  /** why: guards against panning during the *initial* build of a scene
   * (mount, or a fresh zone/version swap) -- that moment already has its
   * own "frame the whole map" camera placement (effect 1, below); panning
   * on top of that mid-setup would fight it and glitch. Reset to false at
   * the top of every effect-1 run, set true only after that run's own
   * framing has happened -- see effect 1's own comments. */
  let sceneFramed = false;
  /** why: dedupes `panCameraTo` to real, distinct `/loc` readings -- every
   * parse-tick re-fetches `lastLocation` regardless of whether the player
   * actually moved, and panning on every tick even when they haven't
   * would be a constant unwanted drift. `ts_ms` uniquely identifies one
   * real reading. */
  let lastPannedTsMs: number | null = null;
  /** why: same dedup idea as `lastPannedTsMs`, but a guess has no natural
   * timestamp to key on -- it's driven by zone/context, not a new reading.
   * Keyed on the guess's own identity (which marker it landed on) so a
   * tick that recomputes the *same* guess doesn't re-pan, but a genuinely
   * new guess (a different zone visit, or the candidate marker itself
   * changing) does. */
  let lastPannedGuessKey: string | null = null;

  /** why: "soft move" -- smoothly re-centers the view on a fresh `/loc`
   * reading, translating both the camera and its orbit target by the same
   * offset (the same relation OrbitControls' own drag-to-pan uses) so the
   * view slides over without changing zoom or viewing angle, rather than
   * jump-cutting or re-framing the whole map. */
  function panCameraTo(target: THREE.Vector3, durationMs = 700) {
    const cam = liveCamera;
    const ctl = liveControls;
    if (!cam || !ctl) return;
    cancelAnimationFrame(panRaf);
    const startTarget = ctl.target.clone();
    const startCam = cam.position.clone();
    const endCam = startCam.clone().add(target.clone().sub(startTarget));
    const startTime = performance.now();
    function step() {
      const c = liveCamera;
      const k = liveControls;
      if (!c || !k) return;
      const t = Math.min(1, (performance.now() - startTime) / durationMs);
      const eased = 1 - (1 - t) * (1 - t); // ease-out
      k.target.lerpVectors(startTarget, target, eased);
      c.position.lerpVectors(startCam, endCam, eased);
      if (t < 1) panRaf = requestAnimationFrame(step);
    }
    step();
  }

  function clearHereMesh() {
    if (hereMesh && scene) {
      scene.remove(hereMesh);
      hereMesh.traverse((obj) => {
        if (obj instanceof THREE.Mesh) {
          obj.geometry.dispose();
          (obj.material as THREE.Material).dispose();
        }
      });
    }
    hereMesh = null;
  }

  /** why: a real confirmed `/loc` (or a teleport landing, now trusted at
   * the same tier -- see `placeHereMesh`'s own doc) gets an unmistakable
   * marker -- a red X, not just another small dot among the map's own
   * P-markers, per an explicit ask, and explicitly made bigger a second
   * time after the first size still read as too small in practice. Two
   * crossed flat bars in the horizontal (X/Z) plane, same as the map's
   * own geometry lies -- reads correctly from any orbit angle the way a
   * floor decal would, not a billboarded sprite that would face the
   * camera instead of the ground. Also grown dynamically with camera
   * distance in `tick()` (see `HERE_MESH_SCALE_REFERENCE_DIST`) so it
   * never shrinks to imperceptible at a large outdoor zone's default
   * zoomed-out framing -- this base size is only ever the *floor*. */
  function makeHereCross(): THREE.Group {
    const group = new THREE.Group();
    const geo = new THREE.BoxGeometry(34, 2.4, 5.5);
    const mat = new THREE.MeshBasicMaterial({ color: 0xdd1111 });
    const a = new THREE.Mesh(geo, mat);
    a.rotation.y = Math.PI / 4;
    const b = new THREE.Mesh(geo, mat);
    b.rotation.y = -Math.PI / 4;
    group.add(a, b);
    return group;
  }

  /** why: a guess needs to be genuinely visible too, not just logically
   * correct -- the original radius-4 sphere was real but easy to miss
   * entirely at the default "frame the whole map" zoom in a large outdoor
   * zone, with nothing panning the camera to it either (see
   * `panToGuessIfNew`, added alongside this). Bigger sphere than the
   * original, and a brighter, more saturated yellow than the first
   * brighten pass -- the muted amber still didn't read as clearly against
   * this scene's light-blue background as it needed to. Still visually
   * distinct from the confirmed marker's red cross, on purpose (never
   * claim guess-level confidence looks like confirmed-level -- see
   * `placeHereMesh`'s own doc for exactly which sources still count as a
   * guess now that a teleport landing no longer does). */
  function makeGuessMarker(): THREE.Mesh {
    return new THREE.Mesh(new THREE.SphereGeometry(10, 16, 16), new THREE.MeshBasicMaterial({ color: 0xffe600 }));
  }

  /** why: real `/loc` and a teleport landing are both `confirmed`-tier
   * evidence (the red cross) -- neither one categorically outranks the
   * other any more. Whichever is genuinely *more recent* wins: a stale
   * `/loc` reading from before a teleport must not keep overriding the
   * teleport's own landing, but a `/loc` reading taken *after* walking
   * away from a landing point is real too and must win right back. This
   * used to check `/loc` first unconditionally and only fall to a
   * teleport landing when no `/loc` existed for the zone at all -- which
   * silently kept showing an old pre-teleport `/loc` as "here" even after
   * a fresher teleport landing was known (reported directly: "you are
   * using the prev location as a you are here"). Landing evidence now
   * *fills in* location rather than being an alternate display of it.
   * A teleport landing isn't *literally* a live reading, but it's been
   * independently checked against real map-pack marker data and lands
   * within the same margin the `/loc` calibration itself has, so treating
   * it as strictly weaker would be its own kind of dishonesty -- see the
   * teleport-landing branch below for the actual cross-check numbers.
   * Reads `lastLocation` via `get()` and `zoneContext` via `untrack()` on
   * purpose: this function is also called from the scene-build effect,
   * which must NOT depend on either (see this file's own top doc) --
   * the *reactive* re-checks live in the small effect further down that
   * calls this same function.
   */
  function placeHereMesh() {
    if (!scene) return;
    clearHereMesh();
    hereStatus = null;

    const loc = get(lastLocation);
    const locMatches = loc && zoneMatches(loc.map_zones, loc.zone, zone);
    const ctx = untrack(() => zoneContext);
    const landingMatches = ctx?.current && zoneMatches(ctx.current_map_zones, ctx.current, zone) && ctx.teleport_landing;

    // Both sources present for this zone: freshest timestamp wins outright
    // (equal/unknown timestamps keep the historical `/loc`-first lean).
    const preferLoc = !landingMatches || (locMatches && (ctx!.teleport_landing_ts == null || loc!.ts_ms >= ctx!.teleport_landing_ts));

    if (locMatches && preferLoc) {
      // `/loc`'s (x, y) map to this format's own (-y, -x) -- both swapped
      // *and* negated, elevation (z) untouched. Found by brute-forcing
      // every sign/order combination of x/y against 9 real `/loc`
      // readings in Lower Guk, scored by distance to the zone's own real
      // wall geometry: this combination averaged 9.3 units off (max 16.1)
      // across all 9 -- decisively tighter than every other combination
      // tried, including plain "no swap" (which had *looked* right against
      // a single sample earlier, but averaged 80.6 units off with a 294
      // unit outlier once checked against more readings) and a plain
      // unsigned swap (1082.7 average -- badly wrong). Same
      // elevation/Y/Z remapping into three.js space the walls/markers
      // already use below (mapfile X -> three X, mapfile Z(elevation) ->
      // three Y, mapfile Y -> three Z) -- only which raw `/loc` field
      // supplies mapfile X vs Y changed.
      hereMesh = makeHereCross();
      const pos = new THREE.Vector3(-loc.y, loc.z, -loc.x);
      hereMesh.position.copy(pos);
      scene.add(hereMesh);
      hereStatus = { kind: 'confirmed', note: '/loc reading' };
      if (sceneFramed && loc.ts_ms !== lastPannedTsMs) {
        lastPannedTsMs = loc.ts_ms;
        panCameraTo(pos);
      }
      return;
    }

    // No real /loc for this zone yet, or a teleport landing is genuinely
    // the fresher evidence -- is the player actually here right now (not
    // just browsing an old map)?
    if (!ctx?.current || !zoneMatches(ctx.current_map_zones, ctx.current, zone)) return;

    // Landed via a recognized teleport cast (Wizard Translocate/Gate/
    // Portal, Druid Circle/Ring, or a learned Origin) from "You" or a
    // proven ally -- `teleport_landing` carries the spell's exact,
    // wiki-confirmed (or, for Origin, empirically-learned) destination
    // coordinate directly (see `crates/app/src/teleportdata.rs`'s own
    // doc). Treated as `confirmed`, the same tier as a real `/loc`
    // reading, not `guess` -- independently verified against the Brewall
    // map pack's own hand-plotted markers (a completely separate data
    // source from the wiki): North Karana's own `Wizard_port`/`Druid_Ring`
    // markers land within 15-45 units of this same transform applied to
    // the wiki-quoted coordinates, on a zone spanning thousands of units
    // -- see docs/design/maps.md's "Coordinate systems" section for the
    // full check. No marker-matching involved -- this used to guess via a
    // map marker labeled "Wizard_Spire"/"Druid_Circle" existing exactly
    // once on the map, which was never confirmed for Druid against a real
    // map pack; the exact coordinate makes that guess unnecessary.
    if (ctx.teleport_landing) {
      const { class: cls, x, y, z } = ctx.teleport_landing;
      // Same /loc -> map-file transform as the confirmed-/loc path above
      // -- see LastLocationDto's own doc for the mapping itself, and
      // TeleportLandingDto's own doc for why this coordinate is assumed
      // to be in that same space (now independently verified, see above).
      const pos = new THREE.Vector3(-y, z, -x);
      hereMesh = makeHereCross();
      hereMesh.position.copy(pos);
      scene.add(hereMesh);
      // `'any'` is Origin's own learned landing (see TeleportLandingDto's
      // own doc) -- no fixed spire/circle landmark the way a real
      // Wizard/Druid teleport has, so this note says what it actually is
      // instead of guessing one of those two.
      const note = cls === 'wizard' ? 'landed at the wizard spire' : cls === 'druid' ? 'landed at the druid circle' : "landed at your Origin's starting city";
      hereStatus = { kind: 'confirmed', note };
      panToGuessIfNew(`teleport:${cls}:${x},${y},${z}`, pos);
      return;
    }

    if (!ctx.previous) return;
    const candidates = map.markers.filter((m) => looksLikeEntranceFor(m.label, ctx.previous!));
    // why: more than one plausible match means genuine ambiguity (e.g. a
    // zone with two entrances from adjoining areas) -- a wrong guess is
    // worse than no marker, same reasoning the NPC-candidate matching
    // uses elsewhere in this module.
    if (candidates.length !== 1) return;
    const m = candidates[0];
    const pos = new THREE.Vector3(m.pos[0], m.pos[2], m.pos[1]);
    hereMesh = makeGuessMarker();
    hereMesh.position.copy(pos);
    scene.add(hereMesh);
    hereStatus = { kind: 'guess', note: `probably entered from ${ctx.previous}` };
    panToGuessIfNew(`entrance:${ctx.previous}:${m.label}`, pos);
  }

  /** why: pans to a guess the same way a real `/loc` does -- a guess with
   * no camera movement to it is easy to miss entirely in a large outdoor
   * zone (the default "frame the whole map" view can put a small marker
   * at imperceptible size). Gated on `sceneFramed` for the same reason as
   * the confirmed path (don't fight the initial framing), and on
   * `lastPannedGuessKey` so a tick re-confirming the *same* guess doesn't
   * re-pan every time. */
  function panToGuessIfNew(key: string, pos: THREE.Vector3) {
    if (!sceneFramed || key === lastPannedGuessKey) return;
    lastPannedGuessKey = key;
    panCameraTo(pos);
  }

  // ---- effect 1: the expensive one -- (re)builds the whole scene, camera,
  // controls, walls, and render loop. Deliberately depends on nothing but
  // `map`/`zone` (plus the canvas/container bindings) so a `parse-tick`
  // never touches it -- see this file's own top doc.
  $effect(() => {
    if (!canvas || !container) return;
    sceneFramed = false;

    const localScene = new THREE.Scene();
    // why: light muted blue, not near-black -- real map files carry a lot
    // of pure-black wall geometry (confirmed: 4,799 of Befallen's 8,177
    // `L` lines are literally `0,0,0`), which vanished entirely against
    // the old near-black `0x0b0e13` background. Gray (`128,128,128`,
    // the format's other common wall color) and the app's own marker
    // colors (red/bright-yellow "here", cyan NPC dots) all still read
    // clearly against this.
    localScene.background = new THREE.Color(0xb3c7d6);
    scene = localScene;

    const camera = new THREE.PerspectiveCamera(60, 1, 0.1, 100_000);
    // why: WebGL can genuinely be absent (Remote Desktop, VMs,
    // blocklisted GPUs, WebView2 with acceleration disabled) -- an
    // unguarded constructor throw left the map tab a silent blank
    // canvas; a visible reason beats a mystery
    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
    } catch {
      webglError = 'The 3D map view needs WebGL, which this system reports unavailable (common over Remote Desktop, in VMs, or with GPU acceleration disabled).';
      return;
    }
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.08;
    liveCamera = camera;
    liveControls = controls;

    // ---- walls: one LineSegments draw call, whatever the segment count.
    const lineCount = map.lines.length;
    const positions = new Float32Array(lineCount * 6);
    const colors = new Float32Array(lineCount * 6);
    for (let i = 0; i < lineCount; i++) {
      const l = map.lines[i];
      const o = i * 6;
      positions[o] = l.a[0];
      positions[o + 1] = l.a[2]; // Y-up: map file's Z (elevation) -> three.js Y
      positions[o + 2] = l.a[1];
      positions[o + 3] = l.b[0];
      positions[o + 4] = l.b[2];
      positions[o + 5] = l.b[1];
      const [r, g, b] = l.color;
      colors[o] = colors[o + 3] = r / 255;
      colors[o + 1] = colors[o + 4] = g / 255;
      colors[o + 2] = colors[o + 5] = b / 255;
    }
    const wallGeo = new THREE.BufferGeometry();
    wallGeo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
    wallGeo.setAttribute('color', new THREE.BufferAttribute(colors, 3));
    const walls = new THREE.LineSegments(wallGeo, new THREE.LineBasicMaterial({ vertexColors: true }));
    localScene.add(walls);

    // ---- markers: one Points draw call, hover-picked via raycasting.
    const markerCount = map.markers.length;
    const mPositions = new Float32Array(markerCount * 3);
    const mColors = new Float32Array(markerCount * 3);
    for (let i = 0; i < markerCount; i++) {
      const m = map.markers[i];
      mPositions[i * 3] = m.pos[0];
      mPositions[i * 3 + 1] = m.pos[2];
      mPositions[i * 3 + 2] = m.pos[1];
      const [r, g, b] = m.color;
      mColors[i * 3] = r / 255;
      mColors[i * 3 + 1] = g / 255;
      mColors[i * 3 + 2] = b / 255;
    }
    const markerGeo = new THREE.BufferGeometry();
    markerGeo.setAttribute('position', new THREE.BufferAttribute(mPositions, 3));
    markerGeo.setAttribute('color', new THREE.BufferAttribute(mColors, 3));
    const markerPoints = new THREE.Points(
      markerGeo,
      new THREE.PointsMaterial({ vertexColors: true, size: 6, sizeAttenuation: false }),
    );
    localScene.add(markerPoints);

    // ---- wiki NPC spawn points: a second, distinctly-colored Points
    // object -- real data, but a *different* real data source than the
    // map file's own hand-plotted markers (see npcMarkers prop's own
    // doc), so it stays visually distinguishable rather than merged into
    // the same point cloud. Built from an *untracked* snapshot here (the
    // scene-build effect must not depend on npcMarkers, see this file's
    // own top doc) -- the effect further down keeps it current after a
    // toggle without rebuilding any of the rest of this scene.
    const initialNpcMarkers = untrack(() => npcMarkers);
    liveNpcMarkers = initialNpcMarkers;
    const npcGeo = new THREE.BufferGeometry();
    npcGeo.setAttribute('position', new THREE.BufferAttribute(npcPositionsOf(initialNpcMarkers), 3));
    npcGeo.setAttribute('color', new THREE.BufferAttribute(npcColorsOf(initialNpcMarkers), 3));
    const localNpcPoints = new THREE.Points(
      npcGeo,
      // why: vertexColors -- a surveyed spawn (the in-game /loc pack) is
      // green, a wiki XY-only spot stays cyan, same confirmed/assumed
      // split the drop list uses
      new THREE.PointsMaterial({ vertexColors: true, size: 7, sizeAttenuation: false }),
    );
    localScene.add(localNpcPoints);
    npcPointsMesh = localNpcPoints;

    // ---- "you are here" -- a real /loc if one matches this zone,
    // otherwise the entrance guess. See `placeHereMesh`'s own doc.
    placeHereMesh();

    // ---- camera: frame the walls' own bounding box.
    wallGeo.computeBoundingSphere();
    const sphere = wallGeo.boundingSphere;
    if (sphere && sphere.radius > 0) {
      const dist = sphere.radius * 2.2;
      camera.position.set(sphere.center.x + dist, sphere.center.y + dist * 0.6, sphere.center.z + dist);
      controls.target.copy(sphere.center);
    } else {
      camera.position.set(200, 200, 200);
    }
    camera.updateProjectionMatrix();
    controls.update();
    sceneFramed = true;

    function resize() {
      const w = container.clientWidth;
      const h = container.clientHeight;
      renderer.setSize(w, h, false);
      camera.aspect = w / Math.max(h, 1);
      camera.updateProjectionMatrix();
    }
    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(container);

    const raycaster = new THREE.Raycaster();
    raycaster.params.Points = { threshold: 8 };
    const pointer = new THREE.Vector2();
    function onPointerMove(e: PointerEvent) {
      const rect = canvas.getBoundingClientRect();
      pointer.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
      pointer.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
      raycaster.setFromCamera(pointer, camera);
      // why: check both point clouds, take whichever hit is nearer the
      // camera -- either source can be hovered, never just whichever was
      // added to the scene first.
      const mapHit = raycaster.intersectObject(markerPoints)[0];
      const npcHit = npcPointsMesh ? raycaster.intersectObject(npcPointsMesh)[0] : undefined;
      const closer = !mapHit ? npcHit : !npcHit ? mapHit : mapHit.distance <= npcHit.distance ? mapHit : npcHit;
      if (!closer) {
        hovered = null;
      } else if (closer === mapHit) {
        const m = map.markers[mapHit.index ?? -1];
        hovered = m ? { label: m.label.replace(/_/g, ' '), source: 'map' } : null;
      } else if (npcHit) {
        // why: `liveNpcMarkers`, not the closed-over `initialNpcMarkers`
        // -- a toggle after this effect last ran must not hover stale
        // entries or index past the end of a shrunk array.
        const m = liveNpcMarkers[npcHit.index ?? -1];
        hovered = m ? { label: m.name, source: 'npc' } : null;
      } else {
        hovered = null;
      }
      if (hovered) hoverPos = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    }
    canvas.addEventListener('pointermove', onPointerMove);

    // why: same raycast setup as the hover handler above, but only ever
    // against the map's own markers (not the NPC overlay -- clicking a
    // wiki NPC spawn to route to it would need a stated confidence caveat
    // this doesn't have time to carry yet) and only wired up at all when
    // a caller opted in via `onMarkerClick`.
    function onClick(e: MouseEvent) {
      if (!onMarkerClick) return;
      const rect = canvas.getBoundingClientRect();
      pointer.x = ((e.clientX - rect.left) / rect.width) * 2 - 1;
      pointer.y = -((e.clientY - rect.top) / rect.height) * 2 + 1;
      raycaster.setFromCamera(pointer, camera);
      const hit = raycaster.intersectObject(markerPoints)[0];
      const m = hit ? map.markers[hit.index ?? -1] : undefined;
      if (m) onMarkerClick(m);
    }
    canvas.addEventListener('click', onClick);

    let raf = 0;
    function tick() {
      controls.update();
      // why: keeps the "you are here" marker visible regardless of zoom
      // -- see HERE_MESH_SCALE_REFERENCE_DIST's own doc. `hereMesh` is
      // reassigned (not mutated in place) by the other two effects, so
      // this always reads whichever mesh is current, cross or sphere.
      if (hereMesh) {
        const dist = camera.position.distanceTo(hereMesh.position);
        hereMesh.scale.setScalar(Math.max(1, dist / HERE_MESH_SCALE_REFERENCE_DIST));
      }
      renderer.render(localScene, camera);
      raf = requestAnimationFrame(tick);
    }
    tick();

    return () => {
      cancelAnimationFrame(raf);
      cancelAnimationFrame(panRaf);
      ro.disconnect();
      canvas.removeEventListener('pointermove', onPointerMove);
      canvas.removeEventListener('click', onClick);
      controls.dispose();
      wallGeo.dispose();
      markerGeo.dispose();
      npcGeo.dispose();
      hereMesh?.traverse((obj) => {
        if (obj instanceof THREE.Mesh) {
          obj.geometry.dispose();
          (obj.material as THREE.Material).dispose();
        }
      });
      for (const m of pathLineMeshes) {
        m.geometry.dispose();
        (m.material as THREE.Material).dispose();
      }
      renderer.dispose();
      scene = null;
      hereMesh = null;
      npcPointsMesh = null;
      pathLineMeshes = [];
      liveCamera = null;
      liveControls = null;
      sceneFramed = false;
    };
  });

  function npcColorsOf(markers: NpcMarkerDto[]): Float32Array {
    const out = new Float32Array(markers.length * 3);
    for (let i = 0; i < markers.length; i++) {
      const survey = markers[i].source === 'survey';
      out[i * 3] = survey ? 0.13 : 0.18;
      out[i * 3 + 1] = survey ? 0.77 : 0.83;
      out[i * 3 + 2] = survey ? 0.37 : 1.0;
    }
    return out;
  }

  function npcPositionsOf(markers: NpcMarkerDto[]): Float32Array {
    // `z` is null for most real entries (the wiki scrape is 2D for the
    // majority of mobs) -- rendered at 0 rather than guessed from nearby
    // wall geometry, an honest "unknown", not a fabricated elevation.
    const out = new Float32Array(markers.length * 3);
    for (let i = 0; i < markers.length; i++) {
      const m = markers[i];
      out[i * 3] = m.x;
      out[i * 3 + 1] = m.z ?? 0;
      out[i * 3 + 2] = m.y;
    }
    return out;
  }

  // ---- effect 2: keeps the NPC overlay's point cloud current when
  // `npcMarkers` changes (a toggle in Maps.svelte) -- swaps just this
  // mesh's geometry in place, camera/controls/walls untouched.
  $effect(() => {
    const markers = npcMarkers; // tracked read -- this effect's whole point
    liveNpcMarkers = markers;
    if (!npcPointsMesh) return;
    const oldGeo = npcPointsMesh.geometry;
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.BufferAttribute(npcPositionsOf(markers), 3));
    geo.setAttribute('color', new THREE.BufferAttribute(npcColorsOf(markers), 3));
    npcPointsMesh.geometry = geo;
    oldGeo.dispose();
  });

  // ---- effect 2.5: draws/clears the computed route line -- its own
  // small effect, own mesh, camera/controls/walls/here-marker untouched.
  // Rebuilt whenever `path` changes (a new route computed, or cleared) or
  // the zone/map itself changes (a stale route from a different zone must
  // never linger on screen -- `Maps.svelte` is expected to clear its own
  // `path` state on a zone switch too, this is defense in depth).
  $effect(() => {
    const p = path;
    void zone;
    void map;
    for (const m of pathLineMeshes) {
      scene?.remove(m);
      m.geometry.dispose();
      (m.material as THREE.Material).dispose();
    }
    pathLineMeshes = [];
    if (!scene || !p || p.waypoints.length < 2) return;
    // Same mapfile -> three.js axis remap the walls/markers already use
    // (mapfile X -> three X, mapfile Z/elevation -> three Y, mapfile Y ->
    // three Z) -- these waypoints are already in the map file's own
    // coordinate order (see this file's own `path` prop doc), not
    // `/loc`-space, so no additional transform is needed here.
    const toGeo = (pts: [number, number, number][]) => {
      const positions = new Float32Array(pts.length * 3);
      pts.forEach(([x, y, z], i) => {
        positions[i * 3] = x;
        positions[i * 3 + 1] = z;
        positions[i * 3 + 2] = y;
      });
      const geo = new THREE.BufferGeometry();
      geo.setAttribute('position', new THREE.BufferAttribute(positions, 3));
      return geo;
    };
    // why: bright green, distinct from every existing marker color (red
    // confirmed, bright yellow guess, cyan NPC) -- `linewidth` is a real
    // WebGL limitation, not a mistake here: most GPU/driver combinations
    // render `LineBasicMaterial` at a flat 1px regardless of this value,
    // so the route reads as a thin line, not a thick highlighted one.
    // One Line per leg: walk legs solid, hop legs (teleporter pads --
    // the location changes, nothing is walked) dashed, so a Sky route
    // reads as islands-walked plus ports-taken, not one impossible line
    // across the void. `legs` always present from the current backend;
    // guarded anyway so an old mock fixture still draws the flat line.
    const legs = p.legs?.length ? p.legs : [{ kind: 'walk' as const, waypoints: p.waypoints, label: null }];
    for (const leg of legs) {
      if (leg.waypoints.length < 2) continue;
      const geo = toGeo(leg.waypoints);
      let mesh: THREE.Line;
      if (leg.kind === 'hop') {
        mesh = new THREE.Line(geo, new THREE.LineDashedMaterial({ color: 0x22c55e, dashSize: 8, gapSize: 6 }));
        mesh.computeLineDistances();
      } else {
        mesh = new THREE.Line(geo, new THREE.LineBasicMaterial({ color: 0x22c55e, linewidth: 2 }));
      }
      scene.add(mesh);
      pathLineMeshes.push(mesh);
    }
  });

  // ---- effect 3: keeps the "you are here" marker current as real `/loc`
  // readings and zone-context updates arrive on every `parse-tick` --
  // moves/rebuilds just this one small mesh, camera/controls/walls
  // untouched. `zone`/`map` are read too so a zone switch (which effect 1
  // already handles via its own inline `placeHereMesh()` call) doesn't
  // leave this effect holding a stale dependency snapshot.
  $effect(() => {
    void $lastLocation;
    void zoneContext;
    void zone;
    void map;
    placeHereMesh();
  });
</script>

<div bind:this={container} class="relative h-full w-full">
  <canvas bind:this={canvas} class="block h-full w-full cursor-grab active:cursor-grabbing" class:hidden={webglError !== null}></canvas>
  {#if webglError}<p class="p-3 text-[12px] text-bad">{webglError}</p>{/if}
  {#if hovered}
    <div
      class="pointer-events-none absolute flex items-center gap-1.5 rounded-sm border border-border bg-card px-2 py-1 text-[11px] shadow-md"
      style:left="{hoverPos.x + 12}px"
      style:top="{hoverPos.y + 12}px"
    >
      <span class="size-1.5 rounded-full" style:background={hovered.source === 'npc' ? '#2dd4ff' : 'var(--color-foreground)'}></span>
      {hovered.label}
      {#if hovered.source === 'npc'}<span class="text-muted-foreground">(wiki NPC)</span>{/if}
    </div>
  {/if}
  <p class="pointer-events-none absolute bottom-2 left-2 text-[10px] text-muted-foreground">
    drag to orbit · scroll to zoom · hover a marker for its label
    {#if npcMarkers.length > 0}· cyan = wiki NPC spawns, height not always known{/if}
    {#if hereStatus}
      · <span style:color={hereStatus.kind === 'confirmed' ? '#dd1111' : '#ffe600'}>●</span>
      {hereStatus.kind === 'confirmed'
        ? hereStatus.note === '/loc reading'
          ? 'you are here'
          : `you are here -- ${hereStatus.note}`
        : hereStatus.note}
    {/if}
  </p>
</div>

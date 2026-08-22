# Maps — design notes

Rationale for the Maps module (`crates/app/src/mapsdata.rs`,
`crates/app/src/teleportdata.rs`, `crates/app/src/npcdata.rs`,
`crates/app/src/commands.rs`'s map-facing commands, `ui/src/lib/maps/`).

## What it is

A live 3D wireframe viewer over the game's own community map-pack files, with
a "you are here" marker that's right even when the player hasn't typed `/loc`
— entrance guesses filled in from log context instead. Three.js renders the
wall geometry as one `LineSegments` draw call regardless of segment count
(tens of thousands in the biggest zones); markers are a second `Points`
draw call, hover-picked via raycasting.

## Three independent data sources, kept separate on purpose

1. **Map geometry** (`mapsdata.rs`) — the classic EQ ASCII format,
   `<install>/maps/*.txt` (`L x1,y1,z1,x2,y2,z2,r,g,b` wall segments,
   `P x,y,z,r,g,b,size,Label` points), read from the user's own configured
   game install at runtime, not bundled. A zone's map is split across a base
   file plus optionally-numbered siblings (`northkarana.txt` +
   `northkarana_1.txt` + `northkarana_2.txt`) — `load_zone_map` always merges
   every matching file, since some siblings carry only points, others carry
   real wall geometry too.
2. **NPC spawns** (`npcdata.rs`) — wiki-scraped coordinates, a completely
   different confidence level than the map file's own hand-plotted points
   (2D for most entries — the scrape rarely gives elevation). Rendered as a
   second, distinctly cyan-tinted point cloud in `MapViewer.svelte` so the
   two sources are never visually conflated, and the hover tooltip tags
   which source a hit came from.
3. **Teleport landings** (`teleportdata.rs`) — also wiki-scraped, but a
   different field entirely: eqlwiki states a Wizard Translocate/Gate/Portal
   or Druid Circle/Ring spell's *exact* destination coordinate directly, via
   a `SpellSlotRowSmart` template row on the spell's own page. See that
   module's doc comment for the scraper bug that hid this for years (a
   parser only matching the literal `SpellSlotRow` template name, never the
   `Smart` variant that turned out to be dominant — 831 of 1,928 spells lost
   all slot data to it) and for why this pack is the sole recognizer of "is
   this cast a real teleport" rather than a name-shape heuristic (real
   false positives found: "Circle of Summer"/"Circle of Winter" match the
   "Circle of X" shape but are resist buffs, not teleports).

## Zone tracking — knowing "current zone" and "which visit" at all

Every `zone.enter` line lands in `Ingest.zone`, a `Spans` (`crates/session/
src/context.rs`) — a general-purpose append-only "what label was current at
this instant" structure, not specific to zones at all (`Sessions` is the
same structure keyed on silence-gaps instead of zone lines, and
`ClassDetector`'s per-visit evidence grouping keys off the same `index_at`
this module uses). Out-of-order insertion is handled (a late line can't
corrupt lookups) and re-entering the same zone consecutively collapses
rather than fragmenting into a new span.

Four queries fall out of that one append-only log, and the Maps module
uses three of them directly:

- `at(ts)` — the raw zone label current at `ts`. `ZoneContextDto::current`.
- `label_before(ts)` — the label of the *previous* span, i.e. what zone the
  player almost certainly walked in **from**. `ZoneContextDto::previous`,
  the input to the previous-zone entrance guess (see below).
- `index_at(ts)` — which *visit* `ts` falls in, distinct from the label:
  you visit Nektulos Forest 35 times across a session, and those are 35
  different visits, not one bucket. Not surfaced on `ZoneContextDto`
  itself, but this is the same mechanism `combat::class_combination`'s
  per-zone-visit class evidence and `Ingest::record_history`'s loadout
  tagging both key off — one shared "what visit was this" primitive
  across otherwise-unrelated features.
- `bounds(i)` — start/end of a given visit; not currently used by Maps.

## Zone identity — matching a raw log label to a real map file

A raw `zone.enter` label ("The Ruins of Old Guk 4 (Refined)") and a real
map file's own internal shortname ("gukbottom") share no text in common at
all for most zones — this is the gap `zonedata.rs`/`zoneMatch.ts` exist to
close, in two layers:

1. **Real resolution** (`zonedata::map_shortnames`, backend). The wiki's
   own scrape (`packs/zones.json`) carries a `who_name` field per zone —
   the game's own internal shortname(s), sometimes several
   (`"gukbottom/gukbottom2"`, comma-separated for a multi-area zone, with
   `<tag>`-style annotations stripped). `commands::map_zones_for_raw_label`
   resolves a raw label to its wiki `Zone` via `zone::zone_matches` first
   (difficulty-tier suffix, `"- Group"`/`"- Solo"` raid-instance markers,
   and a leading `"The"` all stripped/normalized before comparing — see
   `zone.rs`'s own doc for why *both* sides of a comparison have to go
   through the same stripping, not just one), then splits that zone's
   `who_name` into real shortnames. This is what `ZoneContextDto::
   current_map_zones` carries, and what the frontend's `zoneMatches`
   (`zoneMatch.ts`) checks membership in to decide "is the map I have open
   actually my current zone" — real membership, not a text-similarity
   guess.
2. **Fallback guess** (`looksLikeSameZone`, frontend only). A loose
   normalized-substring check, used only when step 1 comes back empty (no
   wiki match for that raw label, or that zone's `who_name` is blank) —
   so a zone with no shortname data isn't worse off than before real
   resolution existed, but a zone that *does* resolve never falls back to
   the weaker guess.

`looksLikeEntranceFor` is a third, narrower text match — not "is this my
current zone" but "does this specific map marker look like the zone-line
exit toward `previous`" (`to_West_Commonlands` vs `"West Commonlands 4
(Refined)"`), used only for the previous-zone entrance guess itself, and
only trusted when it narrows to exactly one marker.

## Coordinate systems — the actual gotcha

The map file's own `(x, y, z)` order is **not** the order `/loc` prints in,
and is not the order the wiki's teleport-destination text uses either — all
three of "map file", "`/loc`", and "wiki-quoted destination" turned out to
agree with each other, but only once you know the right transform.

`/loc`-space -> map-file-space is `(mapfile_x, mapfile_y) = (-loc_y, -loc_x)`,
elevation untouched. Found by brute-forcing every sign/order combination
against 9 real `/loc` readings in Lower Guk, scored by distance to that
zone's own real wall geometry: this combination averaged 9.3 units off (max
16.1) across all 9 — decisively tighter than "no swap" (which looked right
against a single early sample, but averaged 80.6 off with a 294-unit outlier
once checked against more) or a plain unsigned swap (1082.7 average, badly
wrong). Applied identically in `MapViewer.svelte` to a real `/loc` reading
and (assumed, see below) to a `teleport_landing` coordinate before either
gets plotted; the wall/marker geometry itself uses a third, related mapping
(mapfile X -> three X, mapfile Z/elevation -> three Y, mapfile Y -> three Z)
since Three.js is Y-up and the map format isn't.

**The teleport-landing coordinate space was originally an assumption, not a
proven fact** — reasoned as "these wiki entries are almost certainly sourced
by a player typing `/loc` right after landing," but the reference log has
zero real teleport-cast-then-`/loc`-reading pairs to check that against (of
115 real landings, none were followed by a `/loc` before the next zone
change). **Since independently confirmed** by cross-referencing against the
Brewall map pack's own hand-plotted markers, which are a completely
separate data source from the wiki: North Karana's own map file has a
`Wizard_port` marker at `(-1250, 3700, 0)` and a `Druid_Ring` marker at
`(1449.4, 2730.7, -10.6)`. Applying the `/loc`-space transform to the wiki's
quoted destinations for that zone — Wizard `(-3685, 1209, -5)` and Druid
`(-2706, -1494, -4)` — lands at `(-1209, 3685, -5)` and `(1494, 2706, -4)`
respectively: within ~15-45 units of the real markers on a zone spanning
thousands of units, the same order of tightness as the original `/loc`
calibration's own residual. Two independently-sourced estimates of "the
same real spot" converging like that is about as strong a confirmation as
this project can get without a live `/loc` reading. Note the real marker
labels turned out to be `Wizard_port`/`Druid_Ring` for this specific zone,
not `Wizard_Spire`/`Druid_Circle` as earlier, unshipped label-matching code
had guessed — one more reason the exact-coordinate approach replaced that
guess entirely rather than being layered on top of it.

## "You are here" — the location-estimation ladder

`MapViewer.svelte`'s `placeHereMesh` runs one strictly-ordered ladder, every
tick and every zone/map switch, stopping at the first rung that has
something to say — never blending two rungs, never falling back past a
rung that *did* produce an answer:

1. Real `/loc` reading for the currently-open zone.
2. A confirmed `teleport_landing`.
3. Previous-zone entrance guess (`to_<previous>` marker, exactly one match).
4. Nothing plotted at all — an honest "don't know", not a fabricated guess.

Rungs 1-2 render identically (red cross, `confirmed`); rung 3 renders
weaker (yellow sphere, `guess`) — see below for why 1 and 2 collapsed into
one tier. Every rung also requires `zoneMatches(ctx.current_map_zones,
ctx.current, zone)` to hold first (the currently-*open* map really is the
player's real current zone) — a stale map from a zone visited hours ago
never gets a marker just because *some* old context still matches it.
Spelled out in detail, in the same order:

1. **Confirmed** (red cross, bigger than it first shipped and growing with
   camera distance — see `HERE_MESH_SCALE_REFERENCE_DIST` — so it never
   reads as a few stray pixels in a large outdoor zone) — either a real
   `/loc` reading for the currently-open zone (rare — only fires on the
   manual `/loc` command), **or** a `teleport_landing` (exact wiki
   coordinate, see above). The two were originally different tiers (a
   teleport landing shipped as a `guess`), promoted to the same tier once
   the coordinate-space assumption was independently checked against real
   map-pack marker data (see "Coordinate systems" above) and landed within
   the same margin the `/loc` calibration itself has — treating it as
   strictly weaker than a live reading after that would be its own kind of
   dishonesty, not caution.
2. **Guess** (bright yellow sphere) — no real `/loc` or teleport landing,
   but the log context implies a position: a `to_<previous zone>` map
   marker matched against the zone the player just walked in from
   (`looksLikeEntranceFor`, only used when exactly one marker matches —
   ambiguity means no marker at all, never a wrong guess).

The two tiers are deliberately never visually conflated (different
shape/color) and the caption text says which one is showing — "never claim
guess-level confidence looks like confirmed-level" is a standing rule
across this module. That rule is exactly why a teleport landing had to
earn its way into the confirmed tier via the independent cross-check above,
rather than being assumed into it.

`Ingest::entered_via_teleport` (backend) decides *which* zone visit counts
as a landing: set on `Action::Zone` from whichever recognized teleport cast
(`teleportdata::landing_for`) — by "You" or a **proven ally** (`is_ally`,
since group-shaped Portal/Ring/group-Translocate/group-Circle casts land
the whole group, not just the caster) — most recently began within
`TELEPORT_WINDOW_MS` (30s, cast time + loading screen, confirmed ~15s in
the reference log). State is a plain "last one wins" overwrite, not an
accumulating log — which turns out to matter: a fizzled cast followed by an
immediate successful retry (same or a different teleport spell — players
sometimes just switch spells) is handled correctly for free, because the
retry overwrites the stale fizzled value before the zone-enter fires. No
real log case was found of a fizzle *without* a retry being followed by an
unrelated zone-line walk within the window (which would be a genuine, if
rare, false-positive path — nothing currently cross-checks the entered zone
against the spell's own claimed destination).

## Camera stability — why `MapViewer.svelte` has three effects, not one

A `parse-tick` arrives every few seconds while the game is live, updating
`lastLocation` and `zoneContext`. If the expensive scene-build effect
(camera, controls, wall/marker geometry) depended on either, it would tear
down and rebuild the whole scene on every tick, throwing the user's pan/zoom
back to the default framing constantly. So the rebuild only ever depends on
`map`/`zone`; the "you are here" marker and the NPC overlay each get their
own small effect that moves/rebuilds just their own mesh in place, camera
and controls left alone. Guarded by `ui/tests/interaction/
maps-camera-stability.spec.ts` (pixel-diff screenshots against a real
Three.js scene via an isolated harness, not the whole app).

## Pathfinding — in-zone walking and zone-to-zone routing

`crates/app/src/pathfind.rs` (in-zone) and `crates/app/src/routing.rs`
(zone-to-zone) are new infrastructure, built from scratch — there is no
existing graph/pathfinding code anywhere in this workspace to build on
(`eqlp_session::graph` is a union-find encounter builder, not a reusable
weighted graph; no crate here pulls in a graph library).

**In-zone: grid A\*, not a visibility graph.** `mapsdata::ParsedZoneMap` is
only line segments and labeled points — no floor polygons, no nav-mesh. A
classic visibility graph (wall endpoints as nodes) is untenable at real
scale (up to 26,383 wall segments in one zone, `everfrost.txt`, confirmed
on disk — 52k+ endpoints, O(n^2) edge candidates). `pathfind::find_path`
instead grids the zone's own bounding box adaptively (~250 cells per axis,
clamped 8–200 units) and runs binary-heap A* with real segment-segment
intersection tests deciding which adjacent-cell edges are blocked — never
a "is this cell inside a wall" test, since these are open line obstacles,
not filled polygons.

Two real, checked-not-assumed findings shaped the design:

1. **Z-banded, not true 3D.** A multi-level dungeon's walls occupy very
   different Z ranges per floor (confirmed: Befallen spans Z −90.6 to
   +26.1). Every query filters wall segments to a Z window (`Z_BAND`, ±40
   units) around the *starting* point before building the grid — auto-
   discovering stairs/ramps from line-art alone is a research-grade
   problem this format was never built for; a route needing a floor change
   returns `None` rather than a fabricated path through the wrong level.
2. **Not every `L` line is a wall.** Confirmed directly against the real
   `northkarana.txt`: treating every line as a hard obstacle left 2 of 3
   real zone-line exit markers in a disconnected 19% pocket of an
   otherwise-one-piece 2D flood-fill. Sampling every real color used in
   that file found five, not the two ("black or gray") this codebase had
   previously established as wall colors: brown/dirt tones (terrain
   contour art, not obstacles), a blue (water — swimmable in this game,
   not a hard wall), and a magenta appearing exactly once per real zone
   exit (the zone-boundary marker line itself, which must never block the
   route leading to it). `is_wall_color` now only treats grayscale lines
   (`r == g == b`) as real obstacles — fixed one of the two disconnected
   markers outright; the third sits inside a real, narrow, purpose-built
   gate structure a coarse grid can still miss, a separate, already-
   documented resolution tradeoff. `nearest_open_cell` (spiral outward
   from a query point) separately fixes the common case of a query point
   landing exactly on/against wall geometry — confirmed real: a route
   queried to a zone-line marker's own exact coordinate failed outright
   even though every point a few percent short of it succeeded, since
   real exit markers are typically placed right against their own
   boundary geometry.

**Zone-to-zone: cheap candidates, then real-distance scoring.**
`routing::find_zone_route` is two-stage and deliberately lazy: (1) a bounded
DFS over a cheap graph (`zonedata::Zone::adjacent_zones`, populated for
115/117 real zones, plus one teleport edge *from every zone* per
`teleport_landings.json` entry — teleport spells aren't zone-gated in this
game, so a Wizard/Druid shortcut is available from anywhere, not a normal
A↔B edge) generates up to 5 candidate hop sequences within 2 hops of the
shortest; (2) only those candidates get scored with real distance —
`pathfind::find_path` between each hop's own zone-line exit marker(s),
falling back to straight-line distance when a hop's map/marker/path can't
be resolved (a real, confirmed possibility — never a silent zero or a
dropped candidate). Never precomputes a distance matrix across all 117
zones; cost stays proportional to what's actually queried.

`TeleportLanding::zone` (added to `teleportdata.rs` for this — previously
unused) isn't one clean format: confirmed against the real pack, sometimes
a proper `Zone::name`, sometimes a bare map-shortname string the wiki left
unlinked, and for 5 real entries a shorter name than the wiki's own
zone-guide title for the same zone (`"North Karana"` vs the guide's
`"Northern Plains of Karana"` — `routing::TELEPORT_ZONE_ALIASES`, hand-
verified one at a time, same "small stated exceptions" shape `zone.rs`'s
own `ZONE_ALIASES` already uses for the analogous log-label problem). One
real entry (`"Grimling Forest"`) genuinely isn't in the 117-zone scrape at
all — a stated gap, not guessed around.

**Teleport shortcuts are gated by the player's own assumed class/level, not
unconditionally available.** `TeleportLanding` also carries `level` (added
alongside `zone`, straight from `spells.json`'s own per-class `classes:
[{class, level}]` field — 2 more real entries lost when their own
`classes` came back empty in the raw scrape, packs down to 103).
`routing::zone_graph_for(player_classes, player_level)` only adds a
teleport edge when both match: the log owner's *dominant* confirmed class
configuration (`combat::class_configurations`'s first, most-zone-visits
entry) and that configuration's own `level_range` upper bound as the
assumed level — "assumed" deliberately, since `Ingest::levels` only ever
tracks one *effective* level across a whole 3-class loadout, never one
level per class (see that struct's own doc); a per-spell-exact level isn't
derivable from anything this app parses. No confirmed configuration at all
(a fresh session) means walk-only, not a guessed default. Every
`RouteHopDto` still names its own spell (`via_spell`) rather than folding
into a generic "shortcut" even though it's now gated — the frontend
surfaces it plainly so the *player* can judge viability for a case this
app's own class detection got wrong or hasn't caught up to yet, not just
so the backend can skip a check it now actually makes.

**Succor/evacuate points are a real intra-zone shortcut, not just a better
`zone_centroid` guess.** Per the user's own confirmation of the mechanic:
Lesser Evacuate (or simply changing the zone's difficulty tier) relocates
you to a zone's own succor point from *anywhere* in that zone, no class
restriction. `zonedata::succor_points` parses `Zone::succor_evacuate`'s
real, messy wiki text (114/117 zones carry it; multi-part zones like
Cabilis list several, `<br>`-separated; at least one real entry is missing
its own closing `)` where the `<br>` cut it off — all handled, not
guessed past) into structured points. `routing::hop_distance` tries both
"walk directly from wherever you arrived" and "warp to the succor point,
then walk from there" for every walked hop and takes whichever is
genuinely shorter (`SUCCOR_WARP_COST`, a flat cost, since it's not
class-gated) — the user's own words, "I'd rather cross 2 short stones...
than 1 really long one." `best_start_position` (renamed from
`zone_centroid`) prefers a zone's own first succor point over the wall-line
average for a route's unknown-position first hop, a measured, large real
improvement: `Ak'Anon -> Steamfont Mountains` dropped from a 3-teleport,
2400-unit detour to the honest 1-hop, 60-unit walk it actually is, once
the start position stopped being an arbitrary mathematical average.

**Real, measured performance gap, not fixed**: a walk-only route (no
teleport access at all) across many real zones is slow — `Ak'Anon ->
Northern Plains of Karana` with no classes took **12.7s** against the
actual configured install (13 real hops, each running real A* against
zones up to tens of thousands of wall segments). `cached_map` (a
process-lifetime cache of parsed map files, keyed by shortname) cut a
Wizard-gated version of the same query from 195ms to 128ms, but didn't
meaningfully help the walk-only case — most of its 13 zones are visited
only once across the whole candidate search, so there's little real
file-load reuse to cache away; the actual cost is dominated by running
many distinct real `pathfind::find_path` searches, not repeated I/O.
Acceptable for the realistic case this feature exists for (a player with
real Wizard/Druid access, which resolves in well under a second); a
genuinely walk-only long-distance query is a real, un-optimized slow path,
stated here rather than silently shipped without a number attached.

## Known gaps for future work

- `teleport_landings.json` covers 103 of 111 name-shape-matched spells —
  eight real gaps, three false positives correctly excluded (see
  `teleportdata.rs` doc), three genuine upstream wiki data gaps (`Cazic
  Gate`, `Iceclad Portal`, `Ring of North Karana` never surfaced in the
  scrape at all despite a real page existing), two more (`Ring of
  Faydark`, `Thurgadin Gate`) lost when the `level` field needed a real
  per-class level the scrape's own `classes` came back empty for. A cast
  of one of these eight is invisible to the Maps module's entrance guess
  entirely — no marker-matching fallback was kept once the exact-coordinate
  path replaced it — and never offered as a routing shortcut either.
- Succor-point parsing has 5 real zones producing zero points out of the
  114 that carry the field (`zonedata::succor_points`'s own doc) — 4 are
  genuinely non-coordinate text (`"?"`, `"N/A"`, a bare landmark
  description), 1 (`Nektulos Forest`) is a real upstream wiki typo
  (`"--259"`, a double negative) deliberately not silently "corrected",
  even though the intended value is fairly obvious — consistent with this
  module's stance elsewhere of stating a gap rather than guessing past it.
- The fizzle-without-retry false-positive path above is real but
  unobserved in the reference log — not fixed, since fixing it needs the
  entered zone checked against the spell's own claimed destination zone,
  which `teleport_landings.json`/`TeleportLanding` now carries (`zone`
  field) but `Ingest::entered_via_teleport` still doesn't consume.
- In-zone pathfinding's Z-banding, grid resolution, and wall-color
  heuristic are all real, stated approximations, not exact geometry — see
  this doc's own "Pathfinding" section above for the concrete cases that
  motivated each one.
- `find_walk_path`'s "from" position for a live "walk here" query has no
  access to `MapViewer.svelte`'s own resolved here-marker position (that
  resolution stays internal to the component) — `Maps.svelte` currently
  approximates it from a real `/loc` reading when one matches the loaded
  zone, else the zone's own first marker, the same kind of "somewhere in
  this zone" stand-in `routing.rs::zone_centroid` uses server-side for a
  route's own first hop.
- `base_dir` (the game's install root, where `maps/` and inventory dumps
  live) is a separate configured setting from `log_dir` (the `Logs`
  subfolder) — see `BACKLOG.md`/`config.rs` if a future session needs to
  touch first-launch setup.

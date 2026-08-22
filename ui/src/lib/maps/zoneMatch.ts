// why: shared by MapViewer.svelte (the "you are here" marker/entrance
// guess) and Maps.svelte (the debug info line) -- one definition, so the
// debug line can never show a different answer than what the marker logic
// actually used.

/** why: last-resort fallback only -- a loose substring check on the map
 * file's own zone name against the log's raw zone.enter label, which are
 * never going to match exactly ("befallen" vs "Befallen 4 (Refined)"
 * happens to be close; "gukbottom" vs "The Ruins of Old Guk" shares no
 * text at all). Real resolution comes from the backend's `current_map_
 * zones`/`map_zones` (the wiki's own scraped shortname per zone, see
 * `commands::map_zones_for_raw_label`'s doc) via `zoneMatches` below --
 * this only still runs when that list comes back empty (no wiki match, or
 * that zone has no recorded shortname), so a real zone with no shortname
 * data isn't worse off than before this existed. */
export function looksLikeSameZone(rawLabel: string | null | undefined, mapZone: string): boolean {
  if (!rawLabel) return false;
  const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, '');
  const label = norm(rawLabel);
  const target = norm(mapZone);
  return label.startsWith(target) || target.startsWith(label);
}

/** why: real resolution first (membership in the backend's own
 * wiki-sourced shortname list), the text-guess only as fallback -- see
 * `looksLikeSameZone`'s own doc for why the guess alone isn't enough.
 * `mapZones` empty/undefined means "couldn't resolve it", not "doesn't
 * match" -- falls through to the guess rather than going stricter than
 * before this existed. */
export function zoneMatches(mapZones: string[] | undefined, rawLabel: string | null | undefined, mapZone: string): boolean {
  if (mapZones && mapZones.length > 0) return mapZones.includes(mapZone);
  return looksLikeSameZone(rawLabel, mapZone);
}

export const normalizeWord = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, '');

/** why: a TS port of `zonedata::map_shortnames` (Rust) -- the Maps
 * module's zone list is keyed by real map-file shortname (e.g.
 * "northkarana"), but `find_zone_route` needs `ZoneDto.name`
 * (e.g. "Northern Plains of Karana"). Rather than adding a new backend
 * command just to resolve one string, this mirrors the Rust splitting
 * logic (comma/slash-separated, `<tag>`-annotations and trailing
 * `(parenthetical)` notes stripped) so the frontend can find, from
 * `listZones()`'s own `who_name` field, which `ZoneDto` a given shortname
 * belongs to -- kept deliberately in sync with the Rust version, same
 * reasoning `looksLikeEntranceFor`'s own doc gives for mirroring
 * `routing.rs`'s server-side entrance matching. */
export function mapShortnames(whoName: string): string[] {
  const out: string[] = [];
  for (const part of whoName.split(',')) {
    const cleaned = part.replace(/<[^>]*>/g, '');
    for (const sub of cleaned.split('/')) {
      const name = sub.split('(')[0].trim();
      if (name) out.push(name);
    }
  }
  return out;
}

/** why: EQ map files label zone-line markers "to_<Zone_Name>" by
 * long-standing community convention (confirmed in this app's own bundled
 * data, e.g. befallen_1.txt's one "to_West_Commonlands" marker) --
 * stripped before comparing so the leading "to" doesn't defeat the match
 * against the plain zone name. */
export function stripToPrefix(label: string): string {
  return label.replace(/^to[_\s]?/i, '');
}

/** why: same loose, honest-about-its-limits spirit as `looksLikeSameZone`
 * -- checked both directions and as a substring, since real marker labels
 * and real zone names truncate/extend each other in either direction with
 * no fixed rule ("to_West_Commonlands" vs "West Commonlands 4
 * (Refined)"). Only ever used when the caller has already required
 * *exactly one* marker to match -- see MapViewer.svelte's `placeHereMesh`. */
export function looksLikeEntranceFor(markerLabel: string, previousZoneRaw: string): boolean {
  const marker = normalizeWord(stripToPrefix(markerLabel));
  const prev = normalizeWord(previousZoneRaw);
  if (!marker || !prev) return false;
  return marker.startsWith(prev) || prev.startsWith(marker) || marker.includes(prev) || prev.includes(marker);
}

// A Wizard/Druid teleport landing used to be guessed here via a
// map-marker-label match ("does exactly one Wizard_Spire/Druid_Circle
// marker exist on this map") -- removed once `teleport_landing`
// (backend: `crates/app/src/teleportdata.rs`) started supplying the
// spell's *exact* wiki-confirmed (x, y, z) directly, which MapViewer.svelte
// now plots the same way it plots a real `/loc` reading, no marker
// matching involved. See `teleportdata.rs`'s own doc for why that's
// strictly better (the label-guess was also never confirmed for Druid
// against a real map pack, and the name-shape heuristic that used to
// decide "is this even a teleport" had real false positives -- "Circle of
// Summer"/"Circle of Winter" are resist buffs, not teleports).

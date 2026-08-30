#!/usr/bin/env python3
"""Builds packs/teleport_landings.json (teleport-family spell name -> exact
landing coordinates) directly from the raw scrape (spells.json), the same
way build_spell_classes.py builds packs/spell_classes.json.

Real motivation: the Maps module's "you are here" entrance guess for a
Wizard Translocate/Gate/Portal or Druid Circle/Ring landing used to be a
map-marker-label match ("does exactly one Wizard_Spire/Druid_Circle marker
exist on this zone's map") -- a guess, and for Druid specifically an
unverified one. It turns out unnecessary: eqlwiki states the *exact*
(x,y,z) landing coordinate for every one of these spells directly, via a
`{{SpellSlotRowSmart | 1 | Teleport group to X,Y,Z in [[Zone]] | ... }}`
row -- confirmed by fetching the live raw wikitext for North Karana Gate,
North Karana Portal, Circle of Karana, and Translocate: North Karana
directly (not assumed from the stale local scrape, which had this data
silently dropped -- see the `parse_slots` fix in
scrape_eqlwiki_spells.py's own doc comment for why, and re-run that
scraper before this one if spells.json predates that fix).

`teleport_class` below is a line-for-line mirror of
`crates/app/src/ingest.rs`'s Rust function of the same name -- keep them
in sync by hand (same reasoning `build_spell_classes.py`'s `base_name`
mirrors `Ingest::base_spell_name`).

Real, confirmed false positives caught by cross-checking against actual
slot data rather than trusting the name-shape heuristic alone: "Circle of
Force"/"Circle of Summer"/"Circle of Winter" match the "Circle of X" name
shape but are damage-shield/resist buffs, not teleports -- they have no
coordinate-shaped slot text at all, so they're naturally excluded by
requiring a real coordinate match, not name-pattern alone. This is why the
Rust side should resolve `TeleportClass` by membership in *this* pack
rather than trusting `teleport_class()`'s name heuristic on its own --
see that function's own doc for the wiring.
"""
import json
import re

SRC = "spells.json"
DST = "../eqlp/packs/teleport_landings.json"

TELEPORT_RE = re.compile(
    r'(?:Teleport(?:s)?(?:\s+(?:group|self|your\s+group|target))?|Translocate)\s+to\s+'
    r'\(?(-?\d+)\s*,\s*(-?\d+)\s*,\s*(-?\d+)\)?\s+in\s+'
    r'(?:\[\[([^\]|]+)|([A-Za-z0-9_]+))'
)


def teleport_class(name: str) -> str | None:
    if name.startswith("Translocate: "):
        return "wizard"
    if name.endswith(" Gate") and len(name) > len(" Gate"):
        return "wizard"
    if name.endswith(" Portal") and len(name) > len(" Portal"):
        return "wizard"
    if name.startswith("Circle of ") or name.startswith("Ring of "):
        return "druid"
    return None


def main():
    with open(SRC, encoding="utf-8") as f:
        data = json.load(f)

    out = {}
    skipped = []
    for spell in data["spells"]:
        name = spell["name"]
        cls = teleport_class(name)
        if cls is None:
            continue
        found = None
        for slot in spell.get("slots") or []:
            m = TELEPORT_RE.search(slot["effect"])
            if m:
                found = m
                break
        if found is None:
            skipped.append((name, cls, [s["effect"] for s in spell.get("slots") or []]))
            continue
        x, y, z = (int(found.group(1)), int(found.group(2)), int(found.group(3)))
        zone = (found.group(4) or found.group(5)).strip()
        # Level requirement, for "does this player's assumed class/level
        # actually have this spell" filtering (routing.rs). Matched by
        # class name, not just "first entry" -- a spell's own `classes`
        # list is per-class-with-its-own-level, and this pack's `cls` is
        # already resolved to exactly one of Wizard/Druid by
        # `teleport_class` above, so there's exactly one right entry to
        # pull the level from.
        class_name = "Wizard" if cls == "wizard" else "Druid"
        level = next((c["level"] for c in spell.get("classes") or [] if c.get("class") == class_name), None)
        if level is None:
            skipped.append((name, cls, [f"no {class_name} level found in classes={spell.get('classes')!r}"]))
            continue
        out[name] = {"class": cls, "x": x, "y": y, "z": z, "zone": zone, "level": level}

    payload = dict(sorted(out.items()))
    with open(DST, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=1)
        f.write("\n")

    print(f"wrote {len(payload)} teleport landings to {DST}", flush=True)
    print(f"skipped {len(skipped)} name-shape matches with no parseable coordinate:", flush=True)
    for name, cls, effects in skipped:
        print(f"  {name} ({cls}): {effects}", flush=True)


if __name__ == "__main__":
    main()

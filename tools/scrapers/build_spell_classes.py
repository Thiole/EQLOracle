#!/usr/bin/env python3
"""Builds packs/spell_classes.json (a base-spell-name -> class list lookup)
directly from the raw scrape (spells.json), the same way
build_monster_drops.py builds packs/monsters.json from items.json.

The file that shipped before this script existed (1,431 entries) was
missing real class data spells.json already has for at least 21 spells,
and had 13 entries whose "class" was actually a stray wiki-link elsewhere
on the page ("a ghoul", "Spirit of Inferno Strike") that got hand-dropped
one at a time as they were found in a live player's log rather than fixed
at the source -- both symptoms of building on top of an already-lossy
intermediate file instead of the clean, structured scrape. This rebuilds
from spells.json's own `classes: [{class, level}, ...]` field directly, no
intermediate step.

Key is the *base* (rank-stripped) name, not the wiki page title -- rank
variants ("Elemental Maelstrom X") are separate wiki pages, but
Ingest::base_spell_name in the Rust app strips the rank before ever
looking a name up, so the lookup has to be keyed the same way or every
ranked cast (nearly all of them) misses. Stripping logic here is a
deliberate line-for-line match of that Rust function, including the same
protected-name exceptions -- see its own doc comment for why those exist.
"""
import json
import re

SRC = "spells.json"
DST = "../eqlp/packs/spell_classes.json"

# Mirrors Ingest::base_spell_name's PROTECTED_SPELL_NAMES exactly.
PROTECTED = {
    "Burnout II", "Burnout III", "Clarity II", "Monster Summoning I",
    "Rune I", "Rune II", "Rune III", "Yaulp II", "Yaulp III",
}
ROMAN = set("IVXLCDM")


def base_name(name: str) -> str:
    if name in PROTECTED:
        return name
    if " " in name:
        head, tail = name.rsplit(" ", 1)
        if tail and all(ch in ROMAN for ch in tail):
            return head
    return name


# The wiki's own `classes=` field is occasionally a stray link elsewhere on
# the page rather than a real class (confirmed against the raw data, not
# assumed) -- filtered by an allowlist of the game's actual classes, not a
# denylist of the junk seen so far, since a re-scrape could surface new
# junk shapes. "Shadowknight" (no space) and "Shadow Knight" are the same
# class split across two spellings on different wiki pages; folded into
# one before it ever reaches the app, so evidence for a Shadow Knight
# doesn't fragment across two lookup keys.
VALID_CLASSES = {
    "Bard", "Beastlord", "Cleric", "Druid", "Enchanter", "Magician",
    "Necromancer", "Paladin", "Ranger", "Rogue", "Shadow Knight", "Shaman",
    "Warrior", "Wizard",
}
ALIASES = {"Shadowknight": "Shadow Knight"}


def main():
    with open(SRC, encoding="utf-8") as f:
        data = json.load(f)

    out: dict[str, set[str]] = {}
    dropped_classes = set()
    for spell in data["spells"]:
        classes = set()
        for c in spell.get("classes") or []:
            raw = c.get("class")
            if not raw:
                continue
            norm = ALIASES.get(raw, raw)
            if norm in VALID_CLASSES:
                classes.add(norm)
            else:
                dropped_classes.add(raw)
        if not classes:
            continue
        base = base_name(spell["name"])
        out.setdefault(base, set()).update(classes)

    # Verified by hand against eqlwiki (WebFetch, not guessed) because
    # they're absent from spells.json despite having real wiki pages with
    # real class data -- Category:Spells enumeration missed them for
    # reasons not otherwise investigated. `Harm Touch` in particular lives
    # under a `Skill:`-prefixed title (a redirect), outside this scrape's
    # category scope entirely, not just a missed page within it.
    HAND_VERIFIED = {
        "Leech": ["Necromancer"],
        "Malaisement": ["Magician", "Shaman"],
        "Blast of Cold": ["Wizard"],
        "Harm Touch": ["Shadow Knight"],
    }
    for name, classes in HAND_VERIFIED.items():
        out.setdefault(name, set()).update(classes)

    payload = {k: sorted(v) for k, v in sorted(out.items())}
    with open(DST, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=1)
        f.write("\n")

    print(f"wrote {len(payload)} base spell names to {DST}", flush=True)
    print(f"dropped non-class values seen on {len(dropped_classes)} distinct strings: {sorted(dropped_classes)[:20]}", flush=True)


if __name__ == "__main__":
    main()

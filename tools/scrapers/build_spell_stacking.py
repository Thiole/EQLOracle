#!/usr/bin/env python3
"""Builds packs/spell_stacking.json (spell name -> stacking group id)
from the real client data files shipped with the game install:
spells_us.txt (numeric spell id -> name, the classic-EQ base spell
table this fork's client still ships) and Resources/SpellStackingGroups.txt
(numeric spell id -> stacking group/rank/type -- spells sharing a group
can't both be active; the newer/higher-rank one wins, the other fails to
land or gets overwritten).

Only 48 of this app's 1,928 catalog spells (spells.json) get a real
match -- spells_us.txt is the legacy classic-EQ spell table (ids 1..~74k),
so it only covers spells that carried over from classic content by name
(poison/disease DoT lines, familiar lines, Levitate/Levitation, a handful
of debuffs). This fork's own newer spells (Ice Comet, illusions, Yaulp
variants, etc.) were never in spells_us.txt at all, so they get no entry
here -- the app's own lineKey/isIllusion heuristics (in
ui/src/lib/character/spellSuggest.ts) still carry those. This file is a
real, authoritative supplement for the subset it does cover, not a
replacement.

STACKING_RANK/STACKING_TYPE are dropped -- for this app's purpose (never
suggest two spells that can't both usefully be active), only group
membership matters; which one wins isn't modeled.
"""
import json

GAME_DIR = "/home/Spencer/Games/eq-legends/drive_c/users/Public/Daybreak Game Company/Installed Games/EverQuest Legends"
DST = "../eqlp/packs/spell_stacking.json"

id_to_name = {}
with open(f"{GAME_DIR}/spells_us.txt", encoding="latin-1") as f:
    for line in f:
        parts = line.split("^")
        if len(parts) < 2:
            continue
        try:
            sid = int(parts[0])
        except ValueError:
            continue
        id_to_name[sid] = parts[1]

catalog_names = {s["name"] for s in json.load(open("spells.json"))["spells"]}

name_to_group = {}
with open(f"{GAME_DIR}/Resources/SpellStackingGroups.txt", encoding="latin-1") as f:
    next(f)  # header
    for line in f:
        parts = line.strip().split("^")
        if len(parts) < 4:
            continue
        sid, grp = int(parts[0]), int(parts[1])
        name = id_to_name.get(sid)
        if name in catalog_names:
            name_to_group[name] = grp

with open(DST, "w") as f:
    json.dump(name_to_group, f, indent=1, sort_keys=True)
    f.write("\n")

print(f"wrote {len(name_to_group)} entries to {DST}")

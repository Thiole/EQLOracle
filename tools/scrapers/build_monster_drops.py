#!/usr/bin/env python3
"""Builds packs/monsters.json (a mob name -> known wiki drops lookup) for
eqlp's Monsters module, the same way scrape_eqlwiki_spells.py's output feeds
crates/app/src/classdata.rs.

Source: items.json's `drops` field (`[{"zone": ..., "mobs": [...]}]` per
item) -- the only place a mob name exists anywhere in the scraped data.
There is no dedicated mobs/npcs.json; a mob that drops nothing the scrape
recorded simply never appears here, and eqlp's Monsters module knows that
(see crates/app/src/monsterdata.rs's doc comment) rather than treating
absence as "not a real monster".

Keys are folded the same way `eqlp_session::fold_key` folds combat-log
names (lowercase the first character only, leave the rest alone) before
this JSON is even written -- not left for the Rust side to normalise at
lookup time. The wiki's own mob-name casing is inconsistent purely by
sentence position on whatever page it was scraped from ("A Chetari Hunter"
on one page, "a Chetari Hunter" on another, both the same mob) -- exactly
the case-folding problem this project already solved once for the combat
log itself (docs/design/session.md, "Case folding"). Folding at build time
means every consumer -- today's Rust lookup, a future one in another
language -- gets one clean key per mob for free, instead of everyone having
to remember to re-derive the same fold.
"""
import json
import sys
from collections import defaultdict
from datetime import datetime, timezone

SRC = "items.json"
DST = "../eqlp/packs/monsters.json"


def fold_key(name: str) -> str:
    if not name:
        return name
    return name[0].lower() + name[1:]


# The wiki's `mobs` column isn't always a mob name -- some item pages carry
# an editorial note instead ("no longer drops.", "which mobs?", "various
# Mobs Level 1 - 15"), which the scrape captured verbatim since it can't
# tell prose from a name. None of these will ever equal a real combat-log
# entity name, so they're harmless if kept, but they bloat the embedded
# file and would look wrong in a future "browse all known monsters" view --
# filtered by shape (a real mob name is never a question, a sentence, or
# the literal word "mob(s)") rather than an exhaustive denylist, since new
# junk shapes will keep showing up on every re-scrape.
def looks_like_junk(name: str) -> bool:
    low = name.lower()
    return "?" in name or name.endswith(".") or "mob" in low or "drop" in low or len(name) > 45


def main():
    with open(SRC, encoding="utf-8") as f:
        data = json.load(f)

    mobs: dict[str, set[str]] = defaultdict(set)
    skipped = 0
    for item in data["items"]:
        for drop in item.get("drops") or []:
            for mob in drop.get("mobs") or []:
                if looks_like_junk(mob):
                    skipped += 1
                    continue
                mobs[fold_key(mob)].add(item["name"])

    out = {mob: sorted(items) for mob, items in sorted(mobs.items())}

    payload = {
        "source": data.get("source", "https://eqlwiki.com"),
        "built": datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC"),
        "from": SRC,
        "count": len(out),
        "mobs": out,
    }

    with open(DST, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=2, ensure_ascii=False)
        f.write("\n")

    print(f"wrote {len(out)} mobs to {DST} ({skipped} junk drop-list entries skipped)", file=sys.stderr)


if __name__ == "__main__":
    main()

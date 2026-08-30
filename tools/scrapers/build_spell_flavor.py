#!/usr/bin/env python3
"""Builds packs/spell_flavor.json (a first-person landing-message -> class
list lookup) from spells.json, for attributing buffs that land with no
"begins casting" line at all -- specifically Quick Buff, an AA that's
class-agnostic itself but silently applies whatever buffs the activator
actually knows, leaving only the buff's own landing flavor text behind
("A burst of strength surges through your body.", confirmed against the
reference log immediately after real "<Name> activates Quick Buff." lines).

Keyed by spells.json's own `msg_cast_on_you` field -- the first-person
message a spell prints when it lands *on you* specifically (not
`msg_cast_on_other`, which fires when it lands on someone else and can't
be safely attributed without knowing who cast it -- see
crates/app/src/ingest.rs's quickbuff-window doc for why this stays scoped
to the activator's own buffs landing on themselves). Only spells with real
class data are included; a message with no class signal has nothing to
contribute. A message shared verbatim by more than one spell (rare, not
verified impossible) merges to the union of their classes, the same
ambiguous-evidence handling `classdetect::Detector` already gives a shared
spell *name*.
"""
import json

SRC = "spells.json"
DST = "../eqlp/packs/spell_flavor.json"

VALID_CLASSES = {
    "Bard", "Beastlord", "Cleric", "Druid", "Enchanter", "Magician",
    "Necromancer", "Paladin", "Ranger", "Rogue", "Shadow Knight", "Shaman",
    "Warrior", "Wizard",
}
ALIASES = {"Shadowknight": "Shadow Knight"}

# Real, hand-verified wiki typos in `msg_cast_on_you` itself -- confirmed
# directly against the live wiki page's own raw wikitext (not a scraper
# bug) *and* against the real game log, which never lies about its own
# output: "Guardian Rhythms" (Bard) wiki text says "...surround you.",
# but every real occurrence in the reference log reads "...surrounding
# you." -- a plain missing "-ing", not a rewording, since a mismatched
# message here means the whole point of this pack (exact-string landing
# attribution) silently stops matching. Keyed by spell name so a future
# re-scrape that happens to fix the wiki page itself doesn't leave a
# stale override in place unnoticed (this only overwrites the message
# for the one exact spell named, never blind text-replaces).
MESSAGE_CORRECTIONS = {
    "Guardian Rhythms": "You feel an aura of mystic protection surrounding you.",
}


def main():
    with open(SRC, encoding="utf-8") as f:
        data = json.load(f)

    out: dict[str, set[str]] = {}
    for spell in data["spells"]:
        msg = spell.get("msg_cast_on_you")
        if not msg or msg == "None":
            continue
        msg = MESSAGE_CORRECTIONS.get(spell.get("name"), msg)
        classes = set()
        for c in spell.get("classes") or []:
            raw = c.get("class")
            if not raw:
                continue
            norm = ALIASES.get(raw, raw)
            if norm in VALID_CLASSES:
                classes.add(norm)
        if not classes:
            continue
        out.setdefault(msg, set()).update(classes)

    payload = {k: sorted(v) for k, v in sorted(out.items())}
    with open(DST, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=1)
        f.write("\n")
    print(f"wrote {len(payload)} landing messages to {DST}", flush=True)


if __name__ == "__main__":
    main()

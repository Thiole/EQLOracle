#!/usr/bin/env python3
"""
scrape_eqlwiki_sky_quests.py — builds sky_quests.json from the wiki's own
"Plane of Sky Class Quests" section (https://eqlwiki.com/Plane_of_Sky
#Plane_of_Sky_Class_Quests).

Confirmed directly from the page's own wikitext, not assumed: 16 classes
(Bard, Beastlord, Berserker, Cleric, Druid, Enchanter, Magician, Monk,
Necromancer, Paladin, Ranger, Rogue, Shadow Knight, Shaman, Warrior,
Wizard), one quest-giver NPC each, 6-7 turn-in quests per class, every
quest needing exactly one Wind Rune plus 1-2 other quest items (never a
stacked quantity above 1 -- every item is a unique/rare drop, listed as
its own `<li>`, never "2x"). The page states directly: "completing all of
these quests will unlock the respective class as a Primary Class option
in your loadouts" -- this is EQL's own class-unlock mechanic, not just a
gear-reward quest chain.

Each class's own quest table is a `{| class="eoTable3" ... |}` wikitable,
columns `Quest || Trigger Phrases || Rune || Quest Items || Reward`, with
Rune/Quest Items each rendered as a `<ul><li>[[Item Name]]</li>...</ul>`
block (letting a class's rare 2-item quests still parse the same way a
1-item one does) and Reward as a `{{:Reward Page Name}}` transclusion
(the reward's own item page, pulled by name only here -- this pack does
not carry the reward's own stats, `itemdata.rs`'s own catalog already
does if the name resolves there). A quest item's own parenthetical note
("(3-Gorga)") names which island/boss it drops from -- kept as `source`,
never parsed further (its own format isn't consistent enough to split
mechanically: "3-Gorga" is an island + a specific boss's own short name,
but not always both).

Usage:
    python scrape_eqlwiki_sky_quests.py --out sky_quests.json
"""

import argparse
import json
import re
import sys

try:
    import requests
except ImportError:
    sys.exit("Missing deps. Run:  pip install requests")

API = "https://eqlwiki.com/api.php"
UA = "EQL-Gear-Planner/2.0 (personal build planner; contact: local user)"
PAGE = "Plane_of_Sky"

WIKILINK_RE = re.compile(r"\[\[([^\]|]+)(?:\|[^\]]*)?\]\]")
LI_RE = re.compile(r"<li>(.*?)</li>", re.DOTALL)
SOURCE_NOTE_RE = re.compile(r"\(([^)]+)\)\s*$")
REWARD_RE = re.compile(r"\{\{:([^}]+)\}\}")
CLASS_HEADER_RE = re.compile(r"=== \[\[([^\]]+)\]\] Tests ===")
QUEST_GIVER_RE = re.compile(r"'''Quest Giver:'''\s*\[\[([^\]|]+)")


def fetch_wikitext(title):
    r = requests.get(
        API,
        params={
            "action": "query", "prop": "revisions", "rvslots": "main",
            "rvprop": "content", "titles": title, "format": "json", "formatversion": 2,
        },
        headers={"User-Agent": UA},
        timeout=30,
    )
    r.raise_for_status()
    data = r.json()
    page = data["query"]["pages"][0]
    if "revisions" not in page:
        sys.exit(f"page {title!r} has no content -- check the title")
    return page["revisions"][0]["slots"]["main"]["content"]


def parse_items(block):
    """One `<ul>...</ul>` block -> [{"item": str, "source": str|None}, ...],
    one entry per `<li>`. `[[Wikilink|Display]]` and a real link nested
    inside `'''{{SkyNoDrop|[[Name]]}}'''` are both handled -- only the
    first wikilink target on the line is ever taken, which is correct for
    both real shapes seen on this page (never more than one link per
    `<li>` in the actual data)."""
    out = []
    for li in LI_RE.findall(block):
        m = WIKILINK_RE.search(li)
        if not m:
            continue
        item = m.group(1).strip()
        sm = SOURCE_NOTE_RE.search(li.strip())
        source = sm.group(1).strip() if sm else None
        out.append({"item": item, "source": source})
    return out


def parse_class_block(class_name, block):
    giver_m = QUEST_GIVER_RE.search(block)
    giver = giver_m.group(1).strip() if giver_m else None

    # Split the wikitable into rows on "|-", drop the header row (its own
    # first cell is the literal column title "Quest").
    table_m = re.search(r'\{\|[^\n]*\n(.*?)\n\|\}', block, re.DOTALL)
    if not table_m:
        return giver, []
    rows_raw = table_m.group(1).split("|-")
    quests = []
    for row in rows_raw:
        cells = [c.strip() for c in row.split("\n|") if c.strip()]
        # The header row's first cell literally starts with "! Quest" (a
        # wikitable header marker, not a data row) -- skip it, not by
        # position, since a malformed row could otherwise shift the rest.
        if not cells or cells[0].startswith("!"):
            continue
        # A genuine data row always has: name, trigger, rune-block,
        # items-block, reward -- 5 cells once split on "|". A row with
        # fewer than that is leftover markup between tables (there isn't
        # any on this page, checked), not a quest -- skip rather than
        # guess.
        if len(cells) < 5:
            continue
        name, trigger, rune_block, items_block, reward_block = cells[:5]
        rune = parse_items(rune_block)
        items = parse_items(items_block)
        reward_m = REWARD_RE.search(reward_block)
        reward = reward_m.group(1).strip() if reward_m else None
        quests.append({
            "quest": name.lstrip("|").strip(),
            "trigger": trigger,
            "rune": rune[0]["item"] if rune else None,
            "items": items,
            "reward": reward,
        })
    return giver, quests


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="sky_quests.json")
    args = ap.parse_args()

    wikitext = fetch_wikitext(PAGE)

    section_m = re.search(
        r"== Plane of Sky Class Quests ==(.*?)\n= [^=]",
        wikitext + "\n= END", re.DOTALL,
    )
    if not section_m:
        sys.exit("couldn't find the 'Plane of Sky Class Quests' section -- page structure changed")
    section = section_m.group(1)

    headers = list(CLASS_HEADER_RE.finditer(section))
    classes = []
    for i, h in enumerate(headers):
        class_name = h.group(1).strip()
        start = h.end()
        end = headers[i + 1].start() if i + 1 < len(headers) else len(section)
        giver, quests = parse_class_block(class_name, section[start:end])
        classes.append({"class": class_name, "quest_giver": giver, "quests": quests})
        print(f"{class_name}: giver={giver!r}, {len(quests)} quests")

    payload = {
        "source": f"https://eqlwiki.com/{PAGE}#Plane_of_Sky_Class_Quests",
        "note": "Completing all of a class's quests unlocks it as a Primary Class option (EQL's own loadout mechanic, per the wiki page's own text).",
        "classes": classes,
    }
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=1)
        f.write("\n")
    total_quests = sum(len(c["quests"]) for c in classes)
    print(f"\nwrote {len(classes)} classes, {total_quests} quests to {args.out}")


if __name__ == "__main__":
    main()

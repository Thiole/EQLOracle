#!/usr/bin/env python3
"""Build packs/epic_quests.json from the 15 class epic quest pages.

why: item-first farm list -- the Endgame Epic Quests tab tracks the
loot-drop materials (Kill X, loot Y) so they can be farmed before the
Epic Quests Era opens. NPC-handed intermediates (receive-lines) are
deliberately excluded: they need the era's own quest NPCs.

input:  --from-cache-scan DIR  scan an existing .eqlcache for the pages
        (offline, the normal mode); otherwise fetches via the wiki API
        with the same on-disk cache the other scrapers use
output: packs/epic_quests.json
run:    python3 tools/scrapers/scrape_eqlwiki_epic_quests.py \
            --from-cache-scan ~/eql/.eqlcache
"""

import argparse
import glob
import json
import os
import re
import sys
import urllib.parse
import urllib.request

API = "https://eqlwiki.com/api.php"
CACHE = ".eqlcache"

CLASSES = [
    "Bard", "Berserker", "Cleric", "Druid", "Enchanter", "Magician",
    "Monk", "Necromancer", "Paladin", "Ranger", "Rogue", "Shadow Knight",
    "Shaman", "Warrior", "Wizard",
]

LINK = r"\[\[([^\]|#]+)(?:[^\]]*)?\]\]"


def cache_path(key):
    import hashlib
    os.makedirs(CACHE, exist_ok=True)
    return os.path.join(CACHE, hashlib.md5(key.encode()).hexdigest() + ".json")


def api_get(params, cache_key=None):
    if cache_key:
        p = cache_path(cache_key)
        if os.path.exists(p):
            with open(p, encoding="utf-8") as f:
                return json.load(f)
    url = API + "?" + urllib.parse.urlencode(params)
    req = urllib.request.Request(url, headers={"User-Agent": "eqlp-scraper"})
    with urllib.request.urlopen(req) as r:
        data = json.load(r)
    if cache_key:
        with open(cache_path(cache_key), "w", encoding="utf-8") as f:
            json.dump(data, f)
    return data


def fetch_page(title):
    data = api_get(
        {
            "action": "query", "prop": "revisions", "rvprop": "content",
            "rvslots": "main", "titles": title, "format": "json",
            "formatversion": "2",
        },
        cache_key="epicpage:" + title,
    )
    for p in data["query"]["pages"]:
        if p.get("revisions"):
            return p["revisions"][0]["slots"]["main"]["content"]
    raise SystemExit(f"no content for {title}")


def scan_cache_dir(cache_dir, wanted):
    """Longest cached revision per wanted title, scanning every response file."""
    found = {}
    for f in glob.glob(os.path.join(cache_dir, "*.json")):
        try:
            with open(f, encoding="utf-8") as fh:
                s = fh.read()
        except OSError:
            continue
        if "Epic Quest" not in s:
            continue
        try:
            d = json.loads(s)
        except json.JSONDecodeError:
            continue
        pages = d.get("query", {}).get("pages")
        if not isinstance(pages, list):
            continue
        for p in pages:
            t = p.get("title")
            if t in wanted and p.get("revisions"):
                c = p["revisions"][0].get("slots", {}).get("main", {}).get("content", "")
                if len(c) > len(found.get(t, "")):
                    found[t] = c
    return found


def header_field(text, label):
    m = re.search(r"'''\s*" + label + r":?\s*'''\s*\n\|\s*(.+)", text)
    if not m:
        return None
    v = m.group(1).strip()
    v = re.sub(LINK, lambda mm: mm.group(1).strip(), v)
    return v.strip() or None


def rewards_of(text):
    m = re.search(r"^==\s*Rewards?\s*==\s*$(.*?)(?=^==[^=]|\Z)", text, re.M | re.S)
    if not m:
        return []
    out = []
    for mm in re.finditer(r"\{\{:([^}|]+)\}\}|" + LINK, m.group(1)):
        name = (mm.group(1) or mm.group(2)).strip()
        if name and not name.lower().startswith(("file:", "category:")):
            out.append(name)
    seen, uniq = set(), []
    for n in out:
        if n not in seen:
            seen.add(n)
            uniq.append(n)
    return uniq


def farmable_text(text):
    """Whole page minus stale sections.

    why: checklist formats differ per class (Paladin dash-bullets, Ranger
    scatters loot lines across sub-sections around an empty checklist
    stub) -- scanning everything with dedupe beats guessing the one true
    section. Only Wizard's Pre-Revamp block is cut: superseded content
    whose items would pollute the farm list.
    """
    out = []
    drop = False
    for line in text.splitlines():
        m = re.match(r"^==\s*([^=\n]+?)\s*==\s*$", line)
        if m:
            drop = bool(re.search(r"pre-revamp", m.group(1), re.I))
        if not drop:
            out.append(line)
    return "\n".join(out)


def parse_loot_lines(section):
    """Farmable drops with kill context, three line shapes:
    'loot [N] [[Item]]', dash-bullets '* [[Item]] - [[Mob]] in Zone',
    and forage bullets '* [[Item]] from [[Zone]]'."""
    items = {}
    order = []

    def add(name, mobs, zone, qty, optional, source=None):
        e = items.get(name)
        if e is None:
            items[name] = {
                "item": name,
                "mobs": mobs[:6],
                "zone": zone,
                "qty": qty,
                "optional": optional,
                "source": source,
            }
            order.append(name)
        else:
            e["qty"] = max(e["qty"], qty)
            e["optional"] = e["optional"] and optional
            if e["zone"] is None:
                e["zone"] = zone
            for mb in mobs:
                if mb not in e["mobs"] and len(e["mobs"]) < 6:
                    e["mobs"].append(mb)

    for line in section.splitlines():
        optional = "OPTIONAL" in line.upper()
        # dash-bullet: * [[Item]] - [[Mob]] rest-is-zone (Paladin's shape)
        dm = re.match(
            r"^\*+\s*'*" + LINK + r"'*\s*[-\u2013\u2014]\s*" + LINK + r"(.*)$", line
        )
        if dm:
            zone = None
            zm = re.search(LINK, dm.group(3))
            if zm:
                zone = zm.group(1).strip()
            else:
                tm = re.search(r"(?:in|,)\s+(?:the\s+)?([A-Z][A-Za-z' ]{3,30})", dm.group(3))
                if tm:
                    zone = tm.group(1).strip()
            add(dm.group(1).strip(), [dm.group(2).strip()], zone, 1, optional)
            continue
        # 'Kill [[Mob]] (drops [[X]] & [[Y]])' -- Shaman's shape
        for pm in re.finditer(LINK + r"\s*\(drops?\s+([^)]*)\)", line):
            mob = pm.group(1).strip()
            zone = None
            zm = re.search(r"\bin\s+" + LINK, line[pm.end():])
            if zm:
                zone = zm.group(1).strip()
            for dm2 in re.finditer(LINK, pm.group(2)):
                add(dm2.group(1).strip(), [mob], zone, 1, optional)
        # 'Get/Pickpocket [[Item]] from [[Mob]] in [[Zone]] or ...' -- Rogue's shape
        gm = re.search(
            r"\b(Get|Pickpocket|Acquire)\s+(?:an?\s+)?" + LINK + r"\s+from\s+(.*)$", line, re.I
        )
        if gm:
            tail = gm.group(3)
            zones = [z.group(1).strip() for z in re.finditer(r"\bin\s+" + LINK, tail)]
            mobs = [
                m2.group(1).strip()
                for m2 in re.finditer(LINK, tail)
                if m2.group(1).strip() not in zones
            ]
            src = "pickpocket" if gm.group(1).lower() == "pickpocket" else None
            if not mobs:
                continue
            add(gm.group(2).strip(), mobs, zones[0] if zones else None, 1, optional, source=src)
            continue
        # '[[Item]]/{{:Item}} drops from [[Mob]] ... in [[Zone]]'
        dfm = re.search(
            r"(?:\{\{:([^}|]+)\}\}|" + LINK
            + r")[^\[.]{0,18}dropp?(?:s|ed)?\s+(?:off\s+of|off|from|by)\s+([^.]*)",
            line,
        )
        if dfm:
            item = (dfm.group(1) or dfm.group(2)).strip()
            tail = dfm.group(3)
            zone = None
            zm = re.search(r"\bin\s+" + LINK, tail)
            if zm:
                zone = zm.group(1).strip()
            mobs = [
                m2.group(1).strip()
                for m2 in re.finditer(LINK, tail)
                if not zone or m2.group(1).strip() != zone
            ]
            if mobs:
                add(item, mobs, zone, 1, optional)
                continue
        # 'Kill [[Mob]] ... for the [[Item]]' -- Necro's shape
        km = re.search(
            r"\b(?:kill|slay)\b(.*?)\bfor\s+(?:the\s+|a\s+)?" + LINK, line, re.I
        )
        if km and "loot" not in line.lower():
            zones = [z.group(1).strip() for z in re.finditer(r"\bin\s+" + LINK, km.group(1))]
            mobs = [
                m2.group(1).strip()
                for m2 in re.finditer(LINK, km.group(1))
                if m2.group(1).strip() not in zones
            ]
            if mobs:
                add(km.group(2).strip(), mobs, zones[0] if zones else None, 1, optional)
                continue
        # forage bullet: * [[Item]] from [[Zone]] -- no trade verbs
        fm = re.match(r"^\*+\s*" + LINK + r"\s+from\s+" + LINK + r"\s*$", line)
        if fm and not re.search(r"receive|give|hand|buy|return", line, re.I):
            add(fm.group(1).strip(), [], fm.group(2).strip(), 1, optional, source="forage")
            continue
        if not re.search(r"\bloots?\b", line, re.I):
            continue
        # mobs: links between a Kill/spawns verb and the first loot verb,
        # minus zone links ("in [[Zone]]")
        lootpos = re.search(r"\bloots?\b", line, re.I).start()
        head = line[:lootpos]
        # why: only kill-context loot -- 'receive X ... loot' lines with
        # no slay verb are NPC-handed intermediates, not farm targets
        if (
            not re.search(r"\b(kill|slay|spawns?|dies|death)\b", head, re.I)
            and len(re.sub(r"[*:'\s]", "", head)) > 8
        ):
            continue
        zone = None
        zm = None
        for zm_ in re.finditer(r"\bin\s+" + LINK, head):
            zm = zm_
        if zm:
            zone = zm.group(1).strip()
        mobs = []
        for lm in re.finditer(LINK, head):
            name = lm.group(1).strip()
            if zone and name == zone:
                continue
            if name.lower().startswith(("file:", "category:")):
                continue
            mobs.append(name)
        for im in re.finditer(
            r"\bloots?\b[^\[]{0,25}?(?:(\d+)\s*x?\s*)?" + LINK, line, re.I
        ):
            qty = int(im.group(1)) if im.group(1) else 1
            name = im.group(2).strip()
            if name.lower().startswith(("file:", "category:")):
                continue
            add(name, mobs, zone, qty, optional)
    return [items[n] for n in order]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--from-cache-scan", metavar="DIR")
    args = ap.parse_args()

    wanted = {c + " Epic Quest" for c in CLASSES} | {"Class Epic Quest List"}
    pages = {}
    if args.from_cache_scan:
        pages = scan_cache_dir(os.path.expanduser(args.from_cache_scan), wanted)
        missing = wanted - set(pages)
        if missing:
            print(f"cache scan missing {sorted(missing)}, fetching", file=sys.stderr)
    for t in sorted(wanted - set(pages)):
        pages[t] = fetch_page(t)

    # why: the list page's reward column is the canonical final weapon
    # per class (Berserker's own page has no Reward section at all)
    final_rewards = {}
    cur = None
    for line in pages["Class Epic Quest List"].splitlines():
        m = re.search(r"\[\[([A-Za-z ]+) Epic Quest\]\]", line)
        if m:
            cur = m.group(1).strip()
            continue
        m = re.search(r"\{\{:([^}|]+)\}\}", line)
        if m and cur:
            final_rewards[cur] = m.group(1).strip()
            cur = None

    # why: a zone link caught in an item slot ("acquire ... from the
    # [[City of Mist]]") is provably not an item -- zones.json is the authority
    zones_pack = os.path.join(os.path.dirname(__file__), "..", "..", "packs", "zones.json")
    with open(zones_pack, encoding="utf-8") as f:
        zone_names = {z["name"] for z in json.load(f)["zones"]}

    classes = []
    for c in CLASSES:
        text = pages[c + " Epic Quest"]
        section = farmable_text(text)
        classes.append(
            {
                "class": c,
                "page": c + " Epic Quest",
                "start_zone": header_field(text, "Start Zone"),
                "quest_giver": header_field(text, "Quest Giver"),
                "recommended_level": header_field(text, "Recommended Level"),
                "final_reward": final_rewards.get(c),
                "rewards": rewards_of(text),
                "items": [
                    it
                    for it in parse_loot_lines(section)
                    if it["item"] not in zone_names
                ],
            }
        )

    out = {
        "source": "https://eqlwiki.com/Class_Epic_Quest_List",
        "note": "Farmable loot-drop materials per class epic (loot-lines from "
        "each page's checklist). NPC-handed intermediates excluded on purpose: "
        "they need the era's quest NPCs and can't be pre-farmed.",
        "classes": classes,
    }
    dest = os.path.join(os.path.dirname(__file__), "..", "..", "packs", "epic_quests.json")
    with open(dest, "w", encoding="utf-8") as f:
        json.dump(out, f, indent=1, ensure_ascii=False)
        f.write("\n")
    for c in classes:
        print(f"{c['class']}: {len(c['items'])} items, rewards={c['rewards']}")


if __name__ == "__main__":
    main()

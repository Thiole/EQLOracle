#!/usr/bin/env python3
"""
scrape_eqlwiki_zones.py — build zones.json from eqlwiki.com, sibling to
scrape_eqlwiki_npcs.py. Same transport layer (batched MediaWiki API calls,
on-disk cache, thread pool); the parser differs because a zone page isn't
a `{{Namedmobpage|...}}`-style template call the way an NPC or item page
is -- it's a wikitable (`{| class="zoneTopTable" ... |}`) of `! label \\n
| value` rows, plus a block of narrative prose above it.

Deliberately does NOT capture that narrative prose (confirmed by sampling
both a city page, Ak'Anon, and a dungeon page, Befallen: several
paragraphs of descriptive lore text precede the table on every zone page
checked). Only the structured table -- level range, adjacent zones, notable
NPCs, spawn timer, and whatever else that particular zone's table happens
to carry -- and the zone's own embedded `[[File:...|thumb|...]]` screenshot
filename, if the page has one. The two zone shapes sampled (city, dungeon)
carry *different* field sets (a city has "City Races"/"Guilds", a dungeon
has "Level of Monsters"/"Types of Monsters") with no common schema between
them, so the table parser is generic -- whatever `! label | value` pairs
exist on a given page, not a fixed field list assumed up front.

Usage:
    pip install requests
    python scrape_eqlwiki_zones.py                # full pull
    python scrape_eqlwiki_zones.py --limit 20      # quick test
"""

import argparse
import json
import os
import re
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor, as_completed

try:
    import requests
except ImportError:
    sys.exit("Missing deps. Run:  pip install requests")

API = "https://eqlwiki.com/api.php"
CACHE = ".eqlcache"
UA = "EQL-Gear-Planner/2.0 (personal build planner; contact: local user)"

BATCH = 50
DEFAULT_WORKERS = 8
ZONE_CATEGORY = "Zones"

_local = threading.local()
_throttle = threading.Semaphore(DEFAULT_WORKERS)


def session():
    s = getattr(_local, "s", None)
    if s is None:
        s = requests.Session()
        s.headers.update({"User-Agent": UA})
        _local.s = s
    return s


# ---------------- transport (identical to the other scrapers) ----------------

def cache_path(key):
    import hashlib
    os.makedirs(CACHE, exist_ok=True)
    return os.path.join(CACHE, hashlib.md5(key.encode()).hexdigest() + ".json")


def api_get(params, cache_key=None):
    if cache_key:
        p = cache_path(cache_key)
        if os.path.exists(p):
            try:
                with open(p, encoding="utf-8") as f:
                    return json.load(f)
            except Exception:
                pass

    params = dict(params, format="json", formatversion=2, maxlag=5)
    data = None
    for attempt in range(5):
        try:
            with _throttle:
                r = session().get(API, params=params, timeout=30)
            if r.status_code in (429, 503):
                time.sleep(2 ** attempt)
                continue
            r.raise_for_status()
            data = r.json()
            if "error" in data and data["error"].get("code") == "maxlag":
                time.sleep(2 ** attempt)
                continue
            break
        except Exception:
            if attempt == 4:
                raise
            time.sleep(2 ** attempt)

    if cache_key and data is not None:
        with open(cache_path(cache_key), "w", encoding="utf-8") as f:
            json.dump(data, f)
    return data


def category_members(cat):
    """Only direct pages (cmtype=page), not the subcategories -- Category:
    Zones has 122 subcats (per-expansion groupings) whose *members* would
    just be the same 118 zone pages again under a different path. Asking
    for pages only avoids double-walking them."""
    titles, cont = [], {}
    while True:
        params = {"action": "query", "list": "categorymembers",
                  "cmtitle": f"Category:{cat}", "cmlimit": "500",
                  "cmnamespace": "0", "cmtype": "page"}
        params.update(cont)
        data = api_get(params, cache_key=f"zonecat:{cat}:{cont.get('cmcontinue','')}")
        titles += [m["title"] for m in data.get("query", {}).get("categorymembers", [])]
        if "continue" in data:
            cont = data["continue"]
        else:
            return titles


def fetch_batch(titles):
    key = "zonebatch:" + "|".join(titles)
    out = {}
    cont = {}
    while True:
        params = {"action": "query", "prop": "revisions|categories",
                  "rvslots": "main", "rvprop": "content",
                  "cllimit": "max", "titles": "|".join(titles),
                  "redirects": "1"}
        params.update(cont)
        ck = key + ":" + json.dumps(cont, sort_keys=True) if cont else key
        data = api_get(params, cache_key=ck)
        for page in data.get("query", {}).get("pages", []):
            if page.get("missing"):
                continue
            t = page["title"]
            text, cats = out.get(t, (None, []))
            revs = page.get("revisions")
            if revs and text is None:
                text = revs[0]["slots"]["main"].get("content", "")
            cats = cats + [c["title"].replace("Category:", "").replace("_", " ")
                           for c in page.get("categories", [])]
            out[t] = (text, cats)
        if "continue" in data:
            cont = data["continue"]
        else:
            return out


# ---------------- wikitext parsing (shared helpers, identical to _spells.py) ----------------

def clean(s):
    s = re.sub(r"\{\{[^}]*\}\}", "", s or "")
    return re.sub(r"\s+", " ", s.replace("'''", "").replace("''", "")).strip()


def links(wikitext):
    seen, out = set(), []
    for m in re.finditer(r"\[\[([^\]|]+)(?:\|[^\]]*)?\]\]", wikitext or ""):
        t = clean(m.group(1))
        if t and t not in seen and not t.startswith(("File:", "Category:", "Image:")):
            seen.add(t); out.append(t)
    return out


def item_mentions(wikitext):
    """A list mixing `[[Item Name]]` links and `{{:Item Name}}`
    transclusions (both point at the same item pages, just two different
    wiki authoring habits) -- `links()` alone silently drops every
    `{{:...}}` entry (it's a template call, not a `[[...]]` link), which
    is what corrupted `unique_items` on the first pass into a run of bare
    commas. Order-preserving, de-duplicated, in whichever order the two
    forms actually appear in the source."""
    seen, out = set(), []
    for m in re.finditer(r"\[\[([^\]|]+)(?:\|[^\]]*)?\]\]|\{\{:([^}]+)\}\}", wikitext or ""):
        raw = m.group(1) or m.group(2)
        t = clean(raw)
        if t and t not in seen and not t.startswith(("File:", "Category:", "Image:")):
            seen.add(t); out.append(t)
    return out


# ---------------- zone-specific parsing ----------------

ZONE_TABLE_RE = re.compile(r"\{\|\s*class=[\"']zoneTopTable[\"'].*?\n(.*?)\n\|\}", re.DOTALL)
ZONE_ROW_RE = re.compile(r"!\s*'''\s*([^']+?)\s*:?\s*'''\s*\n\|(.*?)(?=\n\|-|\n!|\Z)", re.DOTALL)
ZONE_IMAGE_RE = re.compile(r"\[\[File:([^|\]]+)")
# A second, independent real source for the same fact: confirmed directly
# (Chardok (Pre-Revamp)) that a zone can have a genuinely *blank*
# `Succor/Evacuate` infobox cell while a separate prose section elsewhere
# on the same page states the real spot in plain text ("At exit to
# Burning Wood (119, 859)") -- used only as a fallback when the infobox
# field itself came back empty, never overrides a real infobox value.
SAFE_EVAC_SECTION_RE = re.compile(r"==\s*Safe/Evac Spot\s*==\s*\n(.*?)(?:\n==|\Z)", re.DOTALL)


def parse_zone_table(text):
    """`{| class="zoneTopTable" ! '''Label:''' | value |- ... |}` -> {label:
    raw value}. Field set varies by zone (a city's table and a dungeon's
    share no common labels -- see this module's docstring), so this
    returns whatever rows the page actually has, not a fixed schema.

    Merges *every* `zoneTopTable` block on the page, not just the first --
    a real, confirmed wiki-editing mistake on at least one live page
    (Blackburrow) closes the table early with a stray `|}` right after
    the "Unique Items" row, then re-opens a second `{| class="zoneTopTable"`
    for the rest (Adjacent Zones, Succor/Evacuate, ZEM Value, ...). The
    old single-`.search()` version silently returned only the first
    block's rows -- not a missing field, a whole second table's worth of
    real data dropped on the floor. Confirmed directly: this is exactly
    why Blackburrow's `succor_evacuate` came back `None` even though the
    real wikitext has a clean `-159, 39 (Zone line to Qeynos Hills)`."""
    fields = {}
    for m in ZONE_TABLE_RE.finditer(text or ""):
        for fm in ZONE_ROW_RE.finditer(m.group(1)):
            fields.setdefault(fm.group(1).strip(), fm.group(2).strip())
    return fields


def parse_zone(title, wikitext, cats):
    fields = parse_zone_table(wikitext)
    if not fields:
        return None

    def get(*names):
        """First matching field by any of `names` -- the same real-world
        fact ("Adjacent Zones" vs "Spawn timer" vs "[[Zone Spawn Timer]]")
        is labelled inconsistently across pages; confirmed by sampling,
        not assumed."""
        for n in names:
            if n in fields:
                return fields[n]
        return None

    def as_links(*names):
        v = get(*names)
        return links(v) if v else []

    def as_text(*names):
        v = get(*names)
        return clean(v) if v else None

    img_m = ZONE_IMAGE_RE.search(wikitext or "")

    # Real fallback, not a guess: an infobox `Succor/Evacuate` cell that's
    # blank on the wiki itself still has a real chance of a separate
    # `== Safe/Evac Spot ==` prose section elsewhere on the same page --
    # see `SAFE_EVAC_SECTION_RE`'s own doc for the confirmed real case
    # this fixes. Never overrides a real (non-blank) infobox value.
    succor_evacuate = as_text("Succor/Evacuate")
    if not succor_evacuate:
        m = SAFE_EVAC_SECTION_RE.search(wikitext or "")
        if m:
            succor_evacuate = clean(m.group(1).strip()) or None

    zone = {
        "id": title.replace(" ", "_"),
        "name": title,
        "url": "https://eqlwiki.com/" + title.replace(" ", "_"),
        "level_range": as_text("Level of Monsters"),
        "monster_types": as_text("Types of Monsters"),
        "notable_npcs": as_links("Notable NPCs"),
        "city_races": as_links("City Races"),
        "guilds": as_links("Guilds"),
        "tradeskill_facilities": as_links("Tradeskill Facilities"),
        "related_quests": as_links("Related Quests"),
        "unique_items": item_mentions(get("Unique Items") or ""),
        "adjacent_zones": as_links("Adjacent Zones"),
        "spawn_timer": as_text("Spawn timer", "[[Zone Spawn Timer]]"),
        "who_name": as_text("Name in /who"),
        "succor_evacuate": succor_evacuate,
        "image": img_m.group(1).strip() if img_m else None,
        "era": None,
        "categories": cats,
    }
    for c in cats:
        if c.endswith("Era"):
            zone["era"] = c
    return zone


# ---------------- main ----------------

def chunks(xs, n):
    for i in range(0, len(xs), n):
        yield xs[i:i + n]


# Real, confirmed wiki-editing artifacts miscategorized under
# Category:Zones -- not real playable zones. "Plane of Hate
# cleanupproject" is a stray cleanup-tagged duplicate of the real "Plane
# of Hate" page (confirmed directly: identical who_name and level_range,
# near-identical adjacent_zones, its own URL slug literally ends in
# "cleanupproject") -- filtered here so it never re-lands in zones.json
# on a future re-scrape, the same "small stated exceptions" shape
# ZONE_TABLE_RE's own doc uses for the analogous real-data-mess problem.
JUNK_TITLES = {"Plane of Hate cleanupproject"}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--workers", type=int, default=DEFAULT_WORKERS)
    ap.add_argument("--out", default="zones.json")
    args = ap.parse_args()

    global _throttle
    _throttle = threading.Semaphore(args.workers)

    print(f"Collecting titles from Category:{ZONE_CATEGORY}...")
    titles = sorted(set(category_members(ZONE_CATEGORY)) - JUNK_TITLES)
    if args.limit:
        titles = titles[:args.limit]
    batches = list(chunks(titles, BATCH))
    print(f"{len(titles)} zone pages -> {len(batches)} batched requests.\n")

    pages, done = {}, 0
    t0 = time.time()
    with ThreadPoolExecutor(max_workers=args.workers) as ex:
        futs = [ex.submit(fetch_batch, b) for b in batches]
        for f in as_completed(futs):
            try:
                pages.update(f.result())
            except Exception as e:
                print(f"  ! batch failed: {e}")
            done += 1
            print(f"  {done}/{len(batches)} batches  {len(pages)} pages", end="\r")

    print(f"\n\nFetched {len(pages)} pages in {time.time()-t0:.1f}s. Parsing...")

    zones, skipped = [], 0
    for t, (text, cs) in pages.items():
        try:
            z = parse_zone(t, text, cs)
        except Exception as e:
            print(f"  ! {t}: {e}")
            continue
        if not z:
            skipped += 1
            continue
        zones.append(z)

    zones.sort(key=lambda z: z["name"])
    payload = {
        "source": "https://eqlwiki.com",
        "scraped": time.strftime("%Y-%m-%d %H:%M"),
        "count": len(zones),
        "skipped_no_table": skipped,
        "zones": zones,
    }
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=1)
    print(f"Wrote {args.out}  ({len(zones)} zones, {skipped} pages with no zoneTopTable)")


if __name__ == "__main__":
    main()

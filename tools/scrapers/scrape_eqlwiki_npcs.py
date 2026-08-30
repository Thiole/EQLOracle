#!/usr/bin/env python3
"""
scrape_eqlwiki_npcs.py — build npcs.json from eqlwiki.com, sibling to
scrape_eqlwiki.py (items) and scrape_eqlwiki_spells.py (spells).

Reuses the same transport layer verbatim (batched MediaWiki API calls,
on-disk cache, thread pool) -- only the enumeration category and the
template parser differ, same split as the spell scraper.

NPCs live in one flat category (confirmed against the live wiki, not
assumed): action=query&list=categorymembers&cmtitle=Category:NPCs, 8,201
pages as of the check that sized this. "Category:Named Mobs" (6,549
pages) turned out to be the *same* pages under a second category, not a
disjoint set -- sampled three titles from each and got an identical list --
so this scrapes Category:NPCs alone; Named Mobs would just re-fetch what's
already here.

Each page is a {{Namedmobpage|...}} template call -- same shape as
{{Itempage|...}}/{{Spellpage|...}}, so split_template() needs no changes.
Empirically, not assumed: a page that isn't a Namedmobpage (a small
fraction -- vendors, quest-only NPCs, disambiguation pages) is skipped and
counted, not guessed at with a second template name, since a live check of
several such pages found no consistent second template to fall back to the
way Spellpagesmart exists for spells.

Deliberately does NOT scrape or store each zone page's narrative
description text (the paragraphs of lore prose at the top of a page like
Ak'Anon's) -- only the structured sidebar table fields (races, guilds,
adjacent zones, spawn timer, etc.). That split lives in
scrape_eqlwiki_zones.py, not here; this file is NPCs only.

Usage:
    pip install requests
    python scrape_eqlwiki_npcs.py                # full pull
    python scrape_eqlwiki_npcs.py --limit 200     # quick test
    python scrape_eqlwiki_npcs.py --workers 4     # be gentler
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
NPC_CATEGORY = "NPCs"

_local = threading.local()
_throttle = threading.Semaphore(DEFAULT_WORKERS)


def session():
    s = getattr(_local, "s", None)
    if s is None:
        s = requests.Session()
        s.headers.update({"User-Agent": UA})
        _local.s = s
    return s


# ---------------- transport (identical to scrape_eqlwiki.py / _spells.py) ----------------

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
    titles, cont = [], {}
    while True:
        params = {"action": "query", "list": "categorymembers",
                  "cmtitle": f"Category:{cat}", "cmlimit": "500",
                  "cmnamespace": "0"}
        params.update(cont)
        data = api_get(params, cache_key=f"npccat:{cat}:{cont.get('cmcontinue','')}")
        titles += [m["title"] for m in data.get("query", {}).get("categorymembers", [])]
        if "continue" in data:
            cont = data["continue"]
        else:
            return titles


def fetch_batch(titles):
    key = "npcbatch:" + "|".join(titles)
    out = {}
    cont = {}
    while True:
        params = {"action": "query", "prop": "revisions|categories|images",
                  "rvslots": "main", "rvprop": "content",
                  "cllimit": "max", "imlimit": "max",
                  "titles": "|".join(titles), "redirects": "1"}
        params.update(cont)
        ck = key + ":" + json.dumps(cont, sort_keys=True) if cont else key
        data = api_get(params, cache_key=ck)
        for page in data.get("query", {}).get("pages", []):
            if page.get("missing"):
                continue
            t = page["title"]
            text, cats, imgs = out.get(t, (None, [], []))
            revs = page.get("revisions")
            if revs and text is None:
                text = revs[0]["slots"]["main"].get("content", "")
            cats = cats + [c["title"].replace("Category:", "").replace("_", " ")
                           for c in page.get("categories", [])]
            imgs = imgs + [i["title"].replace("File:", "") for i in page.get("images", [])]
            out[t] = (text, cats, imgs)
        if "continue" in data:
            cont = data["continue"]
        else:
            return out


# ---------------- wikitext parsing (shared helpers, identical to _spells.py) ----------------

def split_template(text, name="Itempage"):
    """Return {param: value} for the first {{name|...}} call. Splits on
    top-level pipes only -- nested {{...}}/[[...]] are common and must not
    be split through."""
    m = re.search(r"\{\{\s*" + name + r"\s*(?=[|}])", text)
    if not m:
        return None
    i, depth = m.end(), 2
    parts, buf, brack = [], [], 0
    while i < len(text):
        two = text[i:i + 2]
        if two == "{{":
            depth += 2; buf.append(two); i += 2; continue
        if two == "}}":
            depth -= 2
            if depth <= 0:
                parts.append("".join(buf)); break
            buf.append(two); i += 2; continue
        if two == "[[":
            brack += 1; buf.append(two); i += 2; continue
        if two == "]]":
            brack -= 1; buf.append(two); i += 2; continue
        c = text[i]
        if c == "|" and depth == 2 and brack == 0:
            parts.append("".join(buf)); buf = []; i += 1; continue
        buf.append(c); i += 1
    else:
        parts.append("".join(buf))

    out = {}
    for p in parts:
        if "=" not in p:
            continue
        k, v = p.split("=", 1)
        out[k.strip()] = v.strip()
    return out


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


# ---------------- NPC-specific parsing ----------------

# `known_loot` is <ul><li> ... </li></ul>, one item per <li>, each shaped
# like one of:
#   [[Item Name]]  <span class='drare'>(Rare)</span>
#   {{:Item Name}}  <span class='drare'>(Uncommon)</span> <span class='ddb'>[2] 4x 55% (17%)</span>
# `{{:Name}}` is the same "transclude that item's own page" shape
# parse_items_with_effect already handles for spells; `[[Name]]` is a plain
# link. The rarity span and the bracketed drop-rate span are both optional
# -- plenty of loot lines carry no percentage, only a rarity word.
LOOT_LI_RE = re.compile(r"<li>\s*(.*?)\s*</li>", re.DOTALL)
RARITY_RE = re.compile(r"class=['\"]drare['\"]>\(([^)]+)\)")
DROP_PCT_RE = re.compile(r"class=['\"]ddb['\"]>\s*\[(\d+)\]\s*([\d.]+)x\s*([\d.]+)%\s*\(([\d.]+)%\)")


def parse_known_loot(raw):
    """<ul><li>...</li>...</ul> -> [{"item", "rarity", "stack", "chance_per_kill", "chance_per_drop"}].
    `chance_per_kill` is the outer, unparenthesized percent (chance this
    line drops at all on a kill that reaches loot); `chance_per_drop` is
    the parenthesized one (share of the corpse's loot rolls this item
    claims, given something dropped) -- two different questions the wiki's
    own markup already answers separately, kept separate here rather than
    collapsed into one number."""
    out = []
    for li in LOOT_LI_RE.finditer(raw or ""):
        block = li.group(1)
        item = None
        m = re.search(r"\{\{:([^}]+)\}\}", block)
        if m:
            item = clean(m.group(1))
        else:
            ls = links(block)
            if ls:
                item = ls[0]
        if not item:
            continue
        rarity_m = RARITY_RE.search(block)
        pct_m = DROP_PCT_RE.search(block)
        entry = {
            "item": item,
            "rarity": rarity_m.group(1) if rarity_m else None,
            "stack": None,
            "chance_per_kill": None,
            "chance_per_drop": None,
        }
        if pct_m:
            entry["stack"] = float(pct_m.group(2))
            entry["chance_per_kill"] = float(pct_m.group(3))
            entry["chance_per_drop"] = float(pct_m.group(4))
        out.append(entry)
    return out


def parse_npc(title, wikitext, cats, imgs):
    tpl = split_template(wikitext or "", name="Namedmobpage")
    if tpl is None:
        return None

    def field(name, cast=None):
        v = tpl.get(name, "").strip()
        if not v:
            return None
        v = clean(v)
        if cast is None:
            return v
        try:
            return cast(v)
        except ValueError:
            return None

    zone_links = links(tpl.get("zone", ""))
    class_links = links(tpl.get("class", ""))

    npc = {
        "id": title.replace(" ", "_"),
        "name": tpl.get("name", title).strip() or title,
        "url": "https://eqlwiki.com/" + title.replace(" ", "_"),
        "race": field("race"),
        # `class_links` first -- most values are a `[[Warrior]]`-style link
        # that `field()` alone would leave as raw markup; falls back to the
        # cleaned raw text for the free-text cases ("Multiple", "NPC
        # Non-quest") that aren't links at all. Always a single string
        # (joined if more than one class links, comma-separated) so this
        # field has one consistent type across every NPC, not a string for
        # some and a list for others.
        "class": ", ".join(class_links) if class_links else field("class"),
        "level": field("level"),  # kept as text -- often a range ("9-12"), not a single int
        "zone": zone_links[0] if zone_links else field("zone"),
        "location": field("location"),
        "respawn_time": field("respawn_time"),
        "aggro_radius": field("agro_radius", float),
        "run_speed": field("run_speed", float),
        "AC": field("AC", int),
        "HP": field("HP", int),
        "HP_regen": field("HP_regen", int),
        "mana_regen": field("mana_regen", int),
        "attacks_per_round": field("attacks_per_round", int),
        "attack_speed": field("attack_speed"),
        "damage_per_hit": field("damage_per_hit"),
        "special": field("special"),
        "known_loot": parse_known_loot(tpl.get("known_loot", "")),
        # The template's own `imagefilename` is what a page *claims* its
        # portrait is; `images` (from the API's own `prop=images` on this
        # page) is what's *actually* embedded, which is mostly the loot
        # table's item icons, not a monster portrait -- kept both, not
        # collapsed into one, so a consumer can tell "this page names a
        # portrait file" from "this page embeds N images" without
        # conflating the two. See this module's docstring.
        "imagefilename": tpl.get("imagefilename", "").strip() or None,
        "images": imgs,
        "era": None,
        "categories": cats,
    }
    for c in cats:
        if c.endswith("Era"):
            npc["era"] = c
    return npc


# ---------------- main ----------------

def chunks(xs, n):
    for i in range(0, len(xs), n):
        yield xs[i:i + n]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--workers", type=int, default=DEFAULT_WORKERS)
    ap.add_argument("--out", default="npcs.json")
    args = ap.parse_args()

    global _throttle
    _throttle = threading.Semaphore(args.workers)

    print(f"Collecting titles from Category:{NPC_CATEGORY}...")
    titles = sorted(set(category_members(NPC_CATEGORY)))
    if args.limit:
        titles = titles[:args.limit]
    batches = list(chunks(titles, BATCH))
    print(f"{len(titles)} NPC pages -> {len(batches)} batched requests.\n")

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

    npcs, skipped = [], 0
    for t, (text, cs, imgs) in pages.items():
        try:
            n = parse_npc(t, text, cs, imgs)
        except Exception as e:
            print(f"  ! {t}: {e}")
            continue
        if not n:
            skipped += 1
            continue
        npcs.append(n)

    npcs.sort(key=lambda n: n["name"])
    payload = {
        "source": "https://eqlwiki.com",
        "scraped": time.strftime("%Y-%m-%d %H:%M"),
        "count": len(npcs),
        "skipped_not_namedmobpage": skipped,
        "npcs": npcs,
    }
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=1)
    print(f"Wrote {args.out}  ({len(npcs)} NPCs, {skipped} pages not a Namedmobpage)")


if __name__ == "__main__":
    main()

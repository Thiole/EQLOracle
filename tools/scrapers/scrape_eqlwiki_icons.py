#!/usr/bin/env python3
"""
scrape_eqlwiki_icons.py — enriches items.json and spells.json with their
icon references, then downloads the real icon files.

Two different icon systems on eqlwiki, confirmed by sampling real pages,
not assumed:
  - Items: each Itempage template carries `lucy_img_ID = <number>`, and
    the page's own embedded image is `File:Item_<number>.png` -- one
    unique icon per item.
  - Spells: each Spellpage/Spellpagesmart template carries `spellicon =
    <code>` (a single letter/short code, e.g. "O"), and the page's own
    image is `File:Spellicon_<code>.png` -- a small *shared* icon set
    reused across many spells with the same code, not one per spell.

Neither field is in the existing items.json/spells.json (checked: absent
from both), so this re-fetches wikitext for every title already in those
two files (not a fresh category walk) just to pull the one new field, then
verifies + downloads whichever filenames actually resolve to a real
uploaded file -- confirmed empirically (checking items.json/npcs.json
directly, not guessing) that not every field value is backed by a real
upload.

Usage:
    python scrape_eqlwiki_icons.py --items items.json --spells spells.json --out-dir icons/
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

_local = threading.local()
_throttle = threading.Semaphore(DEFAULT_WORKERS)


def session():
    s = getattr(_local, "s", None)
    if s is None:
        s = requests.Session()
        s.headers.update({"User-Agent": UA})
        _local.s = s
    return s


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
                time.sleep(2 ** attempt); continue
            r.raise_for_status()
            data = r.json()
            if "error" in data and data["error"].get("code") == "maxlag":
                time.sleep(2 ** attempt); continue
            break
        except Exception:
            if attempt == 4:
                raise
            time.sleep(2 ** attempt)
    if cache_key and data is not None:
        with open(cache_path(cache_key), "w", encoding="utf-8") as f:
            json.dump(data, f)
    return data


def chunks(xs, n):
    for i in range(0, len(xs), n):
        yield xs[i:i + n]


def fetch_wikitext(titles, tag):
    """titles -> {title: raw wikitext}, batched. `tag` only scopes the
    on-disk cache key so item and spell re-fetches don't collide."""
    out = {}

    def one_batch(batch):
        key = f"{tag}icon:" + "|".join(batch)
        cont = {}
        local = {}
        while True:
            params = {"action": "query", "prop": "revisions", "rvslots": "main",
                      "rvprop": "content", "titles": "|".join(batch), "redirects": "1"}
            params.update(cont)
            ck = key + ":" + json.dumps(cont, sort_keys=True) if cont else key
            data = api_get(params, cache_key=ck)
            for page in data.get("query", {}).get("pages", []):
                if page.get("missing"):
                    continue
                revs = page.get("revisions")
                if revs:
                    local[page["title"]] = revs[0]["slots"]["main"].get("content", "")
            if "continue" in data:
                cont = data["continue"]
            else:
                return local

    batches = list(chunks(titles, BATCH))
    done = 0
    with ThreadPoolExecutor(max_workers=DEFAULT_WORKERS) as ex:
        futs = [ex.submit(one_batch, b) for b in batches]
        for f in as_completed(futs):
            out.update(f.result())
            done += 1
            print(f"  {tag}: {done}/{len(batches)} batches  {len(out)} pages", end="\r")
    print()
    return out


def check_files_exist(filenames):
    """Which of these File: titles actually resolve to an uploaded file --
    checked directly against the API, not assumed from the field being
    present. Returns the subset that's real."""
    real = []
    # why: "|" is the multi-title separator this API call joins with --
    # a filename containing one (garbage from a bad field parse, e.g.
    # the SPELLICON_RE/LUCY_ID_RE fix's own doc comment) splits mid-title
    # and silently corrupts every other title sharing its batch, not just
    # itself. A bad filename can never be a real upload anyway, so drop
    # it before batching rather than let it take good ones down with it.
    unique = sorted(fn for fn in set(filenames) if "|" not in fn)
    for batch in chunks(unique, BATCH):
        titles = "|".join("File:" + fn for fn in batch)
        data = api_get({"action": "query", "titles": titles, "prop": "imageinfo", "iiprop": "url"},
                        cache_key="filecheck:" + "|".join(batch))
        for p in data.get("query", {}).get("pages", []):
            if "missing" not in p:
                real.append(p["title"].replace("File:", ""))
    return set(real)


def download_files(filenames, out_dir):
    os.makedirs(out_dir, exist_ok=True)
    unique = sorted(set(filenames))

    def one(fn):
        dest = os.path.join(out_dir, fn)
        if os.path.exists(dest):
            return True
        data = api_get({"action": "query", "titles": "File:" + fn, "prop": "imageinfo", "iiprop": "url"},
                        cache_key="fileurl:" + fn)
        pages = data.get("query", {}).get("pages", [])
        if not pages or "imageinfo" not in pages[0]:
            return False
        url = pages[0]["imageinfo"][0]["url"]
        r = session().get(url, timeout=30)
        r.raise_for_status()
        with open(dest, "wb") as f:
            f.write(r.content)
        return True

    ok, done = 0, 0
    with ThreadPoolExecutor(max_workers=DEFAULT_WORKERS) as ex:
        futs = {ex.submit(one, fn): fn for fn in unique}
        for f in as_completed(futs):
            done += 1
            try:
                if f.result():
                    ok += 1
            except Exception as e:
                print(f"  ! {futs[f]}: {e}")
            print(f"  downloading: {done}/{len(unique)}  {ok} ok", end="\r")
    print()
    return ok


# why: `\s*` after `=` also swallows the newline into the *next* template
# line, so a blank field (nothing before the line break) greedily
# captured that next line's leading `|` as if it were the value --
# confirmed on a real page ("Laceration"'s empty `spellicon = `). A
# literal "|" title, once joined into a batched multi-title API query
# with "|" as the separator, corrupts that whole batch silently (splits
# mid-title), which is what actually caused most real spellicon codes to
# read back as "file doesn't exist" even though they do -- not a network
# or caching problem, a parsing one. `[ \t]*` stops at the line break, so
# a blank field now correctly fails to match instead of eating garbage.
LUCY_ID_RE = re.compile(r"lucy_img_ID[ \t]*=[ \t]*(\S+)")
SPELLICON_RE = re.compile(r"spellicon[ \t]*=[ \t]*(\S+)")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--items", default="items.json")
    ap.add_argument("--spells", default="spells.json")
    ap.add_argument("--out-dir", default="icons")
    ap.add_argument("--no-download", action="store_true", help="only enrich the JSON, skip the actual image fetch")
    args = ap.parse_args()

    # ---- items ----
    with open(args.items, encoding="utf-8") as f:
        items_doc = json.load(f)
    item_titles = [it["name"] for it in items_doc["items"]]
    print(f"Re-fetching wikitext for {len(item_titles)} items to pull lucy_img_ID...")
    item_pages = fetch_wikitext(item_titles, "item")

    item_icons = {}
    for it in items_doc["items"]:
        text = item_pages.get(it["name"], "")
        m = LUCY_ID_RE.search(text)
        icon = f"Item {m.group(1)}.png" if m else None
        it["icon"] = icon
        if icon:
            item_icons[it["name"]] = icon

    print(f"{len(item_icons)}/{len(item_titles)} items have a lucy_img_ID field.")
    real_item_icons = check_files_exist(item_icons.values())
    print(f"{len(real_item_icons)}/{len(set(item_icons.values()))} distinct item icon files actually exist on the wiki.")
    for it in items_doc["items"]:
        if it["icon"] and it["icon"] not in real_item_icons:
            it["icon"] = None  # field claimed one, but no real upload backs it -- honest null, not a broken reference

    with open(args.items, "w", encoding="utf-8") as f:
        json.dump(items_doc, f, indent=1)
    print(f"Updated {args.items} with icon field.\n")

    # ---- spells ----
    with open(args.spells, encoding="utf-8") as f:
        spells_doc = json.load(f)
    spell_titles = [sp["name"] for sp in spells_doc["spells"]]
    print(f"Re-fetching wikitext for {len(spell_titles)} spells to pull spellicon...")
    spell_pages = fetch_wikitext(spell_titles, "spell")

    spell_icons = {}
    for sp in spells_doc["spells"]:
        text = spell_pages.get(sp["name"], "")
        m = SPELLICON_RE.search(text)
        icon = f"Spellicon {m.group(1)}.png" if m else None
        sp["icon"] = icon
        if icon:
            spell_icons[sp["name"]] = icon

    print(f"{len(spell_icons)}/{len(spell_titles)} spells have a spellicon field.")
    distinct_spell_icons = set(spell_icons.values())
    print(f"({len(distinct_spell_icons)} distinct icon files across all of them -- a shared set, not one per spell.)")
    real_spell_icons = check_files_exist(distinct_spell_icons)
    print(f"{len(real_spell_icons)}/{len(distinct_spell_icons)} distinct spell icon files actually exist on the wiki.")
    for sp in spells_doc["spells"]:
        if sp["icon"] and sp["icon"] not in real_spell_icons:
            sp["icon"] = None

    with open(args.spells, "w", encoding="utf-8") as f:
        json.dump(spells_doc, f, indent=1)
    print(f"Updated {args.spells} with icon field.\n")

    if args.no_download:
        print("--no-download set, skipping actual image fetch.")
        return

    print(f"Downloading {len(real_item_icons)} item icons + {len(real_spell_icons)} spell icons to {args.out_dir}/ ...")
    n1 = download_files(real_item_icons, args.out_dir)
    n2 = download_files(real_spell_icons, args.out_dir)
    print(f"Done. {n1} item icons, {n2} spell icons saved to {args.out_dir}/.")


if __name__ == "__main__":
    main()

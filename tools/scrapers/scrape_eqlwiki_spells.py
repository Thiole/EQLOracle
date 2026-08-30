#!/usr/bin/env python3
"""
scrape_eqlwiki_spells.py — build spells.json from eqlwiki.com, sibling to
scrape_eqlwiki.py (items).

Reuses that script's transport layer verbatim (batched MediaWiki API calls,
on-disk cache, thread pool) -- only the enumeration category and the
template parser differ. See scrape_eqlwiki.py's docstring for why batching
matters; the same "~6,800 requests -> ~140" math applies here.

Spells live in one flat category, not one per slot like items:
    action=query&list=categorymembers&cmtitle=Category:Spells
and each page is a single {{Spellpage|...}} template call -- same shape as
{{Itempage|...}}, so split_template() needs no changes, just a different
`name=` argument.

Usage:
    pip install requests
    python scrape_eqlwiki_spells.py                # full pull
    python scrape_eqlwiki_spells.py --limit 200     # quick test
    python scrape_eqlwiki_spells.py --workers 4     # be gentler
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
SPELL_CATEGORY = "Spells"

_local = threading.local()
_throttle = threading.Semaphore(DEFAULT_WORKERS)


def session():
    s = getattr(_local, "s", None)
    if s is None:
        s = requests.Session()
        s.headers.update({"User-Agent": UA})
        _local.s = s
    return s


# ---------------- transport (identical to scrape_eqlwiki.py) ----------------

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
        data = api_get(params, cache_key=f"spellcat:{cat}:{cont.get('cmcontinue','')}")
        titles += [m["title"] for m in data.get("query", {}).get("categorymembers", [])]
        if "continue" in data:
            cont = data["continue"]
        else:
            return titles


def fetch_batch(titles):
    key = "spellbatch:" + "|".join(titles)
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


# ---------------- wikitext parsing (shared helpers, identical) ----------------

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


# ---------------- spell-specific parsing ----------------

NUM_RE = r"[+-]?\d+(?:\.\d+)?"


def parse_classes(raw):
    """'* [[Enchanter]] - Level 41\n* [[Mesmerist]] - Level 44' ->
    [{"class": "Enchanter", "level": 41}, ...]. Multi-line, one class per
    bullet -- a spell shared across a class line (Enchanter/Mesmerist, say)
    lists each separately with its own level."""
    out = []
    for line in (raw or "").splitlines():
        line = line.strip().lstrip("*").strip()
        if not line:
            continue
        cls = links(line)
        lvl = re.search(r"Level\s*(\d+)", line)
        if cls:
            out.append({"class": cls[0], "level": int(lvl.group(1)) if lvl else None})
    return out


def parse_slots(raw):
    """{{SpellSlotRow | 1 | Increase Poison Counter by 1 }} and
    {{SpellSlotRowSmart | 1 | Teleport group to -3685,1209,-5 in [[North
    Karana]] | simple = {{#ifeq:{{{Table|0}}}|0|0|1}} }} both occur in the
    wild -- the numbered effect lines a spell actually does, and for
    teleport-family spells (Gate/Portal/Translocate/Circle/Ring) this is
    where the wiki states the exact landing (x,y,z) in the destination
    zone. This is the closest the wiki gets to a machine-readable mechanic
    description generally, short of hand-classifying by name.

    Real, confirmed bug this replaces: the old single-regex version only
    matched literal `SpellSlotRow` -- never `SpellSlotRowSmart`, which
    turned out to be the dominant variant (1,097 of 1,928 real spells had
    *any* non-empty slots at all under the old regex; confirmed via direct
    wikitext fetch on North Karana Gate / North Karana Portal / Circle of
    Karana / Translocate: North Karana that all four use the Smart
    variant). Even Smart-aware, a single regex can't survive it: the
    `simple = {{#ifeq:{{{Table|0}}}|0|0|1}}` parameter nests `{{...}}` and
    `{{{...}}}` inside the row, and a naive `[^}]+?` capture truncates on
    the first stray `}` from that nesting, producing garbage. Fixed by
    reusing split_template's own brace-depth-counting approach instead of
    one regex -- see that function's doc for the same technique."""
    out = []
    for m in re.finditer(r"\{\{\s*SpellSlotRow(?:Smart)?\s*(?=[|}])", raw or ""):
        i, depth = m.end(), 2
        parts, buf = [], []
        while i < len(raw):
            two = raw[i:i + 2]
            if two == "{{":
                depth += 2; buf.append(two); i += 2; continue
            if two == "}}":
                depth -= 2
                if depth <= 0:
                    parts.append("".join(buf)); break
                buf.append(two); i += 2; continue
            c = raw[i]
            if c == "|" and depth == 2:
                parts.append("".join(buf)); buf = []; i += 1; continue
            buf.append(c); i += 1
        else:
            parts.append("".join(buf))
        # Positional fields only -- a leading empty part is an artifact of
        # the lookahead match end (mirrors split_template's own leading-
        # garbage-part tolerance), and a trailing `simple = ...` is a named
        # param, not a second positional field.
        positional = [p.strip() for p in parts if p.strip() and "=" not in p]
        if len(positional) < 2:
            continue
        try:
            slot = int(positional[0])
        except ValueError:
            continue
        out.append({"slot": slot, "effect": clean(positional[1])})
    return out


def parse_items_with_effect(raw):
    """<ul><li>{{:Item Name}}</li>...</ul> -> item page titles. Cross-links
    back to scrape_eqlwiki.py's item `effects` block -- an item's `proc`/
    `click`/`worn` effect name should match a spell scraped here."""
    return [clean(m.group(1)) for m in re.finditer(r"\{\{:([^}]+)\}\}", raw or "")]


def parse_spell(title, wikitext, cats):
    # Two template names in real use on eqlwiki: {{Spellpage}} for a normal
    # spell, {{Spellpagesmart}} for AA-adjacent passives/focus effects that
    # got pulled into Category:Spells too (confirmed on "Affliction
    # Efficiency I" -- has Table/TableLevel params Spellpage doesn't, but
    # the fields we read are identical otherwise). split_template's regex
    # requires the char right after the name to be '|' or '}', so it will
    # not accidentally match "Spellpagesmart" when asked for "Spellpage".
    tpl = split_template(wikitext or "", name="Spellpage")
    if tpl is None:
        tpl = split_template(wikitext or "", name="Spellpagesmart")
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

    spell = {
        "id": title.replace(" ", "_"),
        "name": tpl.get("spellname", title).strip() or title,
        "url": "https://eqlwiki.com/" + title.replace(" ", "_"),
        "description": field("description"),
        "classes": parse_classes(tpl.get("classes", "")),
        "skill": field("skill"),
        "mana": field("mana", int),
        "range": field("range", int),
        "casting_time": field("casting_time", float),
        "fizzle_time": field("fizzle_time", float),
        "recast_time": field("recast_time", float),
        "duration": field("duration"),
        "target_type": field("target_type"),
        "spell_type": field("spell_type"),
        # "Unresistable" here is authoritative -- ground truth for spells
        # like the Tash line that never produce a resisted-line in the log
        # because there's nothing to resist.
        "resist": field("resist"),
        # The exact client text, per outcome -- feeds a dictionary
        # classifier for the eqlp rule pack's remaining unmatched lines,
        # and (for beneficial/no-damage spells) the only landed-confirmation
        # signal that exists for them at all.
        "msg_cast_on_you": field("msg_cast_on_you"),
        "msg_cast_on_other": field("msg_cast_on_other"),
        "msg_wears_off": field("msg_wears_off"),
        "slots": parse_slots(tpl.get("slots", "")),
        "items_with_effect": parse_items_with_effect(tpl.get("items_with_effect", "")),
        "where_to_obtain": field("where_to_obtain"),
        "era": None,
        "categories": cats,
    }
    for c in cats:
        if c.endswith("Era"):
            spell["era"] = c
    return spell


# ---------------- main ----------------

def chunks(xs, n):
    for i in range(0, len(xs), n):
        yield xs[i:i + n]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--workers", type=int, default=DEFAULT_WORKERS)
    ap.add_argument("--out", default="spells.json")
    args = ap.parse_args()

    global _throttle
    _throttle = threading.Semaphore(args.workers)

    print(f"Collecting titles from Category:{SPELL_CATEGORY}...")
    titles = sorted(set(category_members(SPELL_CATEGORY)))
    if args.limit:
        titles = titles[:args.limit]
    batches = list(chunks(titles, BATCH))
    print(f"{len(titles)} spell pages -> {len(batches)} batched requests.\n")

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

    spells, skipped = [], 0
    for t, (text, cs) in pages.items():
        try:
            sp = parse_spell(t, text, cs)
        except Exception as e:
            print(f"  ! {t}: {e}")
            continue
        if not sp:
            skipped += 1
            continue
        spells.append(sp)

    spells.sort(key=lambda s: s["name"])
    payload = {
        "source": "https://eqlwiki.com",
        "scraped": time.strftime("%Y-%m-%d %H:%M"),
        "count": len(spells),
        "spells": spells,
    }
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=1)
    print(f"Wrote {args.out}  ({len(spells)} spells, {skipped} pages not a Spellpage)")


if __name__ == "__main__":
    main()

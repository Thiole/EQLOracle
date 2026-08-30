#!/usr/bin/env python3
"""
scrape_eqlwiki_aa.py — build aa.json from eqlwiki.com's "Alternate
Advancement" index page.

Different shape from items/spells: AAs have no individual wiki page and no
{{Itempage}}/{{Spellpage}} template. The whole catalog lives in wikitable
rows under one page's section headings -- General AAs, Archetype AAs, and
one Class AAs subsection per class (Bard, Enchanter, Wizard, ...). Category
membership (general / archetype / which class) comes from *which section a
row is in*, not a column -- confirmed by fetching the section list and two
real sections directly (see chat history, not reproduced here).

Table shape, same in every section:
    ! Name !! Ranks !! Cost !! Description
    |-
    | <name> || <ranks> || <cost, possibly "3/6/9"> || <description, often
      ending "Requirements: Level 12/30/50.">

One rank's cost/level don't line up column-for-column with anything else in
the row -- they're both slash-separated lists parsed positionally against
`ranks`, on the assumption ordinals match (rank 1 = first cost = first
level). Flagged per-row (`certain: False`) when that assumption doesn't
hold (count mismatch) rather than silently mis-pairing them.

Usage:
    pip install requests
    python scrape_eqlwiki_aa.py
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
PAGE = "Alternate Advancement"


def get(params):
    r = requests.get(API, params=dict(params, format="json", formatversion=2),
                      headers={"User-Agent": UA}, timeout=30)
    r.raise_for_status()
    return r.json()


def list_sections():
    """title -> (index, category), where category is 'general' | 'archetype'
    | '<Class Name>'. Special AAs are skipped -- confirm shape before
    including them; they may not follow the same table layout."""
    data = get({"action": "parse", "page": PAGE, "prop": "sections"})
    out = {}
    for s in data["parse"]["sections"]:
        line = s["line"]
        if line == "General AAs":
            out[s["index"]] = "general"
        elif line == "Archetype AAs":
            out[s["index"]] = "archetype"
        elif line.endswith("Class AAs") and s["toclevel"] == 2:
            out[s["index"]] = line[: -len(" Class AAs")]
    return out


ROW_RE = re.compile(
    r"^\|\s*(?P<name>[^|]+?)\s*\|\|\s*(?P<ranks>[^|]+?)\s*\|\|\s*(?P<cost>[^|]+?)\s*\|\|\s*(?P<desc>.+?)\s*$",
    re.M,
)
LEVEL_RE = re.compile(r"Requirements?:\s*[Ll]evel\s*([0-9/]+)")


def parse_table(wikitext, category):
    rows = []
    for m in ROW_RE.finditer(wikitext):
        name = m.group("name").strip()
        if name in ("Name",):  # header row, in case it slips through
            continue
        try:
            ranks = int(m.group("ranks").strip())
        except ValueError:
            continue  # not a data row (stray table markup)

        cost_raw = m.group("cost").strip()
        desc = m.group("desc").strip()
        lvl_m = LEVEL_RE.search(desc)
        level_raw = lvl_m.group(1) if lvl_m else ""

        costs = [c.strip() for c in cost_raw.split("/") if c.strip()]
        levels = [l.strip() for l in level_raw.split("/") if l.strip()]
        certain = len(costs) in (1, ranks) and len(levels) in (0, 1, ranks)

        per_rank = []
        for i in range(ranks):
            per_rank.append({
                "rank": i + 1,
                "cost": costs[i] if len(costs) == ranks else (costs[0] if costs else None),
                "level": int(levels[i]) if len(levels) == ranks and levels[i].isdigit()
                else (int(levels[0]) if len(levels) == 1 and levels[0].isdigit() else None),
            })

        rows.append({
            "name": name,
            "category": category,  # "general" | "archetype" | class name
            "ranks": ranks,
            "cost_raw": cost_raw,
            "certain": certain,
            "per_rank": per_rank,
            "description": desc,
        })
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="aa.json")
    args = ap.parse_args()

    sections = list_sections()
    print(f"Found {len(sections)} sections to pull: {sorted(set(sections.values()))}")

    all_rows = []
    for idx, category in sections.items():
        data = get({"action": "parse", "page": PAGE, "section": idx, "prop": "wikitext"})
        wikitext = data["parse"]["wikitext"]
        if isinstance(wikitext, dict):  # formatversion quirk on some MW installs
            wikitext = wikitext.get("*", "")
        rows = parse_table(wikitext, category)
        print(f"  {category:12s} section {idx}: {len(rows)} AAs")
        all_rows.extend(rows)

    uncertain = [r for r in all_rows if not r["certain"]]
    if uncertain:
        print(f"\n{len(uncertain)} rows have a rank/cost/level count mismatch -- "
              f"per_rank pairing may be wrong for these, check by hand:")
        for r in uncertain:
            print(f"  {r['category']:12s} {r['name']}  (ranks={r['ranks']}, cost_raw={r['cost_raw']!r})")

    payload = {
        "source": "https://eqlwiki.com/Alternate_Advancement",
        "count": len(all_rows),
        "aas": all_rows,
    }
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(payload, f, indent=1)
    print(f"\nWrote {args.out} ({len(all_rows)} AAs, {len(uncertain)} flagged uncertain)")


if __name__ == "__main__":
    main()

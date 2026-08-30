#!/usr/bin/env python3
"""why: recipe catalogs for the 9 core tradeskills -- item, ingredients,
implements/container, yield, trivial level, use/effect. Feeds eqlp's
Tradeskill module (per-skill recipe browser + a real Craft Log joining
craft.success/craft.failure log lines against this catalog by output
item name).

Two real, incompatible table shapes exist across these 9 pages (not a
bug, just how the wiki was hand-edited over the years):
  - raw HTML <table> (Baking, ...): rows are <tr><td>...</td>...</tr>,
    ingredients as a bulleted "* N [[Item]]" list (has quantities) OR a
    comma-separated "[[Item]], [[Item]]" list (no quantities shown).
  - MediaWiki pipe-table (Blacksmithing, ...): "{| ... |}", cells either
    "!!"/"||"-joined on one line or one cell per line (both real, same
    page even -- see parse_pipe_tables' own doc).
Both are parsed; a row/table that fits neither recognizable shape is
skipped and counted, not guessed at.

Real, confirmed-not-a-bug wiki quirk: some tradeskills (Blacksmithing's
"melt this back down" rows especially) have *multiple distinct recipes
sharing the same output item name* (several different old weapons all
melt down into "Small Piece of Ore", for instance). Recipes are kept as
a list, not a name-keyed map -- deduping by name alone would silently
keep only one and misrepresent the rest as if that were the only way to
make that item. Only an exact (name, ingredient-set) duplicate collapses
(the real case of a recipe appearing in two different tables on the
same page, e.g. Baking's own "Quick Leveling Guide" excerpt + its full
recipe table).

Trivial: some pages carry two era columns ("P99 Trivial"/"Classic
Trivial") -- P99 preferred (this project's other era-sensitive code
already treats EQL as P99-ruleset, see gearplanner.rs's on_server doc).
Not always a clean integer ("?", "??", "<67", ">41<67" all appear for
real) -- kept as the raw string when it doesn't parse as a plain int,
never guessed into a number.

input: none (network)
output: tradeskills.json -- {source, scraped, skills: [{skill, recipes: [...]}]}
"""
import argparse
import datetime
import html
import json
import re
import sys
import time
import urllib.parse
import urllib.request

API = "https://eqlwiki.com/api.php"
UA = "eqlp-tradeskill-scraper/1.0 (local research tool)"

# why: exact wiki page titles, confirmed via Category:Tradeskills --
# "Jewelcrafting" (not "Skill Jewelcrafting") is the real one, everything
# else is "Skill <Name>"
PAGES = {
    "Alchemy": "Skill Alchemy",
    "Baking": "Skill Baking",
    "Blacksmithing": "Skill Blacksmithing",
    "Brewing": "Skill Brewing",
    "Fletching": "Skill Fletching",
    "Jewelcrafting": "Jewelcrafting",
    "Pottery": "Skill Pottery",
    "Tailoring": "Skill Tailoring",
    "Tinkering": "Skill Tinkering",
}


def fetch_wikitext(title: str) -> str:
    url = API + "?action=parse&page=" + urllib.parse.quote(title) + "&prop=wikitext&format=json"
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    for attempt in range(4):
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                data = json.loads(r.read().decode("utf-8"))
            if "error" in data:
                return ""
            return data["parse"]["wikitext"]["*"]
        except Exception:
            if attempt == 3:
                raise
            time.sleep(2**attempt)
    return ""


def strip_wikilinks(s: str) -> str:
    # why: {{:Item}} (transclusion) and [[Item]]/[[Item|label]] (link) --
    # both just mean "the item named Item" for our purposes
    s = re.sub(r"\{\{:([^}]+)\}\}", r"\1", s)
    s = re.sub(r"\[\[([^\|\]]+)\|[^\]]*\]\]", r"\1", s)
    s = re.sub(r"\[\[([^\]]+)\]\]", r"\1", s)
    s = re.sub(r"<[^>]+>", " ", s)  # drop leftover html tags (<b>, <p>, etc.)
    s = html.unescape(s)
    return re.sub(r"\s+", " ", s).strip()


def parse_ingredient_list(raw: str) -> list[dict]:
    """why: two real shapes -- bulleted "* N [[Item]]" (has qty) and a
    comma/"Nx"-prefixed flat list (qty only if an "Nx"/"N " prefix is
    present, else 1 assumed -- the wiki just doesn't always say)."""
    # why: a trailing "(Yield: N)" describes the *whole recipe's* own
    # output count, not the last-listed ingredient -- strips clean
    # before ingredient splitting so it never bleeds into that name
    raw = re.sub(r"\(Yield:\s*\d+\)\s*$", "", raw.strip(), flags=re.I).strip()
    out = []
    if "*" in raw:
        for line in raw.split("*"):
            line = line.strip()
            if not line:
                continue
            m = re.match(r"^(\d+)\s+(.*)$", line)
            qty = int(m.group(1)) if m else 1
            name = strip_wikilinks(m.group(2) if m else line)
            name = re.sub(r"\s*\(returned\)\s*$", "", name, flags=re.I)
            if name:
                out.append({"item": name, "qty": qty, "returned": "(returned)" in line.lower()})
        return out
    # comma-separated flat list
    for part in raw.split(","):
        part = part.strip()
        if not part:
            continue
        m = re.match(r"^(\d+)\s*x?\s+(.*)$", part, re.I)
        qty = int(m.group(1)) if m else 1
        rest = m.group(2) if m else part
        returned = "(returned)" in rest.lower()
        name = strip_wikilinks(re.sub(r"\(returned\)", "", rest, flags=re.I))
        if name:
            out.append({"item": name, "qty": qty, "returned": returned})
    return out


def parse_trivial(raw: str) -> tuple[int | None, str]:
    raw = strip_wikilinks(raw)
    m = re.match(r"^(\d+)$", raw)
    if m:
        return int(m.group(1)), raw
    return None, raw


def strip_item_qty(raw: str) -> tuple[str, int]:
    """why: an *output* item cell can itself carry a yield prefix
    ('2x [[Small Piece of Ore]]') instead of a separate Yield column --
    confirmed real on Blacksmithing's melt-down rows."""
    m = re.match(r"^(\d+)\s*x?\s+(.*)$", raw.strip(), re.I)
    if m:
        return strip_wikilinks(m.group(2)), int(m.group(1))
    return strip_wikilinks(raw), 1


def _dedup_key(r: dict) -> tuple:
    return (r["item"], tuple(sorted((i["item"], i["qty"]) for i in r["ingredients"])))


# why: real named component-slot columns used instead of one combined
# "Components" column -- confirmed real across multiple pages: Fletching
# arrows (Point/Shaft/Fletch/Nock) and bows (Wood/String/Tool/Cam),
# Blacksmithing-style tables (Mold), Jewelcrafting (Metal/Gem/"Imbued
# Gem"). Deliberately an allowlist, not "anything that isn't a known
# stat column" -- a real risk this avoids: Blacksmithing's own weapon-
# salvage table ("Item | Weapon | Trivial | ... sell price ...") isn't a
# combine recipe at all, it's a value-reference table, and would produce
# a real but *incomplete* duplicate recipe (missing the Sharpening Stone
# a combine like this always needs) if "Weapon" were treated as an
# ingredient slot on a denylist basis. A skipped table (stated, counted)
# beats a wrong one.
NAMED_SLOT_COLUMNS = {
    "point",
    "shaft",
    "fletch",
    "nock",
    "mold",
    "metal",
    "gem",
    "imbued gem",
    "wood",
    "string",
    "tool",
    "cam",
}


def parse_html_tables(wt: str, skill: str, skipped: list[str]) -> list[dict]:
    """why: real <table><tr><td>...</td></tr></table> blocks -- header
    cell text decides column meaning, tolerant of column order/count
    varying page to page (confirmed real: Baking's two tables on the
    same page have 3 and 6 columns respectively). Also tolerant of a
    section-caption row before the real header (confirmed real: Skill
    Fletching's "Arrow Recipes" colspan title row) -- the first row
    whose <th> cells include "item" is treated as the header, not
    blindly assumed to be row 0.

    Requires an ingredients source (a components/ingredients column, or
    at least one named slot column) before trusting a table as a real
    recipe table -- real bug this guards against: Jewelcrafting's own
    item-stat tables (Item/Triv/Cost/AC/.../Metal/Gem) used to be
    accepted with silently empty ingredients (no components column, no
    slot column recognized yet) instead of being skipped like the
    equivalent case already was in parse_pipe_tables. Also broadens
    trivial detection to a substring match ("Triv", not just the exact
    word "Trivial" -- Jewelcrafting's own abbreviation)."""
    recipes: dict[tuple, dict] = {}
    for table in re.findall(r"<table[^>]*>.*?</table>", wt, re.S | re.I):
        rows = re.findall(r"<tr[^>]*>(.*?)</tr>", table, re.S | re.I)
        header_row_i = None
        cols: list[str] = []
        for i, row in enumerate(rows):
            header_cells = [strip_wikilinks(c) for c in re.findall(r"<th[^>]*>(.*?)</th>", row, re.S | re.I)]
            lc = [h.lower() for h in header_cells]
            if "item" in lc:
                header_row_i = i
                cols = lc
                break
        if header_row_i is None:
            continue
        comp_idx = next((i for i, c in enumerate(cols) if "component" in c or "ingredient" in c), None)
        slot_idxs = [i for i, c in enumerate(cols) if c in NAMED_SLOT_COLUMNS]
        if comp_idx is None and not slot_idxs:
            skipped.append(f"{skill}: html table with no item+components column ({cols})")
            continue
        trivial_idx = next((i for i, c in enumerate(cols) if "p99" in c and "triv" in c), None)
        if trivial_idx is None:
            trivial_idx = next((i for i, c in enumerate(cols) if "triv" in c), None)
        item_idx = cols.index("item")
        for row in rows[header_row_i + 1 :]:
            cells = re.findall(r"<td[^>]*>(.*?)</td>", row, re.S | re.I)
            if len(cells) < 2:
                continue
            cells = [strip_wikilinks(c) for c in cells]
            if item_idx >= len(cells):
                continue
            name, out_qty = strip_item_qty(cells[item_idx])
            if not name:
                skipped.append(f"{skill}: unnamed row")
                continue
            if comp_idx is not None and comp_idx < len(cells):
                ingredients = parse_ingredient_list(cells[comp_idx])
            else:
                ingredients = []
                for si in slot_idxs:
                    if si < len(cells) and cells[si].strip():
                        ingredients.extend(parse_ingredient_list(cells[si]))
            trivial_n, trivial_raw = (None, None)
            if trivial_idx is not None and trivial_idx < len(cells):
                trivial_n, trivial_raw = parse_trivial(cells[trivial_idx])
            implements_idx = next((i for i, c in enumerate(cols) if "implement" in c), None)
            yield_idx = next((i for i, c in enumerate(cols) if c == "yield"), None)
            use_idx = next((i for i, c in enumerate(cols) if c == "use"), None)
            r = {
                "item": name,
                "yield_qty": out_qty,
                "ingredients": ingredients,
                "implements": (cells[implements_idx] or None) if implements_idx is not None and implements_idx < len(cells) else None,
                "yield": (cells[yield_idx] or None) if yield_idx is not None and yield_idx < len(cells) else None,
                "trivial": trivial_n,
                "trivial_raw": trivial_raw,
                "use": (cells[use_idx] or None) if use_idx is not None and use_idx < len(cells) else None,
            }
            recipes[_dedup_key(r)] = r
    return list(recipes.values())


def strip_cell_attrs(cell: str) -> str:
    """why: a data/header cell can start with a wiki style attribute
    before its real content ('style="text-align: left;" | [[Item]]') --
    strip that prefix specifically, not "everything before the first |"
    (a piped wikilink like [[A|B]] has its own unrelated | inside)."""
    m = re.match(r'^\s*[a-zA-Z-]+\s*=\s*"[^"]*"(?:\s+[a-zA-Z-]+\s*=\s*"[^"]*")*\s*\|(?!\|)\s*(.*)$', cell, re.S)
    return m.group(1) if m else cell


def _line_cells(line: str, marker: str) -> list[str]:
    """why: a '!'/'|' line can hold one cell, or several joined by '!!'/'||'"""
    s = line.strip()
    if not s.startswith(marker):
        return []
    return [strip_cell_attrs(p.strip()) for p in re.split(r"!!|\|\|", s[len(marker):])]


def parse_pipe_tables(wt: str, skill: str, skipped: list[str]) -> list[dict]:
    """why: real MediaWiki "{| ... |}" tables. Two real header/row shapes
    coexist on the same page (confirmed: Skill Blacksmithing) -- cells
    joined by "!!"/"||" on one line, OR one cell per line -- both are
    handled by gathering cells from every matching line in a block, not
    assuming a single line holds the whole row.

    Requires BOTH a recognizable item-name column and a components/
    ingredients column before trusting a block as a real recipe table --
    several real sub-tables on these pages are armor-stat tables (item,
    AC, weight, a named base-material column) with no single combined
    "components" column at all; guessing which typed column is the
    ingredient would risk wrong data, so those are skipped and counted
    instead, same as any other unparseable shape."""
    recipes: dict[tuple, dict] = {}
    for block in re.findall(r"\{\|.*?\n\|\}", wt, re.S):
        lines = block.split("\n")
        header_cells: list[str] = []
        body_start = 0
        for i, ln in enumerate(lines):
            s = ln.strip()
            if s.startswith("!"):
                header_cells.extend(strip_wikilinks(c) for c in _line_cells(ln, "!"))
                body_start = i + 1
            elif s.startswith("{|") or s == "|-" or not s:
                continue
            else:
                break
        cols = [c.lower() for c in header_cells]
        item_idx = next((i for i, c in enumerate(cols) if c in ("item", "recipe") or c.endswith(" item")), None)
        comp_idx = next((i for i, c in enumerate(cols) if "component" in c or "ingredient" in c), None)
        # why: a real third shape (Alchemy) -- no single combined
        # ingredients column, instead one numbered "Ing. N" column per
        # slot (up to 8 seen); every non-empty one is an ingredient
        ing_slot_idxs = [i for i, c in enumerate(cols) if re.match(r"^ing\.?\s*\d+$", c)]
        if item_idx is None or (comp_idx is None and not ing_slot_idxs):
            skipped.append(f"{skill}: pipe-table with no item+components column ({header_cells})")
            continue
        # why: p99 preferred over any other era's trivial column -- see module doc
        trivial_idx = next((i for i, c in enumerate(cols) if "p99" in c and "trivial" in c), None)
        if trivial_idx is None:
            trivial_idx = next((i for i, c in enumerate(cols) if "trivial" in c), None)

        # why: the very first row's own "|-" separator has no *preceding*
        # newline (it's the first thing after the header, not mid-string)
        # -- prepending one lets a single split regex treat every row
        # boundary identically instead of special-casing row 1
        body = "\n" + "\n".join(lines[body_start:])
        for row_text in re.split(r"\n\|-[^\n]*", body):
            cells: list[str] = []
            for ln in row_text.split("\n"):
                cells.extend(_line_cells(ln, "|"))
            needed = max(x for x in [item_idx, comp_idx, *ing_slot_idxs] if x is not None)
            if len(cells) <= needed:
                continue
            name, out_qty = strip_item_qty(cells[item_idx])
            if not name:
                skipped.append(f"{skill}: unnamed pipe-table row")
                continue
            if comp_idx is not None:
                ingredients = parse_ingredient_list(cells[comp_idx])
            else:
                ingredients = []
                for si in ing_slot_idxs:
                    if si < len(cells) and cells[si].strip():
                        ingredients.extend(parse_ingredient_list(cells[si]))
            trivial_n, trivial_raw = (None, None)
            if trivial_idx is not None and trivial_idx < len(cells):
                trivial_n, trivial_raw = parse_trivial(cells[trivial_idx])
            r = {
                "item": name,
                "yield_qty": out_qty,
                "ingredients": ingredients,
                "implements": None,
                "yield": None,
                "trivial": trivial_n,
                "trivial_raw": trivial_raw,
                "use": None,
            }
            recipes[_dedup_key(r)] = r
    return list(recipes.values())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="tradeskills.json")
    ap.add_argument("--limit", type=int, default=None, help="only fetch first N skills (debugging)")
    args = ap.parse_args()

    skipped: list[str] = []
    skills_out = []
    items = list(PAGES.items())
    if args.limit:
        items = items[: args.limit]
    for skill, title in items:
        print(f"== {skill} ({title}) ==", file=sys.stderr)
        wt = fetch_wikitext(title)
        if not wt:
            print(f"  WARNING: empty/failed fetch for {title}", file=sys.stderr)
            continue
        html_recipes = parse_html_tables(wt, skill, skipped)
        pipe_recipes = parse_pipe_tables(wt, skill, skipped)
        # why: exact (name, ingredients) dupes across the two parse
        # passes collapse (the real case of a recipe listed in more than
        # one table on the same page); anything else -- including a
        # different recipe that merely shares an output name -- stays
        # its own row, see module doc
        seen = {_dedup_key(r) for r in html_recipes}
        merged = html_recipes + [r for r in pipe_recipes if _dedup_key(r) not in seen]
        print(f"  {len(merged)} recipes", file=sys.stderr)
        skills_out.append({"skill": skill, "recipes": sorted(merged, key=lambda r: r["item"])})

    total = sum(len(s["recipes"]) for s in skills_out)
    print(f"\nTotal: {total} recipes across {len(skills_out)} skills. Skipped {len(skipped)} unparseable rows.", file=sys.stderr)
    out = {
        "source": "https://eqlwiki.com",
        "scraped": datetime.datetime.now().strftime("%Y-%m-%d %H:%M"),
        "skills": skills_out,
        "skipped_count": len(skipped),
    }
    with open(args.out, "w") as f:
        json.dump(out, f, indent=1)
    print(f"wrote {args.out}", file=sys.stderr)


if __name__ == "__main__":
    main()

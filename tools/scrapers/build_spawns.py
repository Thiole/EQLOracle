#!/usr/bin/env python3
"""why: the in-game survey -> packs/spawns.json. A `/say %t` + `/loc`
macro writes two lines within a second; each pair is one sighting of
that mob at that spot, in the zone of the last zone line. A `/say invis`
right before a pair flags that sighting as seen while invisible. Same
name within MERGE_UNITS collapses into one entry with a higher count
(the surveyor may have keyed the same mob twice). Rebuilt from the
given logs every run (idempotent) -- pass every surveyed log together.
input:  one or more eqlog_*.txt
output: packs/spawns.json  [{zone, name, x, y, z, count, invis, first_seen, last_seen}]
run:    python3 tools/scrapers/build_spawns.py <log> [<log>...]
"""
import json, re, sys, os, math
from datetime import datetime

PAIR_MAX_S = 2
MERGE_UNITS = 4.0
NOTES = {"invis", "invisible", "inv"}
# why: `%t` with nothing targeted expands to the literal word "Target"
IGNORE = {"target"}

ROOT = os.path.join(os.path.dirname(__file__), "..", "..")
OUT = os.path.join(ROOT, "packs", "spawns.json")

TS = re.compile(r"^\[(\w{3} \w{3} \d{2} \d{2}:\d{2}:\d{2} \d{4})\] (.*)$")
SAY = re.compile(r"^You say, '(.*)'$")
LOC = re.compile(r"^Your Location is (-?[\d.]+), (-?[\d.]+), (-?[\d.]+)$")
ZONE = re.compile(r"^You have entered (.+?)\.$")

def parse(line):
    m = TS.match(line.rstrip("\r\n"))
    if not m:
        return None, None
    return datetime.strptime(m.group(1), "%a %b %d %H:%M:%S %Y"), m.group(2)

def survey(paths):
    sightings = []
    for path in paths:
        zone = None
        pending = None   # (ts, name)
        invis = False
        with open(path, errors="replace") as f:
            for line in f:
                ts, body = parse(line)
                if ts is None:
                    continue
                z = ZONE.match(body)
                if z:
                    zone = z.group(1)
                    pending = None
                    invis = False
                    continue
                s = SAY.match(body)
                if s:
                    said = s.group(1).strip()
                    if said.lower() in NOTES:
                        invis = True
                    elif said.lower() in IGNORE:
                        pending = None
                    else:
                        pending = (ts, said)
                    continue
                l = LOC.match(body)
                if l and pending and zone:
                    pts, name = pending
                    if (ts - pts).total_seconds() <= PAIR_MAX_S:
                        x, y, zz = (float(l.group(i)) for i in (1, 2, 3))
                        sightings.append((zone, name, x, y, zz, invis, ts))
                    pending = None
                    invis = False
    return sightings

def merge(entries, sightings):
    for zone, name, x, y, z, invis, ts in sightings:
        key = (zone, name.lower())
        hit = None
        for e in entries:
            if (e["zone"], e["name"].lower()) == key and math.hypot(e["x"] - x, e["y"] - y) <= MERGE_UNITS:
                hit = e
                break
        stamp = ts.strftime("%Y-%m-%d %H:%M:%S")
        if hit:
            # why: a repeat sighting refines the point (mean) and counts
            n = hit["count"]
            hit["x"] = round((hit["x"] * n + x) / (n + 1), 2)
            hit["y"] = round((hit["y"] * n + y) / (n + 1), 2)
            hit["z"] = round((hit["z"] * n + z) / (n + 1), 2)
            hit["count"] = n + 1
            hit["invis"] = hit["invis"] or invis
            hit["last_seen"] = max(hit["last_seen"], stamp)
            hit["first_seen"] = min(hit["first_seen"], stamp)
        else:
            entries.append({"zone": zone, "name": name, "x": x, "y": y, "z": z,
                            "count": 1, "invis": invis, "first_seen": stamp, "last_seen": stamp})
    return entries

def main():
    logs = sys.argv[1:]
    if not logs:
        print(__doc__); sys.exit(2)
    sightings = survey(logs)
    # why: rebuilt from the logs every run, never merged into the old
    # pack -- the log persists, so a re-run must give the same pack,
    # not doubled counts; pass every surveyed log together
    entries = merge([], sightings)
    entries.sort(key=lambda e: (e["zone"], e["name"].lower(), e["x"], e["y"]))
    with open(OUT, "w") as f:
        json.dump(entries, f, indent=1)
        f.write("\n")
    by_zone = {}
    for e in entries:
        by_zone.setdefault(e["zone"], []).append(e)
    print(f"{len(sightings)} sightings read; pack now {len(entries)} points in {len(by_zone)} zones")
    for z, es in sorted(by_zone.items()):
        names = sorted({e["name"] for e in es})
        print(f"  {z}: {len(es)} points, {len(names)} names, {sum(e['count'] for e in es)} sightings, {sum(1 for e in es if e['invis'])} invis")

if __name__ == "__main__":
    main()

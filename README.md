# EQL Oracle

Parsing & progression assistant for EverQuest Legends. Watches your `eqlog_<Character>_<Server>.txt`, replays what's already in it, keeps parsing live. Classes, AAs, spells, kills — all read off the log, nothing hand-entered. Runs entirely on your machine; nothing is uploaded.

Website: [eqloracle.com](https://eqloracle.com) · Discord: [discord.gg/mN6fwhBBk2](https://discord.gg/mN6fwhBBk2)

## Features

- **Combat** — live DPS meter, per-fight and aggregated views, team/incoming damage, fight timeline with scrub, ally table with pet attribution, per-ability breakdowns, cast outcomes (landed/resisted/interrupted/fizzled), parse history with per-loadout comparisons.
- **Death Recap** — the 30 seconds before each death: incoming damage by source and ability, avoided swings, heals received, killing blow. Opens from a timed prompt when you die.
- **Overlay** — separate always-on-top, click-through widgets over the game: DPS meter, Skill Tracker (status effects, cooldowns, target effects), Drop Watch, CC Tracker (stun/root/fear). Per-widget opacity and position, X11/XWayland.
- **Drop Watch** — track items; when anything in the current engagement can drop one (its own drop table, NPC loot data, or a zone-wide drop), the overlay says so. Prompts to untrack once you loot it.
- **Character** — sheet, gear planner, AA log, known spells, spellbook builder with damage-spell auto-suggest (rank-aware, invocation-aware, simulated rotation), inventory browser with "where is my X" item lookup from `/outputfile inventory` dumps.
- **Class detection** — infers your active class trio from casts, stances, and AAs, per zone visit. No manual entry.
- **Endgame** — raid boss/miniboss kill counts, drop tables vs. what you've looted, solo/group tiers separately, fastest clears. Plane of Sky class unlocks and quest tracking with confirmed turn-in detection.
- **Tradeskill** — recipe catalog and a craft log built from your own combines.
- **Game Data** — zones, items, NPCs, AAs, spells, cross-linked, with your own encounter and loot history per page.
- **Maps** — zone maps with NPC markers, your position from `/loc`, teleport-aware routing between zones.
- **Social** — guild/party/raid chat history and PM threads, read from the log.
- **Session** — plat/hour, XP/hour, motes by tier, AA spent, ETA to next level. Resets on AFK return or on demand.

## The parser

Everything above is a view onto one parse. That parse is the part worth explaining.

**Rules are data.** 188 rules live in `packs/eql.toml`, each carrying the real log lines it was written from. `eqlp shapes` reads a log and reports the line formats nothing matches yet, so a new rule starts from evidence instead of a guess. `eqlp lint` rejects a rule that shadows another, and CI measures the pack against a 6 MB reference slice — coverage cannot regress. Adding a line format is a data change, not a rebuild.

**Written for this game.** EverQuest Legends' log diverges from classic EQ's. Every pattern here came from a line this game actually emitted.

**Usable before it's finished.** A launch serves off the tail of your log straight away and folds the full history behind it. You see your last fight immediately, not after the whole file has been read.

**Fast, and deliberately not greedy.** Backfill spreads classification across up to 16 threads and merges in file order. The cap is on purpose: a backfill often runs while the game is live, and taking every core wins a benchmark and loses a frame rate.

**Small in memory.** Events land in a columnar store at about 46 bytes each, names interned once — a full log becomes well under half its own size in RAM (a 245 MB log is 102 MB of parsed state; measured, see below). Leaving a zone compacts its closed fights; totals, hits, crits, misses, DPS and every ability breakdown still answer identically afterwards.

**Every fight costs the same.** One event store, and aggregates are queries over it. The first fight of a session opens as fast as the most recent — both under 0.1 ms with 4,000 fights in the store. Nothing is capped by count, evicted, or downsampled to keep memory flat.

**Nothing derived is ever saved.** Each start re-reads the log and rebuilds from scratch, so no cached summary can drift from what the log says. Time is injected rather than read from the system clock, which means replaying the same bytes twice gives the same answer twice — and makes a bug report a log excerpt plus a timestamp.

**Read-only, and it stays that way.** The tail never opens your log for anything but reading. Nothing is written into the game's folder, nothing is injected, nothing leaves your machine.

**Failures stay small.** The parser is fuzzed on every push and in a nightly campaign. A line that panics costs its own batch, not your session — the app keeps tailing, and the next start rebuilds clean.

## Benchmarks

Measured 2026-09-07 on the two logs in this folder, release build, rustc 1.95, Intel Core Ultra 9 275HX (24 threads), 46 GB, Linux. Everything below is reproducible with the probes named; nothing is estimated.

### The logs

| file | size | lines | span | matched |
|---|---|---|---|---|
| `eqlog_Manipulator_rivervale.txt` | 245,492,047 B | 2,895,623 | Jul 28 – Aug 22 | 90.5% (274,973 unmatched) |
| `eqlog_Kaeus_rivervale.txt` | 247,137,145 B | 3,013,066 | Jul 1 – Aug 25 | 83.4% (500,480 unmatched) |

Unmatched is overwhelmingly chat. The CI fixture (`fixtures/reference-slice.log`, 6,005,246 B / 60,000 lines) parses at 94.03%; the gate refuses a pack below 80%.

### Reading the log (`examples/backfill_bench`)

Full replay from bytes to a queryable store, wall clock:

| threads | Manipulator (2.90M lines) | Kaeus (3.01M lines) |
|---|---|---|
| 1 | 14.5 s | 13.7 s |
| 4 | 11.6 s | 12.2 s |
| 8 | 11.3 s | 11.0 s |
| 16 | 11.1 s | 10.9 s |
| 32 | 11.1 s | — |

About **255,000 lines/s**, or 3.8 µs a line, at the 16-thread cap. Threads past 8 buy nothing: classification parallelizes, the fold into the store runs in file order and is the floor (~11 s). Chunk size from 10k to 100k lines is within 0.5 s. Matched/unmatched counts are identical at every setting, which is the merge being deterministic. The app itself serves off the tail of the log first and runs this behind it.

### What it holds (`examples/mem_report`, counting allocator, exact live bytes)

Manipulator's log, 429,208 events across 4,017 fights:

| | |
|---|---|
| parsed state, everything | **102.2 MB** (42% of the log's own size) |
| event columns | 19.0 MB — 46 bytes per event |
| class detector | 24.1 MB — the largest single structure |
| timeline | 2.3 MB |
| effects | 1.3 MB |
| names + abilities, interned | 1.0 MB |
| encounters | 0.2 MB |
| game-data packs, loaded once | 34 MB (items 17.6, NPCs 13.1, spells 2.7) |

Live: the desktop app after 14.6 hours tailing a 423 MB / 5.04M-line log sat at 217 MB RSS for the whole process, GTK included.

### What a read costs (`examples/query_cost`)

After the replay above, each of the UI's main reads, warm median of 5:

| read | cost |
|---|---|
| one fight — summary / ally table / timeline, newest or oldest | 0.02 / 0.06 / 0.01 ms |
| overlay DPS meter poll | 0.8 ms |
| encounter list page | 0.12 ms |
| aggregate over all 4,017 fights — summary | 44 ms |
| aggregate over all 4,017 fights — ally table | 62 ms |
| aggregate over all 4,017 fights — enemy table | 68 ms |

Nothing is cached between reads; each one is a pass over the store. The aggregate is the only read that scales with history, and it is the one you ask for on purpose.

### Surface

188 parsing rules, every one carrying the real lines it was written from. 124 IPC commands. 4 fuzz targets, smoke-fuzzed on every push and 30 min per target nightly.

## Install

Grab the newest build from [Releases](https://github.com/Thiole/EQLOracle/releases) (or the picker on [eqloracle.com](https://eqloracle.com)):

- **Windows**: `EQL.Oracle_<version>_x64-setup.exe`
- **Linux**: `.AppImage` (portable), `.deb`, or `.rpm`

First launch: point it at the EverQuest Legends install folder — the one containing `Logs`, not `Logs` itself. That's also where `/outputfile inventory` writes.

Updates are checked in-app. Two channels in Settings: `public` (deliberate releases) and `beta` (every push to the `testing` branch).

### Building from source

Rust stable and Node 20+.

```
cd ui && npm install
npm run tauri -- build
```

Bundles land in `target/release/bundle/`. For development:

```
cd ui && npm install
npm run tauri
```

## Layout

```
crates/core/     the parser. no Tauri, no I/O, no UI. fuzzed.
crates/source/   tail, clock, replay.
crates/session/  events -> encounters, class detection, timelines.
crates/store/    columnar event store. one source of truth; aggregates are queries.
crates/app/      the Tauri app — commands, ingest, everything UI-facing.
crates/cli/      eqlp lint | parse | coverage | shapes.
packs/eql.toml   parsing rules, as data.
ui/              Svelte 5 + Tailwind frontend.
```

Parser is separate from the app on purpose — fuzzable, benchmarkable, diffable without a window open.

## CI

- **verify** — fmt, clippy, tests, fuzz smoke, pack coverage regression. Gates every push.
- **beta** — Playwright against a mock IPC harness + tauri-driver.
- **release** — builds and bundles Linux + Windows on every push to main, publishes to the `latest` release.
- **nightly** — long fuzz campaign.

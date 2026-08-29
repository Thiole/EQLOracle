# EQL Oracle

Parsing & progression assistant for EverQuest Legends. Watches your `eqlog_<Character>_<Server>.txt`, replays what's already in it, keeps parsing live. Classes, AAs, spells, kills — all read off the log, nothing hand-entered. Runs entirely on your machine; nothing is uploaded.

Website: [eqloracle.com](https://eqloracle.com)

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

Parsing rules are written against this game's actual log lines, not classic EQ's — the formats diverge.

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
crates/core/     the parser. no Tauri, no I/O, no UI.
crates/session/  events -> encounters, class detection, timelines.
crates/store/    in-memory event store.
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

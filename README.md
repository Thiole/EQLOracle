# EQL Oracle

Companion app for EverQuest Legends. Watches your `eqlog_<Character>_<Server>.txt`, replays what's already in it, keeps parsing live. Classes, AAs, spells, kills — all read off the log, nothing hand-entered.

## Modules

- **Combat** — real-time DPS meter. Team/incoming damage, per-fight and aggregated, follows the current fight.
- **Character** — sheet, gear, AA log, known spells, and a spellbook builder that suggests spells (solo buff / team buff / combat rotation): respects your classes, dedupes same-line upgrades, credits DoT tick damage over the real window, folds AA crit chance/damage in as an expected-value bump. Settings page lets you reorder which spell wins when multiple classes upgrade the same line.
- **Endgame** — raid boss/miniboss kill counts, drop tables vs. what you've looted, solo/group tiers tracked separately (different instances), fastest-clear times per difficulty, live off the log. Sky's class unlocks and quests.
- **Game Data** — zones, items, NPCs, AAs, spells, cross-linked.
- **Maps** — zone maps with markers.

Parsing rules are written against this game's actual log lines, not classic EQ's — the formats diverge (XP percentages, damage/heal message shapes, etc.).

## Running it

Grab a build from Releases, or:

```
cd ui && npm install
npm run tauri -- build
```

Rust stable, Node 20+. First launch: point it at the EverQuest Legends install folder (the one containing `Logs`, not `Logs` itself — also where `/outputfile inventory` writes).

Dev:

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

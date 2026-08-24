# EQL Oracle

a companion app for EverQuest Legends. reads your log file, tells you stuff you'd otherwise have to alt-tab to a wiki or do math for yourself to know.

not the game. it's a separate Tauri app that sits next to it and watches whatever `eqlog_<Character>_<Server>.txt` you were most recently writing to. point it at your install folder once, it replays what's already in the log, then keeps parsing live as you play. classes, AAs, spells, kills, all of it comes from what actually happened in your log, not from you telling it anything.

## what it actually does

- **Combat** — real-time DPS meter. team damage, incoming damage, per-fight and aggregated across a whole zone visit, follows whatever fight is current automatically.
- **Character** — character sheet, gear, AA log, known spells, and a spellbook builder that actually suggests spells for you (solo buff / team buff / combat rotation), not just a flat list. it knows your classes, won't double up on the same spell line, and does real time-resolved DPS math — DoTs get credited for their tick damage over the actual window, not just their cast, and it accounts for your AA crit chance/damage as an expected-value bump on top (not a fake per-hit RNG sim, just the average going up by the math). there's a settings page to reorder which spell in a line wins when more than one of your classes has an upgrade to the same thing.
- **Endgame** — raid boss/miniboss tracking. real kill counts, drop tables cross-referenced against what you've actually looted, solo vs group tiers tracked separately (they're genuinely different instances in this game), fastest-clear times per difficulty. updates live off the log, no manual refresh. also Sky's primary class unlocks and quest tracking.
- **Game Data** — a real browsable catalog: zones, items, NPCs, AAs, spells. cross-linked, so you can click from a spell to the class that gets it, from an NPC to its drop table, whatever.
- **Maps** — zone maps with markers, for when the wiki's version is wrong or missing.

everything's built off the log itself, not assumptions about how classic EQ worked — this game's log format differs from classic EverQuest in a bunch of specific ways (percentages on XP gain, different damage/heal message shapes, etc.), and every parsing rule here was written against real lines, not memory of a 20-year-old game.

## running it

grab a build from Releases, or build it yourself:

```
cd ui && npm install
npm run tauri -- build
```

needs Rust (stable) and Node 20+. first launch asks for your EverQuest Legends install folder — the one that directly contains `Logs`, not the `Logs` folder itself (that's also where `/outputfile inventory` writes its dump, needs to be reachable from the same place).

for dev:

```
cd ui && npm install
npm run tauri
```

## how it's put together

```
crates/core/     the log parser itself. no Tauri, no file I/O, no UI — pure bytes in, parsed events out.
crates/session/  turns parsed events into encounters, class detection, timelines.
crates/store/    the in-memory event store the app queries against.
crates/app/      the actual Tauri app — commands, ingest pipeline, everything UI-facing.
crates/cli/      eqlp lint | parse | coverage | shapes — standalone tools for working on the parser/rule pack.
packs/eql.toml   every parsing rule, as data, not code.
ui/              the frontend. Svelte 5 + Tailwind.
```

the parser is deliberately its own thing, separate from the app and the UI. that split is what makes it possible to fuzz it, benchmark it, and diff its output without ever opening a window — and it means a parsing bug and a rendering bug never get confused for the same bug.

## CI

- **verify** — fmt, clippy, the full test suite, a fuzz smoke pass, and a coverage-regression check on the rule pack. gates every push.
- **beta** — Playwright against a mock IPC harness plus the real webview via tauri-driver, catches the stuff unit tests can't (window count, hit testing, layout).
- **release** — builds and bundles Linux + Windows on every push to main.
- **nightly** — the long fuzz campaign, too slow to gate a normal push.

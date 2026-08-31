# Architecture — parser, ingest, and the event store

The mechanical reference: what actually happens to a log line, in order,
with the real type names. The `docs/design/*.md` files carry the
*rationale* for these choices; this file is the *map*. Current as of
0.8.x.

```
                         crates/source                     crates/core
  eqlog_*.txt  ──►  Tail::poll (bytes)  ──►  Framer  ──►  Engine/Matcher ──► Outcome
   (game writes,     newest_log_in picks      (carries      (Aho-Corasick        │
    read-only)        the file, 250ms poll)    partial       anchors → regex)    │
                                               lines)                            ▼
                        crates/app                                        extract_action
  ┌──────────────────────────────────────────────────────────────┐              │
  │ Ingest::route / backfill_lines                               │◄─────  Action (160
  │   apply(ts, Action) fans out to:                             │        variants)
  │                                                              │
  │   eqlp-session                     eqlp-store                │
  │   ├ graph::Builder   (encounters)  ├ Store (columnar rows)   │
  │   ├ graph::Entities  (who is who)  ├ Interner (names→Sym)    │
  │   ├ Timeline         (state mach.) ├ Abilities (tags,ceiling)│
  │   ├ Spans            (zone visits) └ Vec<Encounter> (ranges) │
  │   ├ GroupTracker     (roster)                                │
  │   └ ClassDetector    (per-visit class evidence)              │
  └──────────────────────────────────────────────────────────────┘
                                │
                    #[tauri::command] queries (combat.rs, monsters.rs, …)
                    build DTOs on demand — nothing is precomputed
```

## 1. Source layer (`crates/source`)

`newest_log_in(dir)` scans the configured `Logs/` directory every 5s
(`RESCAN_MS`) for the most-recently-modified `eqlog_*.txt` — that is the
only reliable signal for which character is logged in. A change of
target resets the whole `Ingest` and replays the new file from byte 0.

`Tail` polls the file every 250ms (`POLL_MS`). Each `poll()` compares
`fs::metadata` against its saved offset and `FileId` (inode on unix,
creation time on Windows) and returns one of `Grew / Truncated /
Replaced / Missing / Idle`. It reads in 256KB chunks and hands raw bytes
to the sink. Hard invariant, documented in the module: the log is only
ever opened for **reading**.

There is no filesystem-notification path on purpose — polling is the
thing that behaves identically across OSes, network shares, and Wine.

## 2. Framing and the header (`crates/core`)

A poll can land mid-line (the game flushes whenever), so `Framer::push`
carries the partial tail across chunks and emits only complete lines.
An unbounded line (binary file, corruption) is truncated and resynced at
the next newline rather than growing the carry buffer forever.

The `[Tue Jul 28 15:02:15 2026] ` header parses with fixed offsets — no
chrono, no allocation. The result is **`LocalTs`: wall-clock time as
written, no timezone attached**. Every backend comparison is between two
log timestamps, so this is always correct internally. The one rule that
follows: *never compare a backend `Millis` against `Date.now()`*. The
frontend reconstructs the same naive clock via `logClockNowMs()`
(`ui/src/lib/overlay/logClock.ts`) — the Skill Tracker timers and the
Drop Watch checkpoint both broke on exactly this before.

## 3. Classification (`crates/core::Engine`)

The rule pack (`packs/eql.toml`, ~600 rules, compiled in via
`include_str!` in `parser.rs`) defines `id + kind + anchors + pattern +
examples`. Classification is two-stage:

1. One **Aho-Corasick** pass over the line finds which literal anchors
   are present. A rule is a candidate iff *all* its anchors were found
   (anchors are pure optimisation — semantically invisible, enforced by
   the `anchors_never_change_the_answer` test). `excludes` literals ride
   the same pass as deliberate vetoes.
2. Only the (typically 0–1) candidate rules run their regex.

Capture extraction is masked per-rule-kind at runtime — pulling captures
costs ~2.2x the rest of the pipeline, so only consumers that need fields
pay for them.

The result is a total `Outcome`: `Matched(rule, captures) | Unmatched |
Headerless | Blank`. Unmatched lines feed shape clustering (the Debug
module's "Unparsed" tab); nothing is silently dropped.

## 4. Action extraction (`crates/app/src/ingest.rs`)

`extract_action(rule_id, fields) -> Option<Action>` is the one bridge
from pack-land (string rule ids) to typed program-land. `Action` is a
~160-variant enum: `Hit`, `CastBegin`, `Death`, `Loot`, `ZoneEnter`,
`Mez`, `Charm`, `GroupJoined`, `PetSummon`, `OutputfileComplete`,
`TurnInItem`, `Xp`, `CraftAttempt`, … A rule whose kind carries no
state (noise, flavor) extracts to `None` and only counts.

## 5. Ingest — the sequential heart

`Ingest` owns every piece of session state. Two entry paths, one merge
point:

- **Live** (`route`): called per line by the tail worker. Classify,
  extract, `apply`. Sets `recent` / `pending_notifications` only when
  `live` — a decade of backfill must not fire toasts.
- **Backfill** (`backfill_lines`): the parallel path. Lines are split
  into chunks; `classify_chunk` runs on worker threads (classification
  is stateless — no `Ingest` access), producing a `ChunkResult` of
  `(Millis, Option<Classified>)` per matched line plus local shape
  stats. Results are then **replayed sequentially in order** through the
  same `apply` — parallel where it is safe, ordered where it matters.
  `Classified` wraps `Action` plus the flavor/spell-effect dictionary
  hits that need the sequential context (Quick Buff attribution,
  cast-landing confirmation).

  The tail worker feeds this in 100k-line batches
  (`BACKFILL_CHUNK_LINES`), releasing the ingest mutex and emitting a
  progress tick between batches — that is why the UI is usable during a
  multi-million-line replay. Each batch runs under `catch_unwind`
  (`backfill_guarded`), and every lock site uses `lock_recover()`, so a
  panicking line drops its batch instead of poisoning the app.

`apply(ts, action)` is a match that fans out to the subsystems below.
Ordering guarantees live here and nowhere else.

### 5a. Identity (`session::graph::Entities` + interning glue)

Every name passes through `resolve_name` (canonical "You", display-case
folding) and lands in `Entities`, which classifies **monotonically** —
evidence promotes, never demotes:

- `Kind::Player` — proved by a player-only chat channel, group
  membership, or the shared-target promotion (hit the same anchor mob
  "You" confirmed).
- `Kind::Pet` — possessive name (`X's pet`, `` X`s pet ``, `X`s
  warder`).
- `Kind::Unproven` — everything else. **Unproven defaults to enemy.**

Two inference maps refine pets past the name rule (`ingest.pet_owner`,
`ingest.behavioral_pets`): the summon-window match (a new actor's first
action within 8s of a `PetSummon`, cast-triggered or damage-triggered
behind the generated-pet-name shape gate) merges a pet's rows into its
owner's `Sym`; the behavioral classifier graduates a shape-matching
stranger to ally after three clean enemy-directed hits and blacklists it
on any hostile contact with the player's side. `allegiance_at(name, ts)`
is the **single composition point**: permanent `Kind` + charm state +
group membership + pet maps → `Ally | Enemy`, as of a timestamp. Every
side decision in the app goes through it.

### 5b. Encounters (`session::graph::Builder`)

Connected-component detection over damage edges. A damage line links
actor and target into one live encounter; transitive merging pulls
overlapping fights together (capped by `Policy.max_entities`).

Closing is a **two-tier idle** (`Policy`): a fight where something has
died closes 10s after last activity (`idle_ms` — fast close keeps
back-to-back pulls separate); a fight with zero kills waits 60s
(`idle_unresolved_ms` — quiet there means mezz/fled/medding). A `Mez`
line on a fight's entity refreshes its clock (`touch_entity`). Closes
stamp `end_ms` at *last activity*, so patience never inflates duration.
A re-engaged surviving mob within 60s (`link_ms`) records `links_to` on
the new encounter.

Anchoring: an encounter's `target` is its best enemy candidate, and
`retarget_encounter` fixes it when a later edge proves a better one
(the "unspoken tank anchored the boss fight" bug class).

### 5c. Entity state (`session::Timeline`)

A per-entity state machine over `Engaged / Mezzed / Charmed / Dead /
Lost`, with each transition marked `Observed` (a real log line) or
`Inferred` (e.g. `Lost` stamped at fight close for an unaccounted
survivor; charm-break inferred from a charmed pet hitting "You").
Queried as `state_at(sym, ts)` — time-travel is free because it stores
transitions, not current state.

### 5d. Context (`session::Spans`, `GroupTracker`, `ClassDetector`)

- `Spans` (zone): every `ZoneEnter` is a new visit — index, label,
  binary-searchable `at(ts)` / `index_at(ts)`. Difficulty tier is
  derived from the label itself (`zone::zone_tier`).
- `GroupTracker`: explicit join/leave/removed lines plus group-chat
  proof; roster resets on a 2h log gap. Feeds `allegiance_at` and
  `involves_you`.
- `ClassDetector`: per-zone-visit evidence buckets; an unambiguous cast
  confirms a class, subset-elimination narrows, nothing ever evicted.
  `configurations_of` returns every distinct 3-class configuration with
  visit counts.

## 6. The store — the "database" (`crates/store`)

There is no database file. The store is an **append-only, in-memory,
columnar event log**, rebuilt from the log on every launch (that is the
whole persistence model: the game's log file *is* the database of
record; `parse_history.jsonl` is wiped at startup by design).

### Columns (struct-of-arrays, one entry per event)

```rust
pub struct Store {
    pub ts:      Vec<Millis>,     // log-clock timestamp
    pub kind:    Vec<EventKind>,  // Damage | Heal | Miss | Cast | Death
                                  // | Loot | Xp | Currency | Craft
    pub actor:   Vec<Sym>,        // interned name (u32)
    pub target:  Vec<Sym>,
    pub ability: Vec<AbilityId>,  // interned ability; Loot/Craft reuse it
                                  // for the item name
    pub amount:  Vec<u64>,        // damage, heal, qty, milli-% for Xp,
                                  // copper for Currency
    pub flags:   Vec<Flags>,      // u32 bitfield, see below
    pub enc:     Vec<u32>,        // owning encounter id, or NO_ENCOUNTER
    pub tier:    Vec<u8>,         // zone difficulty at the time, app-stamped

    pub names:      Interner,     // name <-> Sym, case-folded
    pub abilities:  Abilities,    // per-ability Tags + damage ceiling
    pub encounters: Vec<Encounter>,
    evicted: u32,                 // ids below this no longer resolve
}
```

`Flags` packs per-event facts: crit/riposte/rampage/flurry…, cast
outcomes (`CAST_LANDED/RESISTED/INTERRUPTED/FIZZLED/UNCONFIRMED`),
avoidance (`MISSED/BLOCKED/DODGED/PARRIED`), `LOOT_AUTO_SOLD`,
`CRAFT_SUCCESS/…`. One u32 per row instead of nine bool columns.

`Abilities` carries the semantic layer per ability: `Tags` (a bitfield —
`MELEE SPELL DOT PROC DAMAGE_SHIELD HEAL PARTIAL_RESIST PET`) that
*accumulate as evidence arrives* (a landing with no cast anywhere is
`PROC` until a cast line proves otherwise), and `ceiling`, the highest
amount seen — which is how partial resists are detected with zero
hardcoded damage values.

### Encounters are ranges, not copies

```rust
pub struct Encounter {
    pub id: EncounterId,          // stable u32, never renumbered
    pub target: Sym,              // the anchor mob (retargetable)
    pub start_ms / end_ms,        // end stamps at last activity
    pub first / last: u32,        // half-open row range into the columns
    pub slain: bool,              // a confirmed *enemy* death
    pub wiped: bool,              // "You" died and no enemy kill confirmed
    pub zone: Option<Sym>,
    pub involves_you: bool,       // you/your pet/a groupmate acted in it
}
```

The graph layer's closed encounters drain into these via
`Ingest::drain_closed` (`enc_map` maps graph `EncId` → store
`EncounterId`); that is also where kill/wipe/reset is decided and
`ParseRecord`s append to parse history. `!slain && !wiped` on a closed
encounter is what the UI labels **reset**.

Eviction is by whole encounter (`evict_before_encounter`) — a
half-evicted fight would report wrong totals silently.

### Everything is a query

No running totals exist anywhere. `query.rs` provides `Filter`
(`encounter × kind × actor × target × tier × time-window × tags`) and
linear-scan aggregators: `by_ability` (the breakdown table — two passes,
39µs per encounter at 750k events), `by_actor`, `total`, `dps_window`
(the live meter — a filtered sum, not a ring buffer),
`by_target_and_ability`, `roll_up_by_tag`. Every DTO the frontend sees
(`combat::summarize`, `list_allies`, `fight_timeline`,
`monsters::list_mobs`, …) is computed from these on demand, per call,
under the one ingest mutex.

## 7. Out the other side

The tail worker emits a `parse-tick` event (throttled to 3s heartbeat,
immediate on news) carrying `TailStatus` + `LineCounts` + recent lines;
the UI's stores fan it out (`ui/src/lib/tauri/events.ts`). Everything
else is pull: `#[tauri::command]`s in `commands.rs` lock the ingest,
run a query, return a DTO. The mock harness (`ui/src/lib/tauri/mock.ts`)
serves the same DTOs from `dump_fixtures`-generated JSON so the entire
UI runs and tests in a plain browser.

## Invariants worth never breaking

1. The game log is opened read-only, always (`source/tail.rs`).
2. Backend `Millis` is log-clock, timezone-less. Frontend comparisons
   use `logClockNowMs()`, never `Date.now()`.
3. Entity classification is monotonic; side is always computed through
   `allegiance_at`, never from raw `Kind` at a call site.
4. The store is the only holder of combat data; aggregates are queries.
5. Nothing persisted survives a restart except explicit config/
   preferences — parsing state is always rebuilt from the log itself.
6. Every command lock uses `lock_recover()`; every parse batch is
   panic-isolated.

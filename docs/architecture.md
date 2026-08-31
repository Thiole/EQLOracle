# Architecture — parsing, ingest, event store

Reference for the line-to-query pipeline. Rationale lives in
`docs/design/*.md`.

```
eqlog_*.txt → Tail::poll → Framer → Engine::classify → Outcome
                                                          │
                                              extract_action → Action
                                                          │
                                                    Ingest::apply
                          ┌───────────────────────────────┼──────────────────┐
                    eqlp-session                     eqlp-store              │
                    graph::Builder  (encounters)     Store    (columns)      │
                    graph::Entities (identity)       Interner (names)        │
                    Timeline        (entity state)   Abilities(tags,ceiling) │
                    Spans           (zone visits)    Encounter(row ranges)   │
                    GroupTracker    (roster)                                 │
                    ClassDetector   (class evidence)                         │
                          └───────────────────────────────┴──────────────────┘
                                                          │
                                       #[tauri::command] → query → DTO
```

## Source (`crates/source`)

- `newest_log_in(dir)`: most-recently-modified `eqlog_*.txt` in the
  configured `Logs/` directory. Rescanned every 5s (`RESCAN_MS`). Target
  change ⇒ `Ingest` reset, full replay of the new file.
- `Tail::poll` every 100ms (`POLL_MS`). Compares `fs::metadata` length
  against a saved offset and `FileId` (inode on unix, creation time on
  Windows). Returns `Grew(n) | Truncated | Replaced | Missing | Idle`.
  Reads 256KB chunks into a sink.
- Log files are opened read-only. No write path to the log exists in the
  codebase.
- Polling, not filesystem notification.

## Framing, header (`crates/core`)

- `Framer::push(chunk, sink)`: emits complete lines; carries the partial
  tail across chunks. Over-long lines truncate and resync at the next
  newline. `flush()` is batch-EOF only.
- Header `[Tue Jul 28 15:02:15 2026] ` parses at fixed offsets. No
  chrono, no allocation.
- Timestamps are `LocalTs`: wall-clock as written, no timezone. All
  backend comparisons are log-time vs log-time. Frontend comparisons
  against the machine clock must use `logClockNowMs()`
  (`ui/src/lib/overlay/logClock.ts`), never `Date.now()`.

## Classification (`crates/core::Engine`)

- Rule pack: `packs/eql.toml`, embedded via `include_str!`
  (`app/parser.rs`). Rule = `id + kind + anchors + pattern + excludes +
  examples`.
- Stage 1: one Aho-Corasick pass collects anchor hits. Candidate rule ⇔
  all its anchors present. Anchors are semantically invisible (enforced
  by `anchors_never_change_the_answer`). `excludes` are literal vetoes
  in the same pass.
- Stage 2: candidate rules (typically 0–1) run their regex.
- Capture extraction is masked per rule kind; unmasked kinds skip it.
- `Outcome` is total: `Matched | Unmatched | Headerless | Blank`.
  Unmatched lines feed shape clustering (Debug → Unparsed).

## Action extraction (`crates/app/src/ingest.rs`)

`extract_action(rule_id, fields) -> Option<Action>`. `Action` ≈ 160
variants (`Hit`, `CastBegin`, `Death`, `Loot`, `ZoneEnter`, `Mez`,
`Charm`, `GroupJoined`, `PetSummon`, `OutputfileComplete`, `Xp`,
`CraftAttempt`, …). Rules with no state effect extract `None` and only
count.

## Ingest

Entry paths:

- Live: `route(engine, line, outcome)` per tailed line. `recent` and
  `pending_notifications` populate only when `live` is set
  (`mark_live()` after backfill completes).
- Backfill: `backfill_lines(ing, engine, lines, threads)`. Chunks
  classify in parallel (`classify_chunk`, stateless), producing
  `ChunkResult { counts, matched: Vec<(Millis, Option<Classified>)>,
  unmatched_shapes }`. Results replay sequentially through `apply` in
  timestamp order. `Classified` = `Action` plus flavor/spell-effect
  dictionary hits that require sequential context (Quick Buff
  attribution, cast-landing confirmation).
- Tail worker batch size: 100k lines (`BACKFILL_CHUNK_LINES`); the
  ingest mutex releases and a progress tick emits between batches.
- Panic isolation: batches run under `catch_unwind`
  (`backfill_guarded`); all lock sites use `lock_recover()`
  (poison-recovering). `panic = "unwind"` in the workspace profile.

`apply(ts, Action)` fans out to the subsystems below. All ordering
guarantees live in `apply`.

### Identity (`session::graph::Entities`, ingest pet maps)

- `Kind` is monotonic: evidence promotes, never demotes.
  - `Player`: player-only chat channel, group membership, or
    shared-target promotion (damaged an anchor "You" confirmed).
  - `Pet`: possessive name — `X's pet`, `` X`s pet ``, `` X`s warder ``.
  - `Unproven`: default; treated as enemy.
- `ingest.pet_owner: HashMap<pet, owner>`: summon-window inference. A
  new actor's first action within 8s (`PET_MATCH_WINDOW_MS`) of a
  `PetSummon` matches the closest pending summon. Cast path: first cast
  is `Inner Fire`. Damage path: gated on `looks_generated_pet_name`
  (first letter G/J/K/L/V/X/Z, ending -n/-er/-tik/-ab, 4–10 alpha
  chars). Bare `* pet` / `* warder` names are excluded from both paths.
  Matched pets' rows intern under the owner's `Sym`.
- `ingest.behavioral_pets`: shape-matching `Unproven` entity with ≥3
  enemy-directed hits and zero hostile contact with the ally side ⇒
  ally. Any ally-directed hit, or any hit received from an ally ⇒
  permanent blacklist, graduation revoked.
- `allegiance_at(name, ts) -> Ally | Enemy`: the single side-decision
  point. Composes `Kind` + charm state (`Timeline`) + `GroupTracker` +
  both pet maps. All side logic routes through it.

### Encounters (`session::graph::Builder`)

- Connected components over damage edges; transitive merge, capped by
  `Policy.max_entities`.
- Idle close, two-tier (`Policy`): `slain` non-empty ⇒ `idle_ms` (10s);
  `slain` empty ⇒ `idle_unresolved_ms` (60s). `Mez` on a member entity
  refreshes `last_ms` (`touch_entity`). Close stamps `end_ms` at last
  activity.
- `link_ms` (60s): a re-engaged surviving non-player records `links_to`
  on the successor encounter.
- `target` is the anchor; `retarget_encounter` replaces it when a later
  edge proves a better enemy candidate.

### Entity state (`session::Timeline`)

State machine per entity: `Engaged | Mezzed | Charmed | Dead | Lost`.
Transitions store cause `Observed | Inferred`. Query:
`state_at(sym, ts)`. `Lost` is inferred at fight close for un-slain,
non-player, non-charmed members.

### Context (`session::Spans`, `GroupTracker`, `ClassDetector`)

- `Spans`: every `ZoneEnter` is a new visit; `at(ts)` / `index_at(ts)`
  by binary search. Difficulty tier derives from the zone label
  (`zone::zone_tier`).
- `GroupTracker`: explicit join/leave/removed lines + group-chat proof;
  roster clears on a 2h log gap.
- `ClassDetector`: evidence per zone visit; unambiguous cast confirms,
  subset elimination narrows, nothing evicted.
  `configurations_of(entity)` → distinct 3-class configurations with
  visit counts.

## Event store (`crates/store`)

In-memory, append-only, columnar. Rebuilt from the log at every launch;
the log file is the durable record. No running totals exist; every
aggregate is a query.

```rust
pub struct Store {
    pub ts:      Vec<Millis>,     // log-clock
    pub kind:    Vec<EventKind>,  // Damage Heal Miss Cast Death Loot Xp Currency Craft
    pub actor:   Vec<Sym>,
    pub target:  Vec<Sym>,
    pub ability: Vec<AbilityId>,  // Loot/Craft: item name
    pub amount:  Vec<u64>,        // damage/heal/qty; Xp: milli-%; Currency: copper
    pub flags:   Vec<Flags>,      // u32 bitfield
    pub enc:     Vec<u32>,        // encounter id or NO_ENCOUNTER
    pub tier:    Vec<u8>,         // zone difficulty at event time

    pub names:      Interner,     // name ↔ Sym (u32), case-folded
    pub abilities:  Abilities,    // per-ability Tags + ceiling
    pub encounters: Vec<Encounter>,
    evicted: u32,
}
```

- `Flags`: crit/riposte/rampage/flurry/…, cast outcomes
  (`CAST_LANDED|RESISTED|INTERRUPTED|FIZZLED|UNCONFIRMED`), avoidance
  (`MISSED|BLOCKED|DODGED|PARRIED`, union `MITIGATED`),
  `LOOT_AUTO_SOLD`, `CRAFT_SUCCESS`, `CRAFT_SKILL_CAPPED`.
- `Abilities`: `Tags` bitfield per ability
  (`MELEE SPELL DOT PROC DAMAGE_SHIELD HEAL PARTIAL_RESIST PET`),
  accumulated as evidence arrives — a landing with no cast anywhere tags
  `PROC`; a later cast clears it. `ceiling` = max amount observed per
  ability; basis for partial-resist detection.

```rust
pub struct Encounter {
    pub id: EncounterId,     // stable u32; ids below `evicted` don't resolve
    pub target: Sym,
    pub start_ms: Millis,
    pub end_ms: Option<Millis>,   // last activity, not close time
    pub first: u32, pub last: u32, // inclusive row range
    pub slain: bool,         // confirmed enemy death
    pub wiped: bool,         // "You" died, no enemy kill confirmed
    pub zone: Option<Sym>,
    pub involves_you: bool,  // you / your pet / a groupmate acted
}
```

- Graph closes drain into the store via `Ingest::drain_closed`
  (`enc_map: EncId → EncounterId`). Outcome mapping: `slain` requires an
  enemy name in the close's slain list; `wiped` = "You" among slain and
  not `slain`; neither ⇒ UI label "reset". `ParseRecord` history appends
  here.
- Eviction: `evict_before_encounter` — whole encounters only; surviving
  ranges rebase.

### Query layer (`store/query.rs`)

`Filter`: `encounter × kind × actor(by) × target × tier × window ×
tags`. Aggregators (linear scans over the filtered range):

| fn | output |
|---|---|
| `by_ability` | per-ability rows: attempts, total, crits, mean, avg_normal/avg_crit, dps |
| `by_actor` | `(Sym, total, hits, crits)` |
| `total` | filtered sum |
| `dps_window` | filtered sum over trailing window / window |
| `by_target_and_ability` | one pass, whole store, keyed by target |
| `roll_up_by_tag` | mechanism view derived from ability rows |

Measured at 750k damage events: `by_ability` on one encounter 39µs;
whole-store `by_ability` 27ms.

## Egress

- `parse-tick` event: 3s heartbeat, immediate on news. Payload:
  `TailStatus` + `LineCounts` (`by_kind` per rule kind) + recent lines.
- All other data is pull: `#[tauri::command]` → lock ingest
  (`lock_recover`) → query → DTO. No DTO is cached backend-side.
- Mock harness (`ui/src/lib/tauri/mock.ts`) serves the same DTOs from
  `dump_fixtures` output keyed by `(command, args)`.

## Invariants

1. Log files: read-only.
2. Backend `Millis` = log clock, no timezone. Frontend wall-clock
   comparisons use `logClockNowMs()`.
3. `Kind` promotion is monotonic. Side decisions go through
   `allegiance_at` only.
4. Combat data lives in `Store` only; aggregates are queries.
5. Parse state rebuilds from the log at launch; only config/preferences
   persist.
6. Every ingest lock is `lock_recover()`; every parse batch is
   panic-isolated.

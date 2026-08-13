# Store — design notes

Rationale for `eqlp-store`.

## One source of truth

The store is an append-only columnar event log. **Nothing else holds combat
data.** Encounters are ranges into it. Every total, DPS figure, breakdown and
rollup is a query, computed on demand.

The alternative — keeping running totals alongside the events — means two things
that can disagree, and they always eventually do: a late event, an eviction, a
retroactive re-attribution, and the aggregate is wrong in a way nothing detects.

This applies to the live number too. `dps_window` is a filtered sum over a time
range, not a ring buffer. No second copy, nothing to keep in sync.

## Is scanning actually cheap enough?

Measured, at 750k damage events (a generous 12-day session, ~25 MiB):

| query | time |
|---|---|
| `by_ability`, one encounter | **39 µs** |
| 50 encounter breakdowns | 1.4 ms |
| `by_actor`, whole store | 12 ms |
| `by_ability`, whole store | 27 ms |

The per-encounter query is what a live panel runs, and it is three orders of
magnitude inside a frame budget. Whole-store queries are session summaries, run
on demand, and 27 ms is fine for those.

An earlier version of `by_ability` did the full-power pass once per ability row,
making it quadratic in ability count — 86 ms for the same work. It is now two
linear passes: the ceiling is only known after aggregating, so a second pass is
unavoidable, but it is one pass, not one per row.

## Why columnar

Struct-of-arrays, so a group-by touches only the columns it needs. `by_actor`
reads `actor` and `amount` and never pages in `flags` or `ability`.

Names are interned to `u32`. Actors, targets and abilities repeat constantly in
a combat log, so grouping becomes integer work and memory stays flat.

## Rows are abilities, not mechanisms

The primary breakdown key is the **ability**: `Backstab`, `Ice Comet`,
`Puma Maw`, `Garrison's Mighty Mana Shock`. Not "melee damage" versus "spell
damage".

This is what makes the interesting comparison possible. `Backstab` is melee and
a burn proc is a proc, but the question a player has is "which of my sources is
actually producing damage", and that question is answered by putting them in the
same table with the same columns.

Mechanism travels alongside as **tags**, a bitfield facet:

```
MELEE  SPELL  DOT  PROC  DAMAGE_SHIELD  HEAL  PARTIAL_RESIST  PET
```

Tags are for grouping, filtering and colour — never for identity. An ability can
carry several: a burn proc is `PROC | DOT`, and appears under both in a rollup,
by design.

`roll_up_by_tag` derives the mechanism view from the ability rows rather than
scanning again, so the two views cannot disagree.

## Tags accumulate as evidence arrives

An ability first seen as a landing with no cast line is tagged `PROC`. If a cast
line for it later appears, `note_cast` clears the tag. This matters because
proc-versus-cast is only knowable from the whole session — `Puma Maw` has 3,051
landings and zero casts anywhere in 1.8M lines, which is what proves it is a
weapon proc rather than a spell we happened to miss.

`ceiling` tracks the highest amount observed per ability, which is how
`full_power` distinguishes a clean hit from a partial resist without hardcoding
any damage values.

## Retention

A live tail runs for days, so the store evicts. Eviction is **by encounter**, not
by event count: a half-evicted fight would silently report wrong totals for the
part that survived. `evict_before_encounter` drops whole encounters and rebases
the surviving ranges.

## Consequence for `eqlp-session`

`Tracker` currently owns damage totals and per-source `Rolling` buffers. That is
now a second copy of what the store already holds, and it must be reduced to an
**encounter builder**: it decides where fights start and end and emits ranges
into the store, and holds no damage of its own. Until that is done there are two
places of truth, which is exactly what this design exists to prevent.

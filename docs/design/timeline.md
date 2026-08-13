# Timeline — design notes

Rationale for `eqlp-session::timeline`.

## Requirement: scrub, not just "now"

The target interaction is a DPS graph you can drag across: at any instant, show
both sides of the fight, who each entity was on, and what state they were in.

That is a stronger requirement than a live meter, and it decides the data
structure. State cannot be a mutable `HashMap<Entity, State>` updated as lines
arrive, because "what was true at 14:32:07" is then unanswerable without
replaying from the start.

So state is an **append-only log of timestamped transitions**, and current state
is just `state_at(now)`. Same principle as the event store: one source of truth,
everything else a query.

Consequences that fall out for free:

- Scrubbing is reversible. Dragging left then right cannot produce different
  history, because nothing mutates.
- Replay and live use identical code.
- A late-arriving line inserts in position rather than corrupting order.

## Mez does not end combat

`State::in_combat()` is true for `Engaged`, `Mezzed` and `Charmed`.

A mesmerized mob is still in the fight and still on the aggro list — mez delays
actions, it does not remove an entity from the field. Treating it as an exit
would close encounters that are still live, and would make a long mez look like
the fight ended.

`Dead` and `Lost` are the only exits.

## Observed vs inferred

Every transition carries a `Cause`.

Some transitions are stated by the log: `X has been mesmerized`,
`X has been charmed`, `X has been slain by Y`. Others cannot be, because the
game never writes them — **memory blur, pacify and lull produce no result line
at all** (4 Memory Blur casts in the reference log, zero outcomes). Fleeing and
moving out of range are likewise silent.

Everything silent collapses into `Lost`, marked `Inferred`. The name is
deliberate: it says the entity left for a reason we could not observe, rather
than implying a mechanic we did not detect.

Entities with no transition at all report `Engaged` / `Inferred` — seen
fighting, nothing has changed, but that is an assumption and it is labelled.

## Series

`series()` buckets damage into fixed-width samples for plotting. Empty buckets
are emitted rather than skipped: a gap in a fight is information, and a series
with holes cannot be drawn against a linear time axis.

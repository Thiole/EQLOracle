# Context — design notes

Rationale for `eqlp-session::context`.

## Zone and session need no columns

Both are answered by the same query as `state_at`: what was true at this
instant. So neither is stored on the encounter.

Putting a `zone` field on each encounter would be a second copy of something the
timeline already knows, and it would drift the moment a zone line arrives late,
a pack is re-derived, or a replay runs. The `Spans` lookup costs a binary search
and cannot disagree with the log.

The same type serves any dimension added later — raid target, group composition,
invocation state, time of day — with no schema change and no migration. That is
the whole return on organising the parse this way.

## Two groupings, both needed

- **By visit** (`group_by_zone_visit`) — keyed on span index. You enter Nektulos
  Forest 35 times in the reference log; those are 35 trips, and a per-trip view
  must not merge them.
- **By name** (`group_by_zone_name`) — keyed on the label. "How do I do in
  Sebilis overall" wants every visit together.

Neither is more correct. They answer different questions, and both are one pass
over the same data.

## Repeat entries collapse

Zone lines repeat on load screens. `Spans::enter` ignores a label identical to
the current one, because treating each as a new span would fragment every
grouping built on top.

Re-entering a zone you have since left *is* a new span. Same label, different
visit.

## Sessions are inferred, so the threshold is a parameter

The log states no session boundary; it is derived from silence. On the reference
log the count depends entirely on where the line is drawn: 25 sessions at a
5-minute gap, 21 at 10 minutes, 16 at 60.

There is no correct answer, so `Sessions::new(gap_ms)` takes one. Default is 10
minutes.

## Unknown is a bucket, not an error

Attaching to a log mid-session means no zone line has been seen yet. Encounters
before the first mark group under `unknown` rather than being dropped or
assigned to a zone we are guessing at.

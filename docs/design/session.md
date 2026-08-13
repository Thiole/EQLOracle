# Encounters and live numbers — design notes

Rationale for `eqlp-session`.

Separate from the parser on purpose. The parser answers "what does this line
say"; this crate answers "what is happening right now", which needs memory, a
clock, and judgement calls about boundaries. Keeping them apart means the parser
stays a pure function you can fuzz, and the judgement calls stay swappable.

Everything is incremental: feed events as they arrive, read the current value.
Nothing rescans, nothing grows without bound.

## Rolling window

### The thing that makes live DPS look broken

Dividing by elapsed time at the start of a fight. Two seconds in, one 600 hit
reads as 300 DPS; a second later the same fight reads 200 and looks like it is
collapsing. Every jittery meter does this. So: divide by window width once the
fight is older than the window, by elapsed time before that, with a floor. The
number ramps in instead of spiking and decaying.

### The fencepost

Eviction compares `<=`, not `<`. With `<`, a 5-second window holding a steady
100/sec keeps six events, not five, and reports 120 DPS — a 20% overstatement
that grows as the window shrinks. A half-open window `(now - width, now]` is the
only one that reports the rate you actually did.

`evict` must also be called from the UI tick, not only on push. Otherwise a fight
that goes quiet reports its last DPS forever — the other classic meter bug.

## Encounter boundaries

The log has no combat-state lines. There is no "you have entered combat". So:

- **Start** — first damage event naming a target.
- **End** — a death line naming that target, or a timeout with no damage.

Both reasons are recorded. `Slain` gives an exact duration; `Timeout` gives an
upper bound, and a DPS number derived from it is less trustworthy. A timeout
encounter ends at `last_ms`, not at `now` — including the silence would dilute
DPS with a minute of nothing.

## Case folding

Encounter keys fold the first character only. The game capitalises the mob name
at sentence start, so the same mob is `an armadillo` in
`You hit an armadillo for 4...` and `An armadillo` in
`An armadillo has been slain by Haken!`. Keying on the raw string meant death
lines silently failed to close encounters — in the reference fixture, 511 deaths
closed only 114 fights. Folding fixed it to 450.

Only the first character. Lowercasing the whole name would merge genuinely
distinct targets, since proper nouns carry meaning (`a gnoll` and
`Gnoll Commander` are different mobs).

## Known limitation

Encounters key on target **name**; the log gives no instance id. Two mobs called
`an abhorrent` are one encounter. Same ceiling that makes charm-pet attribution
impossible, and not solvable from this data. Fights against uniquely-named
targets are exact; fights against several same-named mobs are aggregates.

## Time to kill

The log never states a mob's health. But a fight ending in `Slain` says exactly
how much damage that name absorbed, so after a few kills the median is usable.

Fights ending in `Timeout` are excluded — the mob did not die, so its total says
nothing about its health. Below three samples, `Ttk::NoBaseline` rather than a
guess.

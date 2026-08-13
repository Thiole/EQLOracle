# Parsing — design notes

Rationale for `eqlp-core`. Kept out of the source so the code reads as an API.

## Pipeline

```
bytes -> Framer -> HeaderParser -> Matcher -> Outcome
                                      |          |
                                  rule pack   Coverage
                                   (TOML)    (+ shapes)
```

Every arrow is a trait or a data file. None of them knows about the game.

## Framing

Trivial in a batch parse, not while tailing: the game flushes mid-line, so a
chunk boundary lands anywhere. `Framer` carries a partial line across pushes.

It also refuses to be a memory bomb. A file with no newline (corrupt, or a
binary opened by mistake) must not grow the carry buffer without bound, so an
over-long line is truncated, counted, and resynchronised at the next newline.

`flush()` is for batch EOF only. Calling it while tailing emits a line the game
has not finished writing.

## Header

`[Tue Jul 28 15:02:08 2026] ` — fixed-offset, no `chrono`, no allocation.
Accepts space- and zero-padded days; we do not know which libc wrote the file.

Timestamps carry **no timezone**. The game writes local time with no offset and
no DST marker; inventing a zone would be an unverifiable assumption that cannot
be undone downstream. Ordering and deltas — all a DPS window needs — are correct
regardless. Anything needing a true instant attaches a zone at the edge.

## Classification — why two stages

A mature pack is several hundred regexes. Running them in sequence is O(rules)
per line and dies at scale. A single alternation matches fast but cannot say
which branch won, and its DFA explodes as the pack grows.

Game log lines are highly templated, so one Aho-Corasick pass finds every literal
anchor present, reducing hundreds of candidates to typically zero or one. Only
those get a regex run.

**Anchors are pure optimisation and must be semantically invisible.** A rule is a
candidate iff every anchor it declared was found, so anchors can never cause a
false negative unless an anchor is wrong about its own pattern. That is what
`anchors_never_change_the_answer` exists to prevent — it builds two engines from
one pack, one with anchors stripped, and asserts identical output.

**`excludes` are the opposite**: literal vetoes that change meaning on purpose.
Rust's regex crate has no lookaround (that is what buys linear-time matching), so
"match X but not when Y is present" has no clean regex spelling. A literal veto is
clearer than a contorted pattern and costs nothing — exclusion literals ride along
in the same Aho-Corasick pass.

Found by lint on real data: `melee.hit` was eating
`"...was hit by non-melee for 100 points..."` as `src="a skeleton was",
verb="hit"`. That is what motivated `excludes`.

## Capture extraction is opt-in

Pulling capture groups costs roughly 2.2x everything else combined (measured:
~900ns/line with, ~320ns without). So it is a runtime mask, not a fixed
behaviour. A DPS meter alone pays for damage captures and nothing else; open a
loot panel and the mask widens.

## Zero-copy

`Match` owns no `String`. It is byte offsets into the caller's line buffer, so
the hot path allocates zero times per line. `MAX_CAPS` is a fixed array; rules
needing more are rejected at pack-compile time with a clear error rather than
silently truncated.

## Outcome is total

Every line lands in exactly one of Matched / Unmatched / Headerless / Blank.
There is no path that silently drops a line, which is what makes coverage a real
number rather than a guess.

## Shape clustering

Turns "1.8M lines I don't handle" into a ranked list of templates. Runs with no
pack at all, so a log nobody has parsed yields its own rule backlog.

The naive version — digits to `#`, capitalised words to `@` — over-fragments
badly. Three failures found on a real 1.8M-line log:

1. Multi-word names became several placeholders, so `casting Lifetap` and
   `casting Garrison's Mighty Mana Shock X` were different shapes. One template
   shattered into a dozen entries.
2. `You` and `Your` were treated as names, merging self with everyone else — the
   distinction a combat log most needs to keep.
3. Names containing lowercase connectives (``Footman of V`Zher``,
   `Blessing of the Squire`) split around the connective.

Hence tokenise-then-collapse, an English function-word list (no game knowledge),
and connective bridging. Leading punctuation ends a run: the `(` in an item proc
is not part of the item name.

## Rule packs

Rules are data. Nothing in the crate knows "damage" exists — a rule is
`id + regex + anchors + a free-form kind string`. Delete every rule and you have
a working parser at 0% coverage, which is the property we want.

Packs layer: `--pack base.toml --pack mine.toml` merges by id, last wins, and
`enabled = false` removes a base rule without editing it. Local experiments never
fork the base pack.

`examples` and `counterexamples` are executable — `eqlp lint` runs each through
the whole engine and fails if another rule claims them. Shadowing is caught the
moment it is introduced.

## Known ceilings

- Mob names carry no instance id. Two mobs called `an abhorrent` are
  indistinguishable in every line. This is why charm-pet attribution is
  impossible and why encounters keying on name can merge.
- Landed damage lines drop the spell rank; rank exists only on the cast line.
- ~20% of spell-damage landings have no cast line anywhere in the log. Those are
  weapon procs, and that is a complete answer rather than missing data.

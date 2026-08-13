# eqlp — an EverQuest Legends log parser core

Rust workspace. The parser is a standalone library with **no dependency on
Tauri, on file I/O, or on a UI**.

```
crates/core/     framing → header → classification → coverage.  Pure bytes in, outcomes out.
crates/cli/      eqlp lint | parse | coverage | shapes | bench.  No webview involved.
packs/eql.toml   the rules, as data
corpus/gen.py    synthetic log generator for tests that must not depend on a real log
```

That boundary is the point. When parsing and rendering are tangled, neither can
be tested alone, which is why UI bugs and parse bugs start feeling like the same
bug. Here the expensive-to-get-right half is a pure function you can fuzz,
benchmark and diff without opening a window.

## Status against the reference log

`eqlog_Manipulator_rivervale.txt` — 1,834,873 lines, 147 MiB, 28 Jul – 9 Aug 2026.

| | |
|---|---|
| Coverage | **81.9%** of event lines claimed by a rule |
| Throughput | 2.4 s for the whole file (0.77 M lines/s, 62 MiB/s) |
| With capture extraction off | 1.1 s (1.7 M lines/s) — 2.2× |
| Malformed / headerless lines | **0** |

Measured on a single shared vCPU with Rust 1.75; treat the ratios as solid and
the absolutes as a floor.

## Pipeline

```
bytes ──▶ Framer ──▶ HeaderParser ──▶ Matcher ──▶ Outcome
                                         │           │
                                    rule pack     Coverage
                                      (TOML)     (+ shapes)
```

Every arrow is a trait or a data file. None of them knows about the game.

### Framer
Carries partial lines across chunk boundaries, since the game flushes mid-line
while tailing. Refuses to be a memory bomb: an over-long line is truncated,
counted, and resynchronised at the next newline.

### HeaderParser
`[Tue Jul 28 15:02:08 2026] ` — fixed-offset, no `chrono`, no allocation.
Accepts space- or zero-padded days. Timestamps are stored as the log wrote them
with **no timezone attached**: the game gives local time with no offset, and
inventing a zone would be an unverifiable assumption that cannot be undone
later. Ordering and deltas — all a DPS window needs — are correct regardless.

### Matcher — two stages, and why
A mature pack is hundreds of regexes. Running them in sequence is O(rules) per
line; a single giant alternation cannot say which branch won and its DFA
explodes as the pack grows.

Game log lines are highly templated, so: one Aho-Corasick pass finds every
literal *anchor* present in the line, which reduces hundreds of candidate rules
to typically zero or one, and only those get a regex run.

Anchors are **pure optimisation and must never change results** — see the
differential test below.

`excludes` are the opposite: literal vetoes that change meaning on purpose.
Rust's regex crate has no lookaround (that is what buys the linear-time
guarantee), so "match X but not when Y is present" has no clean regex spelling.
Expressing it as a literal veto is clearer than a contorted pattern, and free —
the exclusion literals ride along in the same Aho-Corasick pass.

### Capture extraction is opt-in
Pulling capture groups out costs roughly 2.2× everything else combined. So it is
a runtime mask, not a fixed behaviour: a DPS meter alone pays for damage
captures and nothing else; open a loot panel and the mask widens.

```rust
let mut m = engine.matcher();
m.capture_only(&[dmg_rule, heal_rule]);   // everything else: boolean match
```

## The iteration loop

```
eqlp shapes   yourlog.txt --top 60                      # what does the log actually say?
eqlp coverage --pack packs/eql.toml yourlog.txt --top 40 # what don't we handle yet?
   ... write rules for the top shapes, with examples ...
eqlp lint     --pack packs/eql.toml --against yourlog.txt --min-rate 0.95
```

`shapes` runs with **no pack at all**. Point it at a log nobody has parsed and it
collapses the variable parts of every line and ranks the skeletons by frequency.
Rule authoring becomes "work down the list."

Getting that clustering right took three fixes, each found on real data:

1. **Multi-word names became multiple placeholders**, so `casting Lifetap` and
   `casting Garrison's Mighty Mana Shock X` were different shapes — one template
   shattered into a dozen list entries. Fixed by collapsing runs.
2. **`You` and `Your` were treated as names**, merging self with everyone else.
   Fixed with a list of English function words. No game knowledge encoded.
3. **Names containing connectives** (``Footman of V`Zher``, `Blessing of the
   Squire`) split around the `of`. Fixed by bridging connectives inside a run.

Together these took the top-60 coverage of the shape list from 28.1% to a list
where the top entry is a real template rather than a fragment.

## What the tooling caught

Three of these are bugs the tooling found in my own rules, which is the case for
having it:

- **`lint` caught a shadowing collision**: `melee.hit` was eating
  `"...was hit by non-melee for 100 points..."` as `src="a skeleton was",
  verb="hit"`. That is what motivated `excludes`.
- **30,164 lines said `1 point of damage`, singular.** The anchor
  `"points of damage."` silently dropped every one. Anchor is now
  `" of damage."`.
- **22,375 hits carry a trailing flag** — `(Critical)`, `(Riposte)`,
  `(Rampage)`, `(Crippling Blow)`, `(Double Bow Shot)` and combinations. The
  pattern's `$` dropped all of them, including 15,798 crits: precisely the stat
  a DPS meter leads with. Fixing these two recovered **52,633 events** in
  `melee.hit` alone.
- **`zone.enter` was matching** `"You have entered an area where levitation
  effects do not function."` Fixed with an exclude.

## Where EQL differs from classic EverQuest

Every pattern in `packs/eql.toml` came from the log. Where classic-EQ intuition
and the log disagreed, the log won:

| | classic EQ | EQL |
|---|---|---|
| XP | `You gain experience!!` | `You gain experience! (11.000%)` — with a percentage |
| Spell damage | `X was hit by non-melee for N` | `X hit Y for N points of magic damage by Spell.` — **the classic form does not occur even once** |
| Heals | `X has healed Y for N` | `X healed Y for N (M) hit points by Spell.` — the `(M)` is the uncapped amount |
| Loot | `--You have looted a X.--` | `--You have looted a X from a Y's corpse.--` |
| DoT | — | `X has taken N damage from Spell by Caster.` (and a rarer uncredited form) |

## Tests

`cargo test --release` — 19 unit + 7 integration, all against real log data.

The load-bearing one is **`anchors_never_change_the_answer`**: it builds two
engines from the same pack, one with anchors and one with every anchor stripped,
and asserts byte-identical output across the corpus. A wrong anchor is the one
bug class that could silently drop events forever; this makes it unshippable.

Alongside it:

- `matcher_state_does_not_leak_between_lines` — the matcher reuses scratch
  buffers, so results must not depend on history. Checked forward, reversed, and
  against virgin matchers.
- `epoch_wraparound_is_safe` — forces the u32 anchor-dedup epoch to wrap.
- `streamed_framing_equals_batch_framing` — eight chunk sizes from 1 byte up.
  This is what makes a live tail behave identically to a batch reparse.
- `arbitrary_bytes_never_panic` — random bytes, valid-header-plus-garbage, and
  every truncation of a real line (what a partial flush looks like).
- `coverage_buckets_sum_to_the_line_count` — no line silently disappears.
- `lazy_captures_match_eager_captures`.

`eqlp lint` is the CI gate and needs no webview:

```
eqlp lint --pack packs/eql.toml --against real.log --min-rate 0.95
```

It runs every rule's declared `examples` through the whole engine and fails if
another rule claims them, verifies each anchor appears in its own examples,
flags rules that never fire against a real log, and gates on coverage.

## Next

**The remaining 18% is one architectural piece, not 200 more regexes.** The
backlog is now dominated by spell landing and fading text — `Your feet move
faster.`, `You feel an aura of mystic protection surrounding you.`, `The jig
sends energy zinging through your body.` These are not templates with variable
slots; they are a *dictionary* of fixed strings, one or more per spell. The
right answer is an exact-body hash lookup consulted before the regex stage —
O(1), and it drops in behind the existing `Classifier` seam without touching
anything above it. That should take coverage well past 95%.

Also outstanding:

- **The live tailer** — rotation, partial flushes, the Windows file-locking
  dance. `Framer` is already built for it.
- **The Tauri boundary.** The fix for Electron-style jank is batching snapshots
  to the webview at ~10 Hz through a bounded ring buffer rather than emitting
  per event. At 0.77 M lines/s the parser is nowhere near the bottleneck; the
  IPC chatter is.

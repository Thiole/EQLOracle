# Backlog

Parked deliberately. Each entry records what it needs, so when the base is
solid it's obvious whether a thing is ready or still blocked.

The rule for this file: nothing here gets started until the base checklist at
the bottom is green. Ideas are cheap to write down and expensive to half-build.

---

## Rotation advisor — parked

**Not a DPS ranker.** DPS is one input among several, and optimising on it alone
would give confidently wrong advice. The real inputs:

| factor | what it needs | have it? |
|---|---|---|
| Cycle time per spell transition | cast→cast timing by pair, invocation-aware | measured, ad hoc |
| DoT setup time / uptime | duration model per DoT, refresh detection, tick attribution | no |
| Land probability per target | resist tracking keyed by mob type and resist school | no |
| Damage per attempt | cast→damage pairing to recover rank | no (rank not on landed line) |
| Mana economy | mana cost per rank, regen, invocation effects | no — log may not carry mana at all |
| Interrupt exposure | why casts fail; movement/stun correlation | partial |
| Target lifetime | encounter duration model — a DoT on a mob that dies in 6s is waste | no |

Observations worth keeping from the reference log, all provisional:

- Alternating spells beats repeating one: 2.28s vs 2.70s cycle in arcane
  mastery, ~19% more sustained nuke DPS. The extra ~0.5s on a repeat is per-spell
  recast, not player slack.
- Transitions are **directional**. `GMMS -> Ice Comet` is 2.16s but
  `Ice Comet -> GMMS` is 3.12s, and `Ice Comet -> Ice Comet` is 3.83s.
- `Elemental Maelstrom -> GMMS` is 1.77s over 489 samples — the fastest observed
  transition, and underused (1,241 EM casts vs 12,293 GMMS X).
- Invocation state dominates everything: arcane 2.64s vs recovery 3.43s on the
  same spell and rank.

All measured at 1-second log resolution via fractional-gap estimation, so treat
±0.1s as noise and never show more precision than that to a user.

**Blocked on:** rank recovery, invocation state on events, encounter model,
resist tracking. Three of those are base work anyway.

## Exact-body dictionary classifier

The remaining ~18% of unmatched lines is spell landing/fading flavour text —
`Your feet move faster.`, `The jig sends energy zinging through your body.`
These are fixed strings, not templates with slots. A hash lookup on the whole
body, consulted before the regex stage, handles them in O(1). Should take
coverage past 95%.

**Blocked on:** the `Classifier` trait actually existing.

## Charm / pet attribution

See the analysis: not solvable in general. `an abhorrent has been charmed.` never
names a caster, and charmed mobs carry no ownership marker. Model as a
contested pool with named candidates; never split silently. The valuable feature
is *detecting and announcing* that attribution is impossible for a given fight.

**Blocked on:** encounter model, and an `Attributor` seam.

## Melee rule coverage

The reference log is a caster's. Third-person melee forms are well exercised by
other players' swings; first-person forms (`You slash...`, your own crits,
ripostes, skill-ups) are nearly untested. Needs a melee player's log, then
`eqlp lint --against` will surface the gaps in seconds.

## Cycle efficiency readout

Achieved cycle vs theoretical, per spell, per invocation. Novel — no parser I
know of shows it, and it tells a player something actionable in a way a damage
total doesn't. Cheap once rank recovery and invocation state exist.

---

# Base: definition of done

Not features. This is the list that has to be green first.

### Parser
- [x] Framing, header, classification, coverage
- [x] Anchors proven semantically invisible (differential test)
- [ ] `Classifier` extracted as a real trait — currently only a doc comment,
      and the dictionary classifier depends on it
- [ ] Coverage >= 95% on the reference log
- [ ] Melee first-person rules exercised against a melee log

## Class-only melee skills — blocked on evidence

**Needs: one real Flying Kick damage line.** Reported as "flying kick is also a
100% monk skill so class detection should be able to use that", and the data
already agrees — `packs/skill_classes.json` carries 26 single-class skills
including Flying Kick, Tiger Claw, Dragon Punch, Eagle Strike, Round Kick and
Tail Rake. `ingest::class_only_melee` reads none of them; it is a hand-written
three-name match (Backstab/Frenzy/Shoot).

Two things block the obvious fix:

- **The generalisation is unsafe as-is.** `Smite` and `Cleave` are single-class
  in that file AND generic melee verbs in the pack's own damage pattern — any
  mob smites. Deriving class evidence from the file wholesale would attribute
  Cleric to every smiting gnoll. Tried, caught, reverted. The exclusion has to
  be the verbs `canonical_melee_ability` can emit, and Backstab/Frenzy are in
  that set while genuinely being exclusive, so no data signal separates them.
- **The parser is the real blocker, not the table.** There is no Flying Kick
  damage line anywhere in the 396 MB reference log — only chat mentions of it.
  This character is not a Monk and no Monk ally produced one, so the line shape
  is unknown and must not be guessed (see the rule-pack discipline: every
  pattern comes from a real line).

**To unblock:** one log excerpt of a Monk landing a Flying Kick, with its
timestamp. That gives the verb form for the melee pattern and the mapping in
one go, and the same excerpt becomes the replay test.

---

### Event model
- [ ] **Reduce `eqlp-session::Tracker` to an encounter builder.** It currently
      owns damage totals and per-source `Rolling` buffers, which duplicates the
      store. It should decide fight boundaries and emit ranges, holding no
      combat data. Two places of truth is the thing the store exists to prevent.
- [ ] **Ingest: parser events -> store.** Melee verb becomes the ability
      (`backstabs` -> `Backstab`); spell name becomes the ability; tags derive
      from the rule `kind` plus cast-line presence.
- [ ] **Rank recovery** — the landed line drops the numeral
      (`by Garrison's Mighty Mana Shock.`), so rank only exists on the cast
      line. Without cast→damage pairing, per-rank numbers are unavailable and
      aggregate damage silently blends five different spells.
- [ ] **Invocation state on every event** — a 30% swing on identical casts. Any
      damage figure that ignores it is wrong, not merely coarse.
- [x] Encounter model — per-target, streaming, bounded memory, kill vs reset
- [ ] Death-message sparsity: only ~21% of encounters get a confirmed kill
      line. `Reset` mostly means "never saw the death", not "mob escaped".
      TTK is only valid on confirmed kills and must be labelled as such.

Those two are listed as base rather than backlog on purpose: they are not
enrichments, they are the difference between a number being right and wrong.

### Plumbing
- [x] Clock injected, enforced in CI
- [x] Tail with rotation/truncation/missing handling
- [x] Deterministic replay
- [ ] IPC contract — one schema, TS generated from Rust, golden fixtures
- [ ] Mock IPC harness — frontend boots in a plain browser, no Rust
- [ ] Capability detection + window roles (docked / floating / click-through)
- [ ] CI green — the workflow file exists but references npm scripts and
      fixtures that don't yet

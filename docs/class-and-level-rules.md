# Class and level rules -- document of truth

Every rule the app uses to decide who is what class, at what level, and
when your own buffs are gone. One rule per line, with the log line that
feeds it. If behavior disagrees with this file, one of them is a bug.
Open questions at the end need Spencer's answer before they become rules.

Code: `crates/session/src/classdetect.rs` (your classes),
`crates/app/src/ingest.rs` (evidence hooks, ally chains, buff ledger),
`crates/app/src/combat.rs` (`you_level_at`, ally table rows).

## Game facts these rules rest on

- G1. A character runs exactly 3 classes at once (`CLASS_COUNT`). Swapping
  happens in town, never mid-zone.
- G2. A loadout swap (classes, race, or slot) prints nothing and strips
  every buff, also silently. Confirmed in the real log.
- G3. "You have gained a level! Welcome to level N" is the *lowest* of the
  three classes, not any one class. A swap can drop it silently.
- G4. An item click prints the same "You begin casting X" as a spellbook
  cast, but always the bare spell name. Rank suffixes (X, IX ...) are
  spellbook casts only.
- G5. Symphonic Aura sings silently: no "You begin singing" for its songs.
  Only a Bard has it.
- G6. Wiki spell levels are not this server's levels (Improved Invisibility
  is listed Wizard 55 on a level-50 cap). They are never used as evidence.

## Your own classes (classdetect)

- C1. Evidence is a cast, song, stance, invocation, or AA line by You. Each
  maps to the set of classes that can use it (packs: spell_classes,
  stance_classes, invocation_classes, aa.json category).
- C2. Evidence is grouped per zone visit. Nothing is evicted or decayed.
- C3. A single-class ("unambiguous") sighting confirms that class once seen
  on 2 distinct visits (`MIN_UNAMBIGUOUS_CASTS`), retroactively on both.
- C4. Elimination (multi-class pools intersecting down to one class)
  confirms only after 3 distinct visits (`MIN_ELIMINATION_CASTS`).
- C5. Once a class is proven by either path it confirms instantly on any
  later visit it is sighted in.
- C6. A contradiction (a pool with empty intersection against the visit's
  narrowing) poisons that visit's elimination for good. Unambiguous
  evidence is unaffected.
- C7. A visit with fewer than 3 confirmed classes is partial, never shown
  as its own configuration in the configurations view. The ally table's
  You row does show the partial set for the fight's visit; with none it
  falls back to your most-played full configuration.
- C8. Excluded from evidence: teleport spells; Origin; spells with a known
  item *click* source (324 spells) when cast with no rank suffix.
  Proc/worn/focus item effects do not exclude. A ranked cast of a
  clickable spell counts.
- C9. Pets' casts are never class evidence for anyone.
- C10. AA names with a ": Enabled"/": Disabled" suffix fall back to the
  bare catalog name.

## Bard through Symphonic Aura

- B1. First-person aura lines are Bard evidence for that visit:
  "You have improved/gained Symphonic Aura: ...", "This song cannot be
  played while Symphonic Aura is enabled.", "Your <song> paused/resumed."
- B2. While an aura line has been seen in the current zone visit, a
  landing on you whose every candidate spell is Bard-only counts as your
  own cast. The flag resets on every zone-in and on a loadout change.
- B3. Allies running the aura are invisible: nothing in your log names
  who is singing silently. They confirm only from manual songs or /who.

## Allies

- A1. An ally's classes come from votes: their cast lines, damage/heal
  "by <spell>" lines, Quick Buff landings, class-only melee verbs
  (Backstab, Frenzy, Shoot). Chains are per ally per stretch of activity.
- A2. A chain ends on 5 minutes of silence, group leave/join, your zone
  line, or their gate/teleport cast followed by silence. The next chain
  starts with a soft prior of 0.4 from the old one.
- A3. A /who line with classes pins that chain's classes and level. /anon
  hides classes and pins nothing; inference carries on.
- A4. "Confirmed" for the Group Buff Tracker means the /who pin or 12+
  votes (`CONFIRMED_VOTES`). Pets are excluded.

## Your level

- L1. The latest ding is the effective level and the floor of the answer.
- L2. Each class keeps its own floor: the highest ding on any visit whose
  resolved full trio contained it. Floors never go down.
- L3. The row's level is the current trio's lowest class floor, never
  below L1. A trio class with no floor yet makes L1 the answer.
- L4. Spell levels are never a floor (G6).

## Loadout swap signals (clear the buff ledger, reset B2 and the class set)

- S1. A 4th distinct single-class cast by You since the last mark
  (G1: the set changed). Buffs landed before that cast are dropped.
- S2. 3+ "You have been granted the following spell" lines within 2s with
  no ding in the previous 15s (a class pick in town). 1 grant is a scribe.
- S3. Your own death clears every buff.
- S4. A swap to a slot with the same three classes is undetectable.

## Group Buff Tracker ledger

- T1. A beneficial party buff (or Self-target spell) whose landing text
  hits you goes on the ledger with the spell's own duration as expiry.
- T2. Shared landing/wear-off text (rank siblings) adds/removes every
  candidate spell; the widget reads by buff kind, so any candidate counts.
- T3. A wear-off text takes the spell off. A swap (S1-S3) takes them all off.
- T4. "Good" means every buff kind a confirmed (A4) party class can cast
  that benefits your class combo is on you.

## Open questions

- Q1. Level cap: is the server cap 50 right now? Should any level above
  it be treated as data error?
- Q2. L3 when a trio class has never dinged under a resolved trio: show
  the latest ding, or show "?" for the level?
- Q3. S1 uses the wiki's class lists. Leech is listed Necromancer-only
  but SK casts it here. Is a curated "server exceptions" list wanted, or
  keep the rare false swap (cost: one wrong "missing" until Quick Buff)?
- Q4. Should a /who on yourself (if it prints your own classes) override
  detection for that visit the way it does for allies?
- Q5. Which character does the app tail? Newest-modified eqlog in the
  install's Logs folder. With two boxes logging to one folder it would
  flip between them. Is a character picker wanted?

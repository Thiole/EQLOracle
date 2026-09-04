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
  Only a Bard has it. Its Self-target songs still land: "Your voice
  booms." on the singer, "<Name>'s voice booms." in everyone else's log
  (Amplification), every 6 seconds.
- G6. Spell level requirements are per CLASS, and the install's own
  `spells_us.txt` states them (columns 37-52, one per class, 255 = that
  class cannot cast it). Game data, this server -- same footing as the
  col-55 reuse timers, not a wiki scrape. A level listed above the cap
  is not bad data, it means no one here casts it as that class:
  Improved Invisibility is WIZ 55 and ENC 50, and an ENC/WIZ character
  at the cap casts it as the Enchanter. Reading the wizard entry off it
  and calling the file wrong was the "WIZ 55" bug (see L8).
- G7. Poisons are Rogue-only. They are ability activations whose name ends
  in " Venom" or " Poison"; the follow-up is "<Name> coats their blades in
  ...". Backstab is Rogue-only.

## Your own classes (classdetect)

- C1. Evidence is a cast, song, stance, invocation, AA line, or skill-up
  line ("You have become better at X!") by You. Each maps to the set of
  classes that can use it (packs: spell_classes, stance_classes,
  invocation_classes, aa.json category, skill_classes).
- C1b. skill_classes holds only single-class skills verified on the wiki's
  class pages (2026-09-03: Rogue, Monk, Warrior, Berserker, Paladin, SK,
  Bard, Druid entries) plus Tracking. Multi-class pools (Kick, Bash, Slam,
  Taunt, Sneak, Hide, Dual Wield, Double Attack, Triple Attack) are left
  out: 11 class pages have no skill section, and an incomplete pool would
  falsely eliminate a class. Forage stays out: Iksar and Wood Elf get it
  from race regardless of class.
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

There is ONE class model (P1-P10), for you and for every tracked player.
Your own log simply feeds it more kinds of evidence about you. Only the
rules below are ally-specific, and they only decide where a chain breaks.

- A1. An ally's evidence is their cast lines, damage/heal "by <spell>"
  lines, Quick Buff landings, and class-only melee verbs. It goes into
  the same detector, keyed by the ally, in the encounter it happened in.
- A2. Their chain is cut by 5 minutes of silence, a group leave or join,
  your own zone line, or a gate they cast and went quiet after. The
  detector's own carry rules (P4) then apply.
- A3. A /who row is ground truth for the chain it printed in: its trio
  wins over inference and its level floors every class in it. /anon
  prints no classes and pins nothing. Your OWN row (matched against the
  log's character name) lands on "You", so it reaches every surface.
- A4. "Confirmed" means a /who row, or a class that cleared the
  detector's own bar (P2). Pets are excluded and never get a chain.

## Your level

Level is a rolling record per class (P6), not something an encounter
re-derives. These rules say what writes it, what it constrains, and how
readily a written value is revised.

- L1. Two quantities, never mixed. A class's own RECORD persists once
  reached and never falls. The EFFECTIVE level while a trio is slotted
  is the minimum of the three records -- a 50 class beside a 26 class
  plays at 26, temporarily de-levelled, its own record untouched. Dings,
  spell access and the row's displayed level are all effective level;
  elimination (L5) runs on records.
- L2. A ding to N sets the effective level to N, so every class in the
  trio is at least N: raise all three records to N (max, never lower).
  A /who row does the same at the level it states, for every class it
  names, for that player or for you.
- L3. Each record entry carries its source, and the source decides how
  readily it is revised -- how quickly the algorithm self-corrects, not
  whether it can:
  - CEMENTED: a /who row, or a ding under a trio confirmed at the P2
    bar. Revised only by another cemented source.
  - FIRM: a spell floor (L8) under a confirmed trio. Revised by a
    cemented source, or by corroborated contradiction.
  - SOFT: anything absorbed under a prior or an unconfirmed trio.
    Revised freely by the next better evidence.
  A level is only ever written for a class the chain has CONFIRMED --
  never a prior or a leading guess. Writing guesses too was measured on
  the real log: every class drifted to ~50 (Warrior 48, Beastlord 47 on
  a character that plays neither) and those records then eliminated real
  trios through L5. The same level proven again by a stronger source
  firms up in place rather than being ignored.
- L4. One strand per class, rewritten in place. When later evidence
  re-attributes an arc (an ally's 26-28 turning out to be the
  Necromancer), the strand is corrected from that date forward; no
  competing guesses are kept alongside it. Reads are as-of-time: a
  record raised at T never constrains anything before T.
- L5. A ding is a constraint on the trio, not just an output of it.
  A SOFT record only ever observes: it never eliminates a trio and never
  closes a chain (Q41, settled by replay -- letting one eliminate fed a
  guess back in as its own proof and unresolved visits jumped 245 -> 489).
  A ding to N at time T means the trio held a class whose record was
  exactly N-1 as of T, so:
  - a trio whose three records were all above N-1 takes a heavy miss
    (weight by L3 tier: cemented -3, firm -2, soft -1, provisional
    until replay calibrates them). Never a hard reject -- a wrong
    record must not permanently rule out the truth.
  - a trio holding a class whose record was exactly N-1 scores +1.
  Real case: the Aug 10 dings 26/27/28 were filed under ENC/SHD/WIZ,
  a trio sitting at 34 the night before. It cannot ding 26. The arc
  was the Necromancer's, whose record stood at exactly 25.
- L6. A backwards ding is a swap signal (S5) only when it contradicts
  the trio the chain currently assumes -- when believing otherwise
  needs a trio to ding below its own minimum. If the chain already
  holds a different trio there, the ding confirms it instead.
  No de-level line exists in any real log (395MB checked), so a
  backwards step is always a swap, never a loss.
- L7. Configurations split on ARC, not wall clock. A ding sequence
  stepping backwards is two arcs by definition (L6); the 24h
  SESSION_GAP_MS bucket welded an ENC/SHD/WIZ 33-34 run to a
  Necromancer 26-28 run and reported the range as (26,34).
- L8. A cast proves the effective level, hence a floor under every
  class in the trio. The requirement is the MINIMUM `spells_us.txt`
  level among the trio's classes that can cast it (G6) -- Conflagration
  is WIZ 43, so casting it in a WIZ/ENC/BRD trio floors all three at 43.
  Only a cast that is provably from the spellbook counts:
  - a rank suffix (I-X) is spellbook-only (G4) -- 75,280 of 94,212 real
    casts carry one, including all 9,544 Conflagration casts;
  - else the measured begin->resolve interval matches col-8 base cast
    time under the modeled focus/AA cut (an instant resolve on a 5s
    spell is an item click, a 15s one is the Vermilion Robe);
  - else the spell has no known click source at all (not in the 324).
  A cast clearing none of the three sets no floor. A cast at a known
  effective level BELOW the file's number ratchets that spell's
  requirement down and flags it -- the log corrects the file, never the
  other way. A floor is never written above the chain's own highest
  ding: every ding is logged, so inside one loadout the level between
  dings is known exactly, and a cast needing more than that did not
  happen in this trio -- the assumption is stale, not the level. Without
  that cap a level-49 spell put a Firm 49 on a Necromancer whose own
  dings had it at 32.
  Not yet implemented: the cast-time branch. Rank suffix and
  "no click source exists" cover 75,280 of 94,212 real casts today.
- L9. The Character Planner reads the rolling record, not configuration
  level ranges. The old estimator took the highest ding inside the
  sessions of configurations a class was confirmed in, so a class that
  stopped producing distinguishable evidence froze at its last provable
  level (NEC read 25 while its arc really ran to 28 and beyond).

## Loadout swap signals (clear the buff ledger, reset B2 and the class set)

- S1. A 4th distinct single-class cast by You since the last mark
  (G1: the set changed). Buffs landed before that cast are dropped.
- S2. 3+ "You have been granted the following spell" lines within 2s with
  no ding in the previous 15s (a class pick in town). 1 grant is a scribe.
- S3. Your own death clears every buff.
- S4. A swap to a slot with the same three classes is undetectable.
- S5. A ding below the assumed trio's own minimum (L6). The only swap
  signal that fires with no cast, grant or death behind it.

## Group Buff Tracker ledger

- T1. A beneficial party buff (or Self-target spell) whose landing text
  hits you goes on the ledger with the spell's own duration as expiry.
- T2. Shared landing/wear-off text (rank siblings) adds/removes every
  candidate spell; the widget reads by buff kind, so any candidate counts.
- T3. A wear-off text takes the spell off. A swap (S1-S3) takes them all off.
- T4. "Good" means every buff kind a confirmed (A4) party class can cast
  that benefits your class combo is on you.

## Chain model (confirmed by Spencer 2026-09-03, being built)

Replaces C2-C8. One rolling evidence chain per character;
no static zone lists anywhere.

- P1. The unit of evidence is the encounter (one You are in, from the
  first line that proves your involvement to its close). Casts between
  encounters attach to the next one; a zone line outside any encounter
  also ends the pending unit. Evidence is attributed all at once
  (Spencer, 2026-09-03): every unit is scored against every possible
  trio (560). A trio the unit fits gains 1, one it contradicts loses 1,
  uncapped, so the trios that fit the whole chain lead. The best guess
  is the intersection of the leading trios; what they disagree on is
  the open slot's candidates. A zone line halves every score (P4).
  The earlier per-fight decay (unsupported -0.5) is gone: a confirmed
  class stays confirmed through fights that show nothing of it.
- P2. A class inside the leading intersection counts as confirmed once it
  has 2 encounters of class-only evidence, or 3 encounters of pool
  evidence, and the leading score is at least 2. Nothing is ever forced.
- P3. Sources: casts, songs, stances, invocations, AA lines, skill-up
  lines (C1b), ability activations (poisons per G7), Bard song landing
  lines on you ("Your voice booms." and every other Bard-only song text),
  and Bard song landing lines in third person, which name the singer
  ("Cauth's voice booms.").
- P4. A zone line, including a confirmed teleport, never breaks the chain.
  It weakens it: the trio carries as a prior that fresh evidence must
  re-clear at the P2 bar. A class proven long ago and re-sighted: prior,
  same bar, then confirmed retroactively over the chain.
- P5. A contradiction (evidence no trio can hold) starts a count. After 3
  consecutive conflicting encounters the chain closes retroactively at the
  encounter where the conflict began, shown "??" from there; a new chain
  starts there and confirms on its own. Until then the row shows the old
  trio with "?". The close lands when the third
  conflicting encounter ends, since a unit's evidence isn't final until then.
- P6. Level is a ROLLING record per class, kept for the character and
  never re-derived per encounter or per chain. A ding raises every class
  confirmed in the chain at that moment; a /who row raises every class it
  names to the level it states (the game shows the trio's lowest, so all
  three are at least that); nothing ever lowers one. The row shows the
  lowest record among the trio AS SHOWN, never below the latest ding, and
  a class with no record yet falls back to the latest ding. What writes
  the record, what it constrains and how readily it is revised are L1-L9.
  Without the rolling record a class swapped in
  after you reach the cap never dings again, and the row reads the
  previous trio's level -- real report: "ENC/SHD/WIZ 41" sitting beside
  its own /who row saying 50.
- P7. Pets: a pet's own casts are never evidence; the summon that produced
  it is. Charm pets prove nothing.
- P8. Loadout swap signals (S1-S3) close the chain as P5 does, without the
  3-encounter wait.
- P10. The stance and invocation in effect are states, not actions: the
  last one seen counts as evidence in every encounter until changed. A
  class swap takes you out of every stance (S1-S3, and a P5 close, drop
  it); the invocation survives a swap.
- P9. Display (Q34): the You row shows the trio; a prior is dimmed; an
  open slot shows "?"; a running conflict adds " ?"; a chain closed by
  contradiction adds " ??". Expanding the row lists what the open slot is
  stuck between, the priors, the conflict count, and how the chain ended.
  The configurations view lists chains (full trios only) with their
  encounter counts; its drill-down shows the zone visits those
  encounters sat in.

## Open questions

- Q1. Level cap: is the server cap 50 right now?
- Q4. Should a /who on yourself (if it prints your own classes) override
  detection the way it does for allies?
- Q5. The app tails the newest-modified eqlog in the install's Logs
  folder. Two boxes logging to one folder would make it flip. Character
  picker wanted?
- Q42. L5's tier weights (-3/-2/-1) are provisional -- calibrate against
  a real replay before they are treated as settled.
- Q9. Wiki class-list errors (Leech listed Necromancer-only, SK casts it
  here): curated exceptions pack, automatic distrust on contradiction
  with a confirmed trio, or both?


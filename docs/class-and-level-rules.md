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
- G6. Wiki spell levels are not always this server's levels (Improved
  Invisibility is listed Wizard 55 on a level-50 cap). Any wiki level
  above the cap is discarded for that class (Q1 pending).
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

## Proposed model (Spencer's caveats of 2026-09-03, not built yet)

Replaces C2-C8 and L2-L3 once confirmed. One evidence chain per
character, rolling; nothing below is a static zone list.

- P1. The unit of evidence is the encounter, not the zone visit. Casts
  between encounters attach to the next encounter. (Q29)
- P2. Unambiguous evidence confirms a class after 2 consecutive encounters
  carrying it; elimination after 3. Nothing is ever forced.
- P3. Evidence sources: casts, songs, stances, invocations, AA lines,
  ability activations (poisons per G7), melee skills with a class list
  (Backstab now; Kick/Bash/etc. once their class lists are verified for
  EQ Legends), Self-target spells landing on you ("Your voice booms."),
  and Self-target spells landing in third person, which name their
  caster ("Cauth's voice booms." is Cauth's own Amplification).
- P4. A zone line, including a confirmed teleport, never breaks the chain.
  It weakens it: the current trio carries as a prior that fresh evidence
  must re-clear at the same bar (P2). Same for a class proven long ago
  and re-sighted: prior, same bar, then confirmed retroactively over the
  chain.
- P5. A contradiction (evidence no configuration of 3 can hold) starts a
  count. After 3 consecutive encounters of conflicting evidence the chain
  closes retroactively at the encounter where the conflict began, shown
  as "??" from there; a new chain starts from that point and confirms on
  its own. Until then the row shows the old trio with "?".
- P6. Level: a ding raises every class confirmed in the chain at that
  moment. A class that confirms later in the same chain gets the chain's
  highest ding at that moment, retroactively within the chain only. The
  row shows the lowest level among the trio's classes. A class with no
  level in this chain shows the latest ding (Q6 answer: no guessing from
  a multi-class spell; a spell only one trio class could cast raises that
  class to its level, capped per G6).
- P7. Pets: a pet's own casts are never evidence, but the summon that
  produced it is (already the case); charm pets prove nothing.
- P8. Loadout swap signals (S1-S3) close the chain the same way P5 does,
  without the 3-encounter wait.

## Open questions

- Q1. Level cap: is the server cap 50 right now?
- Q4. Should a /who on yourself (if it prints your own classes) override
  detection the way it does for allies?
- Q5. The app tails the newest-modified eqlog in the install's Logs
  folder. Two boxes logging to one folder would make it flip. Character
  picker wanted?
- Q9. Wiki class-list errors (Leech listed Necromancer-only, SK casts it
  here): curated exceptions pack, automatic distrust on contradiction
  with a confirmed trio, or both?
- Q29. P1: does out-of-combat activity (buffing in town, porting) form
  its own "encounter" for counting, or attach to the next fight?
- Q30. P3 third-person Self-target landings for allies: one vote per song
  per encounter, or every 6-second pulse?

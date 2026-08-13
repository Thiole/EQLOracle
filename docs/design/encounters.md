# Encounters — design notes

Rationale for `eqlp-session::graph`.

## A fight is a connected component, not a target name

Keying encounters on the target's name splits a multi-mob pull into one fight
per mob, and merges two players' simultaneous fights against same-named mobs.

Instead: each damage line is an edge `(actor, target)`. Entities joined by an
edge are in the same fight. Components expire after `idle_ms` of silence.

Measured on the 1.8M-line reference log (446,749 damage edges):

| idle | encounters | median entities | largest component |
|---|---|---|---|
| 10s | 3,144 | 5 | 1.4% of all damage |
| 30s | 1,476 | 5 | 2.3% |
| 60s | 898 | 4 | 2.3% |

The feared failure — transitive chaining collapsing a crowded zone into one
blob — does not happen. The largest component holds 1.4% of damage at 10s.
Median 5 entities is a real fight: a player, a groupmate, a pet, a mob or two.

`transitive` and `max_entities` exist for zones crowded enough that the guard
matters. Neither is needed on this log.

## Why 10 seconds

It is where out-of-combat recovery begins, so it tracks a real game-state
boundary rather than a number tuned to make encounter counts look tidy.

The log offers no way to verify this: there are no combat-state lines, and
`You begin to regenerate` is a spell landing, not a state transition. So this is
game knowledge, and it is a `Policy` field rather than a constant.

## Linked fights

At 10s, a fight with a lull splits in two. That is correct by the combat-state
definition and wrong for "how long did that mob take", so a closed encounter
records `links_to`: a previous encounter it continues.

Two conditions, and conflating them is a bug that was made and caught here:

- **lookup** — any non-player in this fight may carry a link *back*. Whether it
  died here is irrelevant; a mob that fled and was finally killed is exactly the
  case the link exists for.
- **carry** — only a non-player that left the fight *alive* can be re-engaged
  later. A corpse cannot.

Players are excluded from both. You are in every fight you take part in, so
linking through a player would chain an entire evening into one encounter.

## Entity classification

Three kinds, by descending certainty:

- **Pet** — the ` pet` suffix names the owner (`Gynok Moltor pet`). The only
  ownership marker the log provides. `credit()` attributes damage to the owner.
- **Player** — used a player-only channel (group/guild/raid/General). NPCs use
  `says`, never these.
- **Unproven** — everything else. Deliberately not called `Npc`: it holds both
  real NPCs and players who have not spoken.

Player proof matters more than it looks. Named NPCs are indistinguishable from
players by name alone — `Ktik`, `Zobartik` and `Jabeker` read exactly like
character names, and a roster built from combat lines alone fills up with them.
In a test against the reference log, co-combat predicted group membership at
either ~100% or ~0%, and the 0% cases were all NPCs.

Charmed mobs are never marked. They keep their own name with no owner tag, so
they stay `Unproven` and their damage credits nobody. This is a ceiling of the
log, not a gap in the model: two players charming mobs of the same name produce
byte-identical lines.

Classification is monotonic. Evidence promotes; nothing demotes.

# docs/

Design rationale and process, kept out of the source tree.

Source files carry API documentation only — what a thing does and how to call
it. Why it is shaped that way lives here, because reasoning ages differently
from code and reading it should be a choice.

- `design/parsing.md` — framing, headers, the two-stage matcher, rule packs
- `design/sources.md` — the clock seam, tailing under Wine, replay
- `design/store.md` — the event store, ability rows, tag facets
- `design/encounters.md` — the damage graph, linked fights, entity kinds
- `design/context.md` — zone and session as derived spans
- `design/timeline.md` — state as queryable transitions, scrubbing, series
- `design/session.md` — encounters, rolling windows, TTK
- `ci.md` — branches, gates, release
- `../FOUNDATION.md` — platform decisions and their cost to reverse
- `../BACKLOG.md` — parked work and what unblocks it

# Changelog

## 2026-08-31

### Fight reset model

- Fights now close on a two-tier idle window keyed to whether anything has died: a fight with a kill in it closes 10s after going quiet (what keeps back-to-back pulls separate), a fight with zero kills yet waits 60s — quiet there means mezz, a fled mob, or a med break, not "over". A mezz landing on a fight's own mob also refreshes its clock directly. Measured on a real 5,864-fight log: kills preserved, resets down a quarter, premature splits (same target re-engaged within 2 minutes) down 80%.
- Copying an aggregate report now excludes closed reset fights from both the numbers and the ally lines, and the pasted title says "resets excluded". Single-fight copies and everything on screen are unchanged.

### Reliability

- One hostile log line can no longer take the app down or brick every view: parse batches are panic-isolated, and a poisoned ingest lock recovers instead of cascading. Verified with a 20,000-line fuzz probe.
- Settings, preferences, notification settings, and profiles now write atomically — a crash mid-save can no longer silently reset the app to first-launch.
- A second launch focuses the running window instead of starting a duplicate app (two tail workers, doubled overlays, clobbering preference caches).
- A failed first status fetch retries instead of leaving a permanently blank window; the loading state is visible.
- The notification-sound picker is parented to the main window — same behind-the-window Windows dialog fix the folder picker already had.

### Character planner

- Hand-set race and levels persist across launches. A typed level is flagged "set by you" (brass ring and dot) and the launch-time estimator only fills classes you haven't touched; "Estimate levels" is the explicit reset. Stored clobber-proof, outside the Settings round trip.

### UI

- Windows runs frameless with the toolbar as a real title bar: drag region, double-click maximize, min/max/close controls. Linux keeps native decorations (KWin/XWayland drops drag-region moves on undecorated windows).
- Overlay widgets re-assert always-on-top every 5 seconds on Windows, so a game that raises itself topmost can't permanently bury them.
- Default theme adopts a rounder look (large corner radii, pill controls), every hairline border is warmed toward the theme's accent instead of neutral grey, panels catch a faint accent rim light along the top edge, and scrollbars/selection/keyboard-focus rings are themed. Reduced-motion OS preference is respected everywhere.
- New render test matrix walks every module at five window sizes asserting layout and error invariants; it caught and fixed three null-payload crashes (Tradeskill, tracked-skills lists, and the Raiding tab's stuck "Loading…").

### Versioning

- Bumped to 0.8.0 for the public release carrying all of the above. Minor bump per the updater strict-greater-than convention.

## 2026-08-29

### Game state: party roster

- The Game State party list is now GroupTracker's current roster only. It previously listed every player ever proven anywhere in the log as a "party member" (3,800+ on a 245MB reference log); those remain visible as a known-players count.
- Explicit group lines now drive membership directly: join/leave lines add and remove members, "You have been removed" / joining a fresh group clears the whole roster, an accepted invite records the inviter, and a group-chat message proves the speaker's current membership. Explicit membership has no decay; it ends only via an exit line, a roster reset, or a 2-hour log gap (camping writes no disband line).
- Two new parse rules for lines the pack didn't cover: "X invites you to join a group." and "You notify X that you agree to join the group."

### Encounter involvement

- Every store encounter now carries `involves_you`: whether "You", your pet, or a current groupmate (or their pet) acted in it. Fights between strangers or mob-vs-mob are still parsed as encounters for clean backend data (~20% of encounters on the reference logs) but no longer surface as the overlay's current encounter, in Combat's fight lists, or in zone-visit aggregates. The Debug parsed-encounters view shows everything, flagged yours/other.

### Charm and allegiance

- One composition (`allegiance_at`) now decides ally-vs-enemy from permanent kind, charm state, and group belief together. Previously the group-tracking layer promoted a name to "player" before the charm flip, so a group-tracked charm pet read as an enemy — exactly the case the tracker existed for.
- A corroborated Quick Buff group-cast landing on the tracked charm name now re-affirms the charm: the game itself scopes those casts to valid group targets. This repairs the wrong-instance case where a second same-named mob hitting "You" cleared a still-loyal pet's charm; bounded to a recent clear so a long-ended charm stays ended.

### Versioning

- Bumped to 0.7.0 for the public release carrying the breadth-gated zone drop pool: an item is a zone-wide drop only when three or more distinct NPCs in the zone are attributed with it, so single-mob boss and quest drops stay with their actual dropper instead of smearing onto every engaged mob (a Sky trash mob read 272 known drops under 0.6.0's union pool; 44 now). Minor bump per the updater strict-greater-than convention.
- Bumped to 0.6.0 for the public release carrying the zone-wide drop pool from NPC loot attribution, hover Drop Watch bells on every item name, the charm false-break fixes (per-spell wear-off matching, no Lost stamp on charmed pets), root/fear caster-death resolution with the "maybe?" state, the new Ctrl square for the generic lose-control landing with enemy-cast attribution, and the first-launch folder pick repair (auto-resolve a picked Logs folder, validation before saving, parented dialog plus a paste-a-path fallback). Minor bump per the updater strict-greater-than convention.
- Bumped to 0.5.0 for the public release carrying the two-step updater (background install, restart when ready), the Drop Watch 30s post-fight loot grace, Sky class-unlock material ownership chips and resize-stable grids, navigate-to-an-NPC from its info page, the pathfinding fix for string-pulled legs cutting through walls, your own guild sends parsing into guild chat, and the backfill worker-count ceiling. Minor bump per the same updater strict-greater-than convention as every main release.
- Bumped to 0.4.0: fixes a crash in 0.3.0 where enabling any overlay widget panicked the whole app (click-through applied to a not-yet-realized window -- introduced by the same change that hid overlays from the alt-tab switcher), and folds loot to the base item ("+N" instances) on item pages, mob loot tables, encounter drops, and drop watch. Minor bump not patch, per the convention the testing channel's synthetic versions lean on.
- Bumped to 0.3.0 for the public release carrying the game-state truth pass, encounter involvement, the overlay alt-tab fix, and the new icon -- same reason as every main bump: the updater's strict greater-than comparison means a re-published 0.2.0 would never be offered to existing installs.
- Bumped the app to 0.2.0 -- the first hand-bumped version; public releases were stuck at 0.1.0, which the updater's strict greater-than comparison could never offer as an update.
- Testing builds now derive their synthetic version from the base major.minor (`0.2.<run>`) instead of a hardcoded `0.1.<run>`, so betas keep outranking the public release they're built past.
- Self-install now records the build's real version (including CI's synthetic testing version) instead of the crate version.

### Linux self-install

- A downloaded AppImage now installs itself as a real app on first launch: copied to `~/Applications/EQL-Oracle.AppImage`, with an applications-menu entry and icon, then hands off to that installed copy.
- Launching any old AppImage from Downloads afterwards also hands off to the installed copy instead of running its own stale code; a strictly newer download replaces the install, an older one never downgrades it. The downloaded file itself is never deleted.
- The in-app updater keeps working unchanged; it now always updates the one installed copy, so the stable filename never goes stale.

## 2026-08-25

### Class detection

- Fixed elimination narrowing restarting from the wrong evidence after a contradiction; it now poisons that visit's narrowing instead of guessing.
- Added a stricter, separate threshold for elimination-based class evidence versus unambiguous spell evidence (3 distinct visits vs 2).
- Excluded teleport spells from class evidence; they are castable as a group ritual and do not indicate the caster's own class.
- Excluded spells with a known item-click source from class evidence; a cast alone cannot distinguish a real class cast from an item effect.
- Wired AA (Alternate Advancement) grants into class detection using the existing per-AA class data; previously unused for this purpose.
- Excluded a pet's own casts from feeding its owner's class evidence, including cases where the pet-attribution merge itself is correct.
- Split same-class-set configurations into separate rows when the real time gap between visits exceeds 24 hours, instead of merging them into one continuous level range.

### Log parsing coverage

- Fixed two anchor-too-narrow bugs where a rule's pattern already handled a line's real second-person or non-plural form, but the anchor filtered it out before the pattern ever ran: `ds.damage` (flames/thorns damage) and `melee.blocked`.
- Fixed the same bug in `ability.activated`; its own doc previously claimed the shape was always third-person, which real data disproved.
- Added and extended rules across five batches covering: NPC dialogue and social lines, group and achievement messages, item procs, expedition cooldowns, self-damage, Tracking's directional readout, song pause/resume, camp countdown, and a range of previously unmatched mob-state and self-state flavor lines.
- Added buff/debuff polarity as a fallback classifier for ambiguous effect text that cannot be tied to one specific spell.

### Spellbook Builder

- Added reading, editing, and saving of spell loadouts directly from the character's saved game config file.
- Changed Found Spellbooks to a full list instead of a dropdown, loaded all at once.
- Added save-as-new-file and integrated suggest-fill into real loadouts.
- Made spellbook sections collapsible, collapsed by default.
- Fixed spell-id resolution re-parsing the full spell catalog file on every call; batched into one call per loadout fill.

### Release infrastructure

- Split release channels: pushes to `testing` publish a prerelease build under the `testing` tag; pushes to `main` publish under `latest`.
- Added in-app auto-update with a per-channel signed manifest and an install prompt.

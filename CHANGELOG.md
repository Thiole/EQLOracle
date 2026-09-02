# Changelog

## 2026-09-02 (0.14.0)

### Routing

- Every zone now uses the EQEmu navmesh, collision mesh, and water volume for pathfinding. The water file is fetched on first open like the other two. A land zone with a lake or river bridges mesh islands only through the water; leaving the mesh is only ever a swim. Lake Rathetear: 129 mesh islands down to 10.
- Swim routes ride the navmesh wherever it exists — swim nodes only bridge the shafts and gaps the mesh can't carry (hops through a swim node cost 3x their length).
- Kedge Keep: routes follow the tunnel, the chamber, the shafts, doorways, and door panels by rules that apply everywhere; all 70 wiki spawn points route. The routing walls always come from the game's own map file, never the viewing pack.
- A spawn point with no z: a map-pack label naming that mob nearby (Brewall labels Kedge's named mobs in 3D) is taken as the destination. Otherwise every candidate floor under the spot is listed as ambiguous in the Maps panel, with the route drawn to the top one and a pick re-routing.
- Zone routes only use a teleport whose spell the log shows you know.
- The wiki's newer "X: / Y: / Z:" location shape parses, so a re-scrape carries the z.

### Combat

- An evac leaves the fight behind: a fight that ended at or before the last zone line is no longer the current encounter (meter and duration clear).
- Same-named mobs: a fight where a slain name is hit again gets 6s of grace on top of the 10s resolved idle window, so a raid moving between duplicate-named mobs keeps one fight.

### Maps view

- The map fills the window instead of a fixed 520px card.
- Drop Watch drop lists wrap instead of truncating off the right edge.

## 2026-09-01 (0.13.0)

### Session overlay

- New "Session" overlay widget: AA, levels, and plat per hour as three stat columns, with the next-level ETA under levels/hr and a mote strip underneath — total, rate, and a per-tier circle with counts (hover for the tier name). Same numbers as the Overview tab, compact overlay form. Rates honestly show "--" until the session is a minute old.
- AA/hour counts *earned* ability points — the "You have gained N ability point(s)!" payout line, which was parsed but never used anywhere until now — not points spent on ranks.

### Overlay button

- The top-bar button now reads "Overlay: enabled" / "Overlay: disabled" with the good/deny colors — state in words as well as color, readable colorblind.

## 2026-09-01 (0.12.0)

### Overlay (Windows) — the actual fix

- Enabling an overlay on Windows deadlocked the app's main thread before the overlay window ever existed: the enable command ran inside the main window's own WebView2 event handler, and creating a second WebView2 from there is a documented reentrancy deadlock. The command now runs off the main thread — the overlay window gets created the way it always should have. This was the "no overlay ever appears" report; every earlier style-level fix was aimed at a window that never got built.
- Debug → Overlay diagnostics survive that class of bug now: window readback runs with a 3-second deadline and reports "main thread blocked" instead of hanging the panel, and the panel carries an ordered enable trace showing exactly which stage a wedged creation reached.

## 2026-09-01 (0.11.0)

### Clipboard

- Every copy button (combat report, /tell, diagnostics JSON) now writes the OS clipboard from the backend — the webview's own clipboard API fails silently on Linux, leaving stale clipboard content to paste. A failed copy now says "clipboard copy FAILED" instead of pretending.

### Overlay diagnostics

- The Debug → Overlay panel shows the error when the diagnostics call itself fails, and "copy JSON" copies that error text — a broken call is now a pasteable report instead of a greyed-out button.

## 2026-09-01 (0.10.0)

### Tradeskills

- Current tradeskill levels, parsed from the log's own skill-up lines — the 9 crafting skills plus Fishing, Forage, and Alcohol Tolerance. Unknown shows as "—", never a fake 0, since the log only states a level when a skill-up fires.
- Overview redesigned around them: a skill-level table and a "recently crafted" list (last 15 successful combines with item icons) replace the aggregate craft table — its per-item stats already live on each recipe in the skill tabs.
- With your level known, each recipe list marks combines at or below trivial as "no skill-up".

### Overlay (Windows)

- New Debug → Overlay tab: OS-level readback of every open overlay window — style flags, layered alpha, visibility, cloaking, position vs monitors — with one-click copy, for the "overlay not showing" reports. If yours doesn't show: enable the widget, open this tab, copy, paste in the report.
- Overlay widgets actually stay out of alt-tab on Windows now — the style bit was being silently reverted the moment the window showed.

### DPS meter

- Labeled columns with wider separation.

## 2026-09-01

### Epic Quests

- New Endgame tab: an item-first farm list for all 15 class epics, so the materials can be hunted before the Epic Quests Era opens. Scraped from the wiki's quest pages (loot-drop targets plus forage/pickpocket; NPC-handed intermediates excluded — they need the era's own quest NPCs). Every item carries live ownership status from loot history and the latest inventory dump, with per-item Drop Watch bells, a page-wide "notify for all epic items", and a concise per-class "+ all" button. Berserker honestly lists nothing: its epic is quest-triggered trial spawns end to end.

### Spell performance

- The app now keeps a rolling per-spell landing average and flags a spell landing well under its own usual — partial resists show up within a handful of casts instead of three fights late. The baseline is invocation-matched (your last 5 zones under the same stance), so switching to a lower-output invocation is never a false alarm; the session norm is the fallback until same-stance history accrues.
- The DPS meter shows the struggling spell with its "% of usual" and the proven hitters still holding their norm; the Skill Tracker shows the same signal as "<spell> — partial resist N%". Both appear only while something is actually struggling.

### Kill model

- A kill now means the actual target died: an anchor's kill credit requires the target's own name in the slain list, so a boss's adds or pets dying no longer count the boss as killed.
- A pet dying is never a kill — anywhere. Pet deaths no longer close fights as kills or pad kill counts.
- A fight that resets and re-engages merges back into one encounter: the corpse fight reparents into its keeper, so no more phantom zero-length "reset" fights or double-counted deaths.

### DPS meter

- Rows now show damage share, total, DPS, and time active — where each ally's clock starts at their own first action, so a late joiner's DPS is honest instead of pull-diluted.
- The meter tracks the whole engagement: mobs that join late fold into the same fight instead of hijacking the display.

### Maps & GPS

- Pathfinding now runs on the server's own navigation meshes: routes thread corridors around geometry instead of drawing straight chords through walls, and walk legs hug the ground. The map view itself stays on the game's own map files.
- Teleporter pads and portal rings are location hops in the route graph — Plane of Sky routes go island to island through the portals, drawn dashed.
- The Maps left panel is now an NPC browser: search, con-colored names with levels, drops on select, and "set path here" straight from a row.

### Sky Quests

- One-click "notify for all uncompleted quests" on both Sky tabs, demand-aware across multi-quest materials (an item two quests need stays tracked until you own enough copies), de-duplicated by construction.
- Sky tabs update live when loot lands or a fresh inventory dump appears; the Drop Watch pickup prompt no longer misses due to clock skew.

### Fixes & plumbing

- Pets attacking enemies no longer read as incoming damage on the meter.
- Tail poll dropped from 250ms to 100ms — the overlay reads what just happened.
- Technical reference docs: docs/architecture.md (parser/ingest/store) and generated docs/api (commands and packs).

### Versioning

- Bumped to 0.9.0. Minor bump per the updater strict-greater-than convention.

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

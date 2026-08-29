# Changelog

## 2026-08-29

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

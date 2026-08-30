# Windows failure modes — overlay, input, visibility, maps

Every known way the app can misbehave on Windows, from the 2026-08-30
audit. Status legend: **fixed(commit)** — code landed; **by-design** —
intended, may need UX; **external** — environment, needs user action;
**open** — plausible, needs field data.

## Overlay not visible at all

1. **Layered window never rendered** — tao adds WS_EX_LAYERED for
   click-through but never calls SetLayeredWindowAttributes; MSDN says
   such a window "will not become visible". Every Windows overlay since
   the feature shipped. **fixed(9bd7237)**, style triple self-asserted
   in d355352.
2. **Saved position off any live monitor** — unplugged second display,
   resolution change. Restore now validates against current monitors
   (32px margin). **fixed(9bd7237)**.
3. **Exclusive fullscreen game** — the game owns the display; no
   process's window can appear. **external**: use borderless windowed,
   or leave Windows Fullscreen Optimizations on for eqgame.exe. No
   non-invasive fix exists (drawing over FSE requires injecting the
   game process — ToS line we don't cross).
4. **Game sets itself topmost in borderless** — both in the topmost
   band, last-raised wins. EQ-era clients don't do this. **open**;
   known escape hatch: re-assert always_on_top on a timer.
5. **WebView2 transparent-window rendering quirk** — overlay content
   could render a white/black box instead of transparent, or nothing,
   on machines with GPU acceleration off. **open**, needs field data;
   distinct from (1) because the window frame would still hit-test.

## Overlay visible but not over the game

6. Same as (3)/(4) — layering only; covered above.
7. **Multiple monitors, game on the other one** — overlay opens at OS
   default position on the primary. **by-design**; unlock, drag, lock.

## Clicks blocked ("cant click anything in the app")

8. **Click-through failed to stick** — set_ignore_cursor_events result
   was discarded; an overlay opens at the OS default position ON TOP of
   the main window, and without WS_EX_TRANSPARENT it eats every click
   under it. Invisible pre-9bd7237, making it a mystery freeze.
   **fixed(d355352)**: TRANSPARENT|LAYERED|NOACTIVATE asserted directly
   on the HWND after both click-through sites.
9. **Widget left unlocked** — unlocked mode intentionally intercepts
   clicks (it must, to be draggable) and gets decorations. A user who
   unlocks and forgets has a click-eating window parked wherever they
   left it. **by-design**, but worth UX: an unlocked widget should look
   loudly unlocked; consider auto-relock after idle.
10. **Parented modal dialog left open** — the folder picker is parented
    to the main window (the fix for it opening behind); a parented
    dialog DISABLES its owner while open, so a dialog lost behind other
    windows reads as "app frozen". **open**; only reachable from
    first-launch/settings folder pick.

## App not usable / blank

11. **WebView2 runtime missing or broken** — blank main window at
    startup. Installer uses Tauri's default downloadBootstrapper, so
    only offline installs or corporate-locked machines hit it.
    **external**: install WebView2 evergreen.
12. **SmartScreen/AV quarantine** — unsigned binaries; Defender may
    block or sandbox. **external** until code signing exists.
13. **Backfill saturation** — the UI stays live during backfill by
    design (worker thread, heartbeat-throttled emits); if a report says
    frozen-during-first-minute, suspect (8) first, this second.
    **open**.

## Maps not displaying / loading

14. **Non-UTF8 map file silently skipped** — a Latin-1 label or editor
    stray byte dropped the entire layer with no error anywhere.
    **fixed(b4ebabf)**: lossy read.
15. **No WebGL** — Remote Desktop, VMs, blocklisted GPUs,
    acceleration-disabled WebView2: the renderer constructor threw and
    left a silent blank canvas. **fixed(b4ebabf)**: visible message.
16. **Wrong install folder** — maps live in <base>/maps; a mis-picked
    folder had no maps dir. Largely prevented by the first-launch
    validate-and-repair (c679a27). **fixed** upstream.
17. **.TXT vs .txt** — loader matches ".txt" case-sensitively on the
    filename string; the game ships lowercase and NTFS lookups are
    case-insensitive for the directory scan itself, so only a
    hand-renamed file hits this. **open**, trivial if ever reported.

## Updater

18. **Same-version republish never offered** — the updater's strict
    greater-than comparison; the reason every main ship bumps MINOR.
    **by-design** (documented in CHANGELOG Versioning).
19. **Windows install is exit-and-reinstall** — the plugin's installer
    path exits the process; "the app closed" during update is normal.
    **by-design**.

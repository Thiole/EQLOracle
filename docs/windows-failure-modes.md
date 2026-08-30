# Windows failure modes — full-app audit

Every way the app can misbehave for a Windows user, re-derived from
source 2026-08-30 (second full pass; supersedes the first). Status:
**fixed(commit)** — code landed; **by-design** — intended, may want UX;
**external** — environment, needs user action; **open** — plausible,
needs field data.

## Overlay not visible at all

1. **Layered window never rendered** — tao 0.35.3's click-through sets
   WS_EX_LAYERED via its style rebuild but never calls
   SetLayeredWindowAttributes anywhere (re-confirmed against vendored
   source this pass); MSDN: such a window "will not become visible".
   **fixed(9bd7237)**, self-asserted triple in d355352. Re-lock path
   verified this pass: tao rebuilds the whole EX style word from its
   flags on every window-flag change, and whichever order the queued
   ignore/decorations/ensure-layered calls land, the final style keeps
   LAYERED and the attributes call runs against a layered window.
2. **Saved position off any live monitor** — restore validates against
   current monitors, 32px margin. **fixed(9bd7237)**. Residual:
   mixed-DPI monitor sets don't share one logical coordinate space
   (each monitor's logical rect is its physical rect / its own scale),
   so on e.g. a 100%+150% pair the check can mis-accept and the
   restore can land on the wrong monitor. **open**, rare; worst case is
   a wrong-monitor open, unlock-drag-lock fixes it.
3. **Exclusive fullscreen game** — the game owns the display.
   **external**: borderless windowed, or leave Fullscreen Optimizations
   on (an FSO "fullscreen" is DWM-composited and overlays fine).
   Drawing over true FSE requires injecting the game — ToS line we
   don't cross.
4. **Game raises itself topmost in borderless** — both in the topmost
   band, last raise wins. EQ-era clients don't. **open**; escape hatch
   if ever reported: re-assert always_on_top on a timer.
5. **WebView2 transparency quirk** — GPU acceleration off / RDP can
   render a transparent window as a solid or empty box. **open**,
   distinct from (1): the frame still hit-tests.

## Clicks blocked / app won't respond to input

6. **Click-through failed to stick** — an overlay opens at the OS
   default position on top of the main window; without WS_EX_TRANSPARENT
   it eats every click. **fixed(d355352)** — triple asserted on the HWND
   after both click-through sites.
7. **Widget left unlocked** — unlocked mode intercepts clicks by design
   (it must, to drag). **by-design**; UX debt: unlocked should look
   loudly unlocked, or auto-relock after idle.
8. **Parented dialog disables the main window** — both pickers parent to
   main (the fix for opening behind); a parented dialog lost behind
   other windows reads as "app frozen" until found. **open**, narrow.
9. **Unparented file dialog opens behind the app** — was fixed for the
   folder picker in c679a27 but the notification-sound picker shipped
   without the parent, same "button does nothing" failure.
   **fixed(this pass)**.

## App dead / blank / bricked

10. **One bad log line bricked every view** — a panic inside parse while
    holding the ingest mutex poisoned it; every later command's
    `lock().unwrap()` then panicked forever: window open, all data dead,
    and with `panic = "abort"` the first such panic just killed the
    process outright. **fixed(this pass)**: `panic = "unwind"`,
    catch_unwind around backfill batches and the live seam,
    `lock_recover()` (poison-recovering lock) at every lock site — a
    bad batch is dropped with a stderr note and the app stays live; the
    next full replay rebuilds clean.
11. **First get_status rejection left a permanently blank window** — the
    mount-time call had no retry and the null branch rendered literally
    nothing; one IPC/webview startup flake = empty window forever.
    **fixed(this pass)**: retry loop + visible "Loading…" state.
12. **Second launch runs a whole second app** — two tail workers on one
    log, duplicate overlay widgets ("two DPS meters with different
    numbers"), two in-memory preference caches whose last save wins
    wholesale. **fixed(this pass)**: single-instance plugin; a second
    launch focuses the running window.
13. **Crash mid-save silently reset the user** — config/preferences/
    notification-settings/profiles all wrote with bare fs::write, and
    every loader treats unreadable as "never saved": a power cut during
    a save could land the user back on first-launch with positions and
    tracked lists gone. **fixed(this pass)**: temp-file + rename
    (atomic on NTFS) for all four stores.
14. **WebView2 runtime missing/broken** — blank window at startup;
    installer uses downloadBootstrapper, so offline/locked-down machines
    only. **external**.
15. **WebView2 cache torn by a hard kill** — Linux has the
    unclean-exit sentinel + cache clear (real incident there); Windows
    has no equivalent for EBWebView. **open** — port the sentinel if a
    Windows white-window-after-kill report appears.
16. **SmartScreen/AV on unsigned binaries** — **external** until code
    signing.
17. **Rule pack fails to load** — worker exits after one console-only
    `parse-error` event; UI looks fine, parses nothing. Near-impossible
    (pack is compile-time embedded) but silent. **open**, cheap UX fix
    if ever seen.

## Live feed stale or wrong file

18. **Stale file size from GetFileAttributesEx** — the tail gates reads
    on `fs::metadata` length, and NTFS updates directory metadata lazily
    for a file another process holds open; symptom would be live lines
    lagging while backfill was fine. Same staleness can delay
    `newest_log_in`'s mtime-based character-switch pickup. **open** —
    needs field data; the robust fix is reading from the held handle
    regardless of the metadata size (or GetFileInformationByHandle).
19. **No eqlog file because /log was never enabled in-game** — app sits
    at "idle" forever with no hint. **open** UX gap: a "no log file
    found — type /log in game" callout when configured with no target.
20. **Hand-renamed logs with different case** (`.TXT`, `EQLOG_`) — the
    directory scan matches case-sensitively; the game itself writes
    lowercase. **open**, trivial if reported.
21. **Log dir under OneDrive/redirected Documents/network share** —
    sync locks and stale metadata outside our control. **external**.
22. **Inventory dump read mid-write** — partial parse of a dump the
    game is still writing; next dump re-reads. **by-design** tolerance.

## Maps

23. **Non-UTF8 map file dropped a whole layer silently** —
    **fixed(b4ebabf)**, lossy read.
24. **No WebGL (RDP, VMs, blocklisted GPU)** — was a silent blank
    canvas. **fixed(b4ebabf)**, visible message.
25. **Wrong install folder picked** — prevented by validate-and-repair
    (c679a27): Logs-dir picks resolve to parent, unrecognizable picks
    error without saving. **fixed**.

## Updater / notifications

26. **Same-version republish never offered** — strict greater-than
    compare; the reason main ships bump MINOR. **by-design**.
27. **Windows install is exit-and-reinstall** — the plugin's installer
    path exits the process; "the app closed" during update is normal.
    **by-design**.
28. **Notification sounds configurable but playback not wired** —
    Settings discloses it for volume, but per-kind toggles and the
    custom-sound picker imply working audio. **by-design** (roadmap);
    when wired, note WebView2 blocks autoplay until a window has seen a
    user gesture — click-through overlay windows never see one, so
    sounds must play from the main window.

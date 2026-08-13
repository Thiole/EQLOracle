# Foundation decisions

Things that are cheap now and expensive later. Each entry says what was decided
and, more usefully, what it costs to change.

---

## 1. Native builds per platform. No Windows-under-Wine.

**Ship targets: Windows (WebView2) and Linux (WebKitGTK), both native. macOS
compiles in CI but is unsupported.**

Tauri renders through the system webview: WebView2 on Windows, WebKitGTK on
Linux, WKWebView on macOS. Running the *Windows* build inside a Wine prefix
means running WebView2 inside Wine, and Microsoft has marked Wine/Proton support
low-priority with no near-term plan. The observed failure mode is not a crash —
it is a window that opens blank, or opens after a manual Edge install under
Proton GE. That is unshippable.

We do not need the prefix. The app reads a text file. A native Linux binary
reads `~/.wine/drive_c/.../EverQuest Legends/Logs/eqlog_*.txt` directly, so
"find the log inside a prefix" is path discovery, not packaging. EQ Legends
Companion lives in the bottle because it is Windows-only; that is its
limitation, not a design to copy.

**Cost to reverse:** low, and deliberately so — see §4. If Wine's WebView2 story
ever improves, nothing in the codebase assumes it hasn't.

## 2. WebKitGTK is the frontend baseline, not Chromium.

Because Linux is a first-class target, the *oldest* WebKitGTK we support defines
the feature floor. Ubuntu 22.04 is that floor.

The failure mode this prevents: develop on Chromium, test on Chromium, ship,
and discover on Linux that a layout silently breaks. Tauri's own docs note that
WebKitGTK versions vary enough across distros that they cannot compile accurate
support tables — so we do not guess, we test on the engine.

**In CI: Playwright's WebKit project is blocking. Chromium is advisory.**

House rules that follow, and also just make a combat meter more readable:

- No `backdrop-filter`, no blur. Expensive and inconsistent across engines.
- No continuous CSS animation. A number that moves is a number you can't read.
- No transparency we don't need.
- Flat opaque panels, monospace numeric columns, fixed layout.
- A frame-time budget asserted in the visual tests, not eyeballed.

**Cost to reverse:** high. Retrofitting a WebKit floor onto a Chromium-developed
UI means auditing every component.

## 3. Time is injected. Nothing calls `Instant::now()`.

See `crates/source/src/clock.rs`. Everything downstream takes a `&dyn Clock`.

This exists before any feature because every interesting computation is a
function of time — DPS windows, encounter boundaries, "is this fight over" — and
if any of them read the wall clock directly then three things become impossible
simultaneously: deterministic snapshot tests, replay, and fast history backfill.

Enforced by a grep in CI. One line, and it makes the rule real rather than
aspirational.

**Cost to reverse:** very high. It means touching every consumer.

## 4. The overlay is a negotiated capability, not an assumption.

v1 is a docked panel. Overlay comes later — but the seam goes in now.

Wayland cannot do what a floating overlay needs. `always_on_top` does not work
there (tao#1134); click-through does not work there either — one shipping
overlay tool's README says plainly that everything works on Wayland *except*
click-through. This is upstream and not ours to fix.

So window role is a runtime-detected capability with a declared fallback:

```
Docked      always available, every platform      <- v1
Floating    X11 / Windows / macOS
ClickThru   X11 / Windows / macOS
```

Detect at startup, degrade to `Docked`, and tell the user why in plain language
("floating overlays need X11; you're on Wayland"). Features must ask the
capability, never assume the window.

**Cost to reverse:** high if skipped. Building features that assume a floating
window exists *is* the refactor.

## 5. Polling, not filesystem watching.

`crates/source/src/tail.rs`. inotify and `ReadDirectoryChangesW` are a
translation of a translation under Wine, and they drop and coalesce events. A
meter that silently stops updating mid-raid is worse than one costing 0.1% of a
core. 250 ms polling is four `stat` calls a second and behaves identically on
Windows, Linux, Wine and a network share.

**Cost to reverse:** low. It's behind `Tail`.

## 6. Replay is the testing strategy.

`crates/source/src/replay.rs`. A recorded log drives a `VirtualClock`, so every
UI assertion becomes "feed fixture, advance to *t*, snapshot" — no game, no
waiting, no flake, identical on a laptop and in CI.

This is what "externalize the testing" means concretely. A bug report becomes a
log excerpt plus a timestamp, which is a reproducible test case rather than a
story. There is already 148 MB of fixture material.

Three speeds (`Instant`, `Realtime`, `Scaled(n)`) all drive the clock from the
log's own timestamps, and a test asserts they produce byte-identical output.
`Instant` doubles as the startup history-backfill path.

---

# Test tiers

| | what | where | speed |
|---|---|---|---|
| L1 | core Rust, pure functions | any runner | seconds |
| L2 | rule pack lint + coverage gate | any runner | seconds |
| L3 | frontend vs. mock IPC, **WebKit blocking** | any runner | ~1 min |
| L4 | real webview e2e, smoke only | ubuntu-22.04 + windows | minutes |
| L5 | Wine *filesystem* behaviour | ubuntu + wine | seconds |

L5 deserves a note: it does **not** run the app under Wine. It checks the only
thing that genuinely differs — reading a file a Wine process holds open and
appends to. That is the real Wine risk once WebView2 is off the table.

L3 is where most work happens, and it needs the frontend to be runnable in a
plain browser against a mock backend. Keep that true. The moment the UI can only
run inside Tauri, testing gets slow and people stop doing it.

---

# Build order

1. ~~Parser core~~ — done
2. ~~Clock + Tail + Replay~~ — done
3. **IPC contract** — one schema, TS types generated from Rust, golden fixtures
   asserted from both sides. This is where Tauri apps rot; freeze it early.
4. **Mock IPC harness** — the frontend boots in a browser with no Rust at all.
5. **Capability detection + window roles** — §4.
6. CI matrix — `.github/workflows/ci.yml`
7. Then features.

Step 4 before step 7 is the one that matters. It's what makes every feature
after it cheap to test.

Ship the app version *and the rule pack hash* in the UI from day one, so a bug
report says which parser produced the number.

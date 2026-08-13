# Branches, gates, and release

## Promotion

```
feature/*  ──▶  main  ──▶  beta  ──▶  tag v*
   gate 1      gate 1     gate 2      gate 3
```

Nothing skips a gate. A tag that is not an ancestor of `main` fails the release
guard before anything builds.

## Gate 1 — verify (`1-verify.yml`)

Every push and PR on any branch. Must stay fast enough that nobody routes around
it; if it creeps past a few minutes, move work to nightly rather than loosening
it.

- `fmt`, `clippy -D warnings`, full test suite
- **fuzz smoke** — 60s per target, every target, every PR
- **rule pack lint** — every rule's declared examples run through the whole
  engine; shadowing fails the build
- **coverage gate** — parser coverage against a committed fixture, `--min-rate`
- **grep gates** — no `Instant::now()` outside the clock module, no `#[cfg(test)]`
  in `src/`

The two grep gates exist because both rules are invisible to the compiler and
both are load-bearing. They cost one second and they are the reason the rules
stay true.

## Gate 2 — beta (`2-beta.yml`)

Push to `beta`. This is where rendering and interaction are proven.

Three jobs:

- **render** — visual baselines across `{webkit, chromium} x 3 viewports x 3
  scale factors`. Runs against the mock IPC harness in a plain browser.
- **interaction** — hit testing, click permutations, window identity, field
  values. Same harness.
- **shell** — the real webview via `tauri-driver`, on ubuntu-22.04 and
  windows-latest. Reserved for what the harness cannot see: window count,
  native chrome, real IPC, OS cursor position.
- **wine-filesystem** — reading a file a Wine process holds open. Not the app
  under Wine; we ship native.

**WebKit is blocking, Chromium is advisory.** Linux ships WebKitGTK, so that is
the engine that decides. Chromium runs to catch regressions early but never
gates.

Baselines are per-engine. A shared baseline is either too loose to catch layout
drift or permanently red.

## Gate 3 — release (`3-release.yml`)

Tags matching `v*`. Builds ubuntu-22.04 (oldest supported WebKitGTK),
windows-latest, and macos-14. macOS is **built but never published** — it is
unsupported, and we want to learn it stopped compiling now rather than at
porting time.

## Nightly (`4-nightly.yml`)

30 minutes of fuzzing per target with a persisted corpus, so coverage compounds
instead of restarting every run. Too slow to gate a PR; a gate people wait 20
minutes for is a gate they learn to work around.

## Required setup

Branch protection on `main` and `beta`: require gate 1, and gate 2 for `beta`.

Secrets: `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PASSWORD`.

Committed fixtures: `fixtures/reference-slice.log`. Keep it small enough to clone
comfortably; the full 148MB log is not a repo artifact.

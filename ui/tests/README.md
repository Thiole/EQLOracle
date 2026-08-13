# UI test framework

Scaffolding only. Every spec below is `test.fixme` — it declares the contract and
fails loudly as unimplemented rather than passing vacuously. Fill them in as the
UI exists; do not delete them to make the suite green.

## Why these specific tests

Webview apps fail in a narrow, repeatable set of ways that unit tests never see:

| failure | spec |
|---|---|
| Two windows open, or one opens twice | `interaction/window-identity` |
| Panels overlap / stack wrongly | `render/z-order` |
| Click lands on the wrong element | `interaction/hit-testing` |
| Cursor offset from its hit target | `interaction/hit-testing` |
| Layout breaks at scale or viewport | `render/layout` |
| State corrupts on unusual click order | `interaction/permutation` |
| Field shows wrong/stale value | `interaction/fields` |

## Layers

- `tests/render/` and `tests/interaction/` run against the **mock IPC harness**
  in a plain browser. No Rust, no Tauri, no game. Fast, and where most work goes.
- `tests/shell/` runs the real webview through `tauri-driver`. Slow. Reserved for
  what the harness genuinely cannot see: window count, native chrome, real IPC,
  OS cursor position.

## Determinism

Every test drives the UI from a **replay fixture** at a fixed virtual timestamp.
No wall clock, no live log, no waiting. See `docs/design/sources.md`.

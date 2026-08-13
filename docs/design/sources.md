# Sources and time — design notes

Rationale for `eqlp-source`.

## Clock — why it exists before any feature

Every interesting computation is a function of time: DPS windows, encounter
boundaries, "is this fight over". If any of it calls `Instant::now()` directly,
three things become impossible at once:

- **Deterministic tests.** A DPS number computed against the wall clock differs
  on every run and machine. It cannot be snapshotted.
- **Replay.** Feeding a recorded log through the app to reproduce a bug needs
  time to advance at the log's pace, not the wall's.
- **Fast-forward.** Rebuilding session state from 12 days of history must not
  take 12 days.

Retrofitting means touching every consumer. So nothing downstream calls
`Instant::now()`; it takes a `&dyn Clock`. Enforced by a grep gate in CI, with a
`// clock-exempt: <why>` marker for the rare legitimate case (benchmarks, where
wall time is the point).

`VirtualClock::set_at_least` never moves backwards: a log whose timestamps go
backwards (clock change, DST, hand-edited file) must not make elapsed time
negative downstream, where it would divide by a negative window.

Milliseconds, not micros — the log's own resolution is one second, and finer
units would imply precision we do not have.

## Tail — why polling, not filesystem notifications

The obvious implementation watches with inotify or `ReadDirectoryChangesW`. Do
not. Under Wine — half the target audience — the notification layer is a
translation of a translation and drops or coalesces events. A meter that silently
stops updating mid-raid is worse than one costing 0.1% of a core.

Polling a file's length is one `stat` per interval. At 250ms that is four
syscalls a second and behaves identically on Windows, Linux, Wine and a network
share.

### File sharing

On Windows the game holds the log open for writing. Rust's `File::open` requests
`FILE_SHARE_READ | WRITE | DELETE`, so a plain open works and does not interfere.
We never open for write, never lock, never truncate. Strict reader — which is
also what makes it obviously safe to run alongside the client.

### What can happen to a file under us

- **Grew** — normal. Read from the last offset.
- **Shrank** — deleted or truncated. Reset to 0 rather than seeking past the end
  forever.
- **Replaced** — same path, different file. Detected by identity, not size; a
  replacement can happen to be the same length.
- **Vanished** — uninstalled, unmounted, prefix stopped. Not an error; keep
  polling so it recovers.
- **Torn write** — a poll landing mid-line. Handled by never emitting a line
  without its terminating newline.

### File identity

`ino` on Unix, creation time as fallback, then "assume same". The fallback order
matters: guessing "replaced" re-reads the whole log and double-counts every
event, so ambiguity resolves towards "same file". Truncation is caught separately
by the length check. Some Wine filesystem drivers report a zero index, which is
why the fallback exists at all.

### Which log

A prefix accumulates one `eqlog_<Character>_<Server>.txt` per character ever
played, so "newest file in the folder" is the only reliable signal. Poll it
periodically — switching characters switches files, and a meter stuck on the
character you logged out of is a silent failure.

## Replay — the testing keystone

A UI test that needs the game running is a test nobody runs. Replay makes every
UI assertion "feed fixture, advance to t, snapshot" — no game, no waiting, no
flake, identical on a laptop and in CI. A bug report becomes a log excerpt plus a
timestamp: a reproducible case rather than a story.

Three speeds (`Instant`, `Realtime`, `Scaled(n)`), all driving the clock from the
log's own timestamps, so a ten-second DPS window during replay covers exactly the
events it would have covered live. A test asserts all three produce identical
output. `Instant` doubles as startup history backfill.

The caller owns sleeping, so `Replay` stays usable from an async runtime, a
thread, or a test that never sleeps.

**Undated lines are emitted, not dropped** — dropping them would make replay
lossy and therefore useless as a reproduction. An undated line is a continuation
of the event above it and carries that event's time, so it stays on the same side
of a `run_until` boundary.

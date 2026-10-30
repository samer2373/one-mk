# M1a-3 evidence: security/idle resolved, lifecycle STOP retained

All sessions are isolated headless GNOME Shell 50.1, private session/system
buses, private HOME/XDG, no Xwayland. No owner desktop, one-mk process or
extension directory was used. Reports date observations in UTC; the work
crossed midnight October 30/31, 2026 in Asia/Amman.

## Final-code evidence

- `security-verified.json` and its complete Shell log: name squatter rejected
  before spawning; bypassing that extension guard in the test still makes the
  renderer refuse the foreign unique-name/PID **before sending Hello**.
  Removed FrameReady is absent from introspection. A deliberately retained
  previous toplevel is re-hidden after an attempted show, and its client is
  terminated/reaped after the replacement deadline. No new baseline warnings.
- `idle-final.json`, full Shell log and `idle-final-renderer.strace.gz`: all
  renderer threads traced with strace `-D -f -ttt -T`. Three-second settled
  overview/coverage windows have zero timer returns, zero renderer/Shell CPU
  ticks, and zero extension watchdog/restart/coverage source IDs. Blocking
  poll returns are **0.333293/s paused and 0.666572/s covered**, not zero total
  wakeups. Compositor-origin Wayland traffic still needs responses; this is
  not a claim of zero kernel scheduler wakeups or zero work in the entire Shell.
- `qualified-1.json` and full GDB/Shell log: final full lifecycle matrix reaches
  **74 clean cycles, then fails starting cycle 75**. Hello repeatedly returns
  `org.freedesktop.DBus.Error.UnknownMethod: No such method “Hello”`. The bus
  owner is still `:1.0`, the private Shell is alive (state S), and the observed
  cgroup has zero OOM/oom_kill events. No Shell exit/core was observed before
  cleanup. This is a bridge/export lifecycle failure, **not evidence that the
  old Shell-disconnect root cause has been found**. The five-run batch stops
  at this failure; there are not five clean final-code runs.

The fields named `*_shots` in this last report are now compositor-timing
objects, not screenshots. `shell_events` records GLib monotonic microseconds
at actual monitor/lock notifications in the private test harness extension;
`damage` records the renderer actor's damage signals on the compositor thread.
No screenshot IPC is used in this final matrix. Layout notification to first
new-window damage: **37.069 ms unplug / 30.588 ms replug**. Actual unlock
notification to damage: **4092.193 ms**. The log shows the existing shared
restart budget's third **4000 ms backoff**, followed by about 51 ms from spawn
to first damage. Damage does not establish physical scanout or rule out a
missed paint; the partial matrix does not qualify the whole lifecycle.

## Prior attempts, not final acceptance

- `idle-event`: rejected first implementation; repeatedly removing/adding the
  GLib FD source self-woke. `idle-persistent`: corrected persistent FD source.
  The huge rejected trace is not retained; its failed report/log are retained.
- `stability-debug-3`: pre-hardening attempt fails after 92 completed cycles;
  private Shell remains alive. Its original Hello error lacked the later
  GError diagnostics. Do not attribute it to an OOM or Shell crash.
- `diagnostic-1` through `diagnostic-5`: five consecutive clean 100-cycle runs
  **before** the bridge security changes. They do not meet the final-code
  requirement. Older reports refer to generated lab screenshots; those bulky
  files are deliberately not retained or used for final timing claims.
- `security-negative`: first negative check exposed double-unexport and
  expected-refusal critical logging; both corrected. `security-final`: all
  security assertions succeeded, but sending delete to the old DESKTOP window
  caused a new Mutter bad-serial warning; rejected without adding an allowlist.
  The final implementation instead retains hide enforcement until destruction
  and terminates the client at the deadline. `security-pinned`: fixture API
  typo; corrected using the installed introspected closures2 API.
- `final-1`: test collector incorrectly accessed a disabled bridge during
  lock; corrected to read the extension's retained recovery object. Separate
  from the genuine `qualified-1` UnknownMethod failure.

## Validation / reproduction

`bash docs/spikes/gnome/qualify-m1a3.sh` builds offline with `-j 2`, then runs
security, idle tracing and up to five full repeats against the unchanged
committed M1a-2 baseline. First failure stops the sequence. It never falls back
to the live desktop. GDB is private-session-only; host core handling is not
changed, and automatic host core dumps are disabled for the child Shell.

`test-m1a3.log`: clean-environment Linux workspace tests, risk feature enabled,
**1052 passed, 0 failed, 13 ignored** (ignored gates are not claimed passed).
`clippy-security.log`: Linux workspace/all-target feature-enabled clippy.
`validation-m1a3-corrected.log`: default Linux workspace/all-target clippy and
Windows MSVC / Apple ARM all-target checks for input/proto/ctl/app/tray and
wall/wall-sync/wall-canvas/wall-import, `-D warnings`, `-j 2`. Cross checks use
the repository's documented `blake3/pure` setting; they do not link/run native
hosts. `validation-m1a3.log` retains the rejected, incorrectly selected daemon
cross-build's missing `lib.exe` prerequisite error; it is not a source fix or
a passing check. `security-unit.log` contains the four Rust probe unit tests
and three disabled-command tests. Six Python warning/strace gate tests and
shell syntax checks also pass. No cargo fmt was run.

`manifest.json` records artifact digests, final source/binary hashes, and the
uncompressed trace digest. JSON and full logs are unmodified capture output;
raw log whitespace is preserved. Every supervisor reports owned-child reaping
and removal of its private session/media. Production extension/bridge, CLI
install/uninstall/status, doctor integration and native host qualification
remain **undelivered**. See `docs/spikes/wallpaper-m1-stop.md`.

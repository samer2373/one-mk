# 6D-M1 step A evidence: authenticated daemon presentation session

October 31, 2026 (Asia/Amman). Implementation commits `764545d` (I/O-free session
core), `f9604af` (daemon authority, renderer session, harness) and `15a8b31`
(release artifact gate). `source-sha256.txt` pins the exercised sources, the
unchanged M1a-2 warning baseline and both laboratory binaries.

## What ran

`qualify-m1-presentation.py` with `M1_CYCLES=100` inside the private headless
GNOME Shell 50.1 harness (two virtual 1920x1080 monitors, private session and
system buses, `env -i`, private HOME/XDG/ONE_MK_HOME under a fresh mode-0700
`/tmp/omkm1b`). Real processes only:

- **Daemon:** `one-mk run --backend null --no-discovery --no-notify --no-clipboard
  --port <free>` built with `--features wallpaper-m1-lab`, configured through an
  inherited descriptor (peer fingerprint, media hash `5e…`, canvas
  `-1920,0,5760x1080`, origin `0,0`), consent from its own `settings.toml`.
  Revocation used the real CLI (`one-mk wallpaper consent revoke`, `one-mk
  wallpaper disable`); clean stop used `one-mk stop`.
- **Renderer:** `one-mk-wall` built with `--features m1-lab`, spawned by the
  production extension through `Meta.WaylandClient` via a two-line `exec` wrapper
  adding `--m1-lab` and a log redirect (same PID, same inherited Wayland socket).
- Private Eval set the extension's laboratory authorization and disabled its
  first-frame watchdog for that bridge instance (step A draws no surface).

`result.json` is the machine report; `qualify-run.log` its console output;
`renderer.log`, `daemon.log`, `impostor.log` and `shell.log` the raw logs (only
trailing whitespace in `shell.log` was stripped for the repository whitespace
gate; the warning comparison ran on the original).

## Proven (real processes, this run)

1. **Authentication, no ambient trust.** Absent record; stale record left by a
   SIGKILLed daemon; that stale record replayed while a new daemon runs; the old
   (stale) key and a random (wrong) key on the live daemon's name; the live name
   with another PID; and a Python impostor serving the object path with its own
   record but no key — each refused, nothing admitted, renderer exited. The
   daemon answered an unauthenticated bus caller with `AccessDenied` and no plan.
2. **Plans.** The daemon's plan was admitted through `Gate::accept_encoded`
   (`revision=11 media=5e5e… canvas=-1920,0,5760x1080 slices=monitor-0@0,0,1920x1080;
   monitor-1@1920,0,1920x1080`, reserved left third kept). Pause produced sequence
   2. Replay of plan 1, stale sequence, stale consent revision, 4097-byte payload,
   64 KiB envelope and reserved-bit corruption were each refused (`Replay`,
   `Replay`, `Consent`, `Wire`, `Envelope`, `Wire`) with plan 2 still admitted in
   the same renderer process; `play` then admitted sequence 3, and a Shell layout
   change discarded the plan until fresh sequence 4. A mid-session forged reply
   tore the presentation down and the renderer exited in 5 ms.
3. **Teardown.** Renderer exit after the action (including CLI round trip where
   applicable), as "dedicated proof; maximum over 20 cycles": consent revoke
   20 ms; 19.4 ms, disable 19 ms; 20.0 ms, `one-mk stop` 84 ms; 86.0 ms, SIGTERM
   57 ms; 57.5 ms, SIGKILL 5 ms; 5.3 ms (bound asserted: 2 s). The Shell's automatic
   restart produced a fresh renderer that also refused (consent revoked, record
   absent after clean exit, stale record after SIGKILL). A clean daemon exit
   removed its record. Every renderer and daemon PID was reaped; no process
   carrying the laboratory runtime directory remained beyond the Shell session's
   own baseline after any cycle, and none after the harness stopped.
4. **Repetition and warnings.** 100 start → authenticate → admit → teardown cycles
   rotating revoke/disable/stop/SIGTERM/SIGKILL, each with a fresh daemon and
   consent revision, 100 renderer processes, all reaped. Zero new warning records
   versus `m1a2-evidence/baseline-shell.log` (14 records, all from Shell startup).

Unit coverage: `presentation_auth` (11 tests), the daemon's laboratory module
(3 tests, run with `-p one-mk --features wallpaper-m1-lab`), plus the existing
admission/wire suites. Workspace gates are recorded in the M1 report.

## Not proven / not done

- The renderer creates **no surface and no decoder**; "admitted/active" is intent.
  No media is resolved, imported, decoded or presented; no presented-frame,
  restoration, lock/unlock, suspend/resume or hot-plug surface evidence.
- The authority, runtime record and directives are laboratory-only (features off
  in release builds; the artifact gate rejects a daemon linking them).
  **Production authorization stays false**; `--present=gnome` still refuses. The
  production bootstrap/credential hand-off and its packaging are not designed.
- "Crash" is exercised as SIGKILL (no destructors, no unwinding); core-dumping
  signals were not used so the host's coredump handler is never involved.
- Same-UID code able to read the runtime record or ptrace is outside the threat
  model. No physical GPU/HiDPI, Windows/macOS or multi-computer evidence.

## Cleanup

The harness finalizer stopped the private session (`remaining_lab_processes: {}`);
a host process scan found no laboratory process. `/tmp/omkm1b` and the earlier
trial directory were deleted after copying this evidence; the worktree build
target and private `.lab` tree were deleted at the end of the task.

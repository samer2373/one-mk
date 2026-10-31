# Private GNOME component laboratory

This directory contains an **isolated headless Mutter** harness, not instructions
for manipulating the owner's running desktop. Read `production-component.md`,
`../../WALLPAPER-DESIGN.md` revision 2 and
`../../reviews/WAVE6-M1-REPORT.md` before interpreting results.

## Component conformance and repeated child teardown

Use one worktree-local `CARGO_TARGET_DIR=$PWD/target`. Build with a cleared
environment, a throwaway HOME, mode-0700 XDG_RUNTIME_DIR and ONE_MK_HOME, supplying
only PATH and the toolchain's CARGO_HOME/RUSTUP_HOME. Never inherit DISPLAY,
WAYLAND_DISPLAY, WAYLAND_SOCKET or DBUS_SESSION_BUS_ADDRESS. Build:

```text
cargo build --offline -j 2 -p one-mk -p one-mk-wall \
  --features one-mk-wall/gnome-bridge --examples --bins
python3 docs/spikes/gnome/qualify-component.py
python3 docs/spikes/gnome/test_m1_gate.py
```

The qualification script creates `target/gnome-component-qa`, copies the extension
into a disposable data directory, then starts `isolated-session.sh` with two
virtual monitors, private session/system buses and no X11. It compares warning
records, including stacks, against `m1a2-evidence/baseline-shell.log`. A changed
warning is a failure, not a reason to update the baseline. Its finalizer stops
the private session and removes copied extensions; reports/logs remain for
inspection. If sandbox restrictions prevent private sockets, request permission
for this same isolated command rather than connecting to a real session.

`bridge-conformance` checks the existing product protocol. `bridge-lifecycle`
is a **test-only example**: it authenticates Hello and reports Paused, never
creates a surface or decodes media. The runner starts/stops it 100 times, waits
for each exact PID to disappear and each Shell child handle to be reaped, checks
the private background URI is unchanged and forces GC between cycles. Private
Eval supplies authorization and resets the retry budget between these independent
cycles; neither is a production authorization path. A separate negative still
exercises the real three-start rolling-minute cap.

Passing this suite proves component IPC/child lifetime only. It does **not** prove
media playback, restoration after visible playback, daemon kill/crash teardown,
lock/unlock, suspend/resume, hot-plug surfaces, physical GPU behavior or production
consent. Never label these 100 paused-fixture cycles as 100 playback cycles.

## Presentation admission precursor

`crates/wall/src/presentation.rs` is an I/O-free, bounded state machine with
failure-path tests. A future adapter must provide bus-authenticated credentials,
the explicit local consent revision, frozen canvas and authoritative local
layout; call `owner_lost` on disconnect or owner replacement, and reconcile
consent changes. No identity supplied inside a plan is trusted. It must actually
destroy presentation/decoder resources when `active()` is absent. `active()` is
an admitted intent, **not permission to enable production playback**.

The module is not yet called by the daemon, bridge or renderer. A bounded
[presentation payload codec](presentation-wire.md) and `Gate::accept_encoded`
entry point now exist, with ten additional unit tests. This remains a precursor:
there is no new D-Bus method, daemon authority bootstrap or local-media resolver.
Transport body/rate limits,
cache verification, native lifecycle hooks and all production authorization gates
remain integration work. The original bridge XML deliberately still carries no
paths, media selection or frame bytes through Shell.

## Cleanup

Do not run concurrent laboratories at the same path. Stop a private session using
`SPIKE=<that exact private session path> isolated-session.sh stop`, never by
killing arbitrary GNOME/dbus processes. Preserve evidence outside `target` before
deleting the one build target. Do not touch X servers :0/:1, the clipboard,
the real session bus or `~/.config/one-mk`.

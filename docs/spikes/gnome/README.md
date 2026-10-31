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

## Presentation admission and daemon session (6D-M1 step A, laboratory)

`crates/wall/src/presentation.rs` is the I/O-free admission gate;
`presentation_wire.rs` the bounded payload; `presentation_auth.rs` the record,
keyed tags, daemon publisher and renderer session. The laboratory daemon
(`one-mk` built with `--features wallpaper-m1-lab`, null backend only) and the
laboratory renderer (`one-mk-wall` built with `--features m1-lab`, run as
`--present=gnome --m1-lab`) call them for real; see
[the session design](presentation-wire.md). `active` is admitted intent only:
the renderer creates **no surface or decoder** and production authorization
stays false.

`qualify-m1-presentation.py` proves the session with real processes in this
harness. It needs a short new private directory because Unix socket paths must
fit in 108 bytes (a worktree `target/` path is too long):

```text
cargo build --offline -j 2 -p one-mk --features wallpaper-m1-lab
cargo build --offline -j 2 -p one-mk-wall --features m1-lab
mkdir -p target/m1-bin && cp target/debug/one-mk target/debug/one-mk-wall target/m1-bin/
env -i PATH=/usr/bin:/bin HOME=<throwaway> LANG=C.UTF-8 USER=$USER \
  M1_BIN=$PWD/target/m1-bin M1_LAB=/tmp/<new-short-dir> M1_CYCLES=100 \
  python3 -B docs/spikes/gnome/qualify-m1-presentation.py
```

The production extension is copied with a two-line `libexec/one-mk-wall` wrapper
that redirects the renderer's output to `$M1_LAB/renderer.log` and `exec`s the
laboratory renderer with `--m1-lab` (same PID, same inherited Wayland socket).
Private Eval authorizes the bridge and disables its first-frame watchdog for that
bridge instance only, because step A draws nothing; neither is a production path.
Results and logs stay in `$M1_LAB`; the harness session is stopped at the end.
Evidence from the qualifying run is in
[m1-presentation-evidence](m1-presentation-evidence/README.md).

## Cleanup

Do not run concurrent laboratories at the same path. Stop a private session using
`SPIKE=<that exact private session path> isolated-session.sh stop`, never by
killing arbitrary GNOME/dbus processes. Preserve evidence outside `target` before
deleting the one build target. Do not touch X servers :0/:1, the clipboard,
the real session bus or `~/.config/one-mk`.

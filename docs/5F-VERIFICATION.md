# 5F verification record — 2026-10-30

This is an implementation acceptance record, **not release or hardware
qualification**. Baseline: `ea5df2a` (the per-device-key-rules plan). No
native capture/injection backend was started. Tests, daemons, CLI commands,
and windows ran with disposable homes/runtime directories and a clean
environment, without the owner's Wayland, D-Bus or session variables.
Window checks used only a private Xvfb display. No owner configuration was
used. All test processes were stopped after each harness run.

## Automated checks

The exact clean-environment `cargo test -j 2 --workspace` is **not green**:
the unchanged input-backend test
`portal::tests::the_portal_going_away_lets_go_and_reconnects_when_it_is_back`
times out waiting for reconnection. The same failure reproduces with the
previous build's existing input-test executable; neither portal source nor
portal tests changed in 5F. This is an acceptance blocker outside 5F, not a
test to silently ignore. Actual output:

```text
failures:
    portal::tests::the_portal_going_away_lets_go_and_reconnects_when_it_is_back
test result: FAILED. 179 passed; 1 failed; 3 ignored; 0 measured; 0 filtered out; finished in 10.76s
```

The previous executable, running only that test:

```text
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 143 filtered out; finished in 10.60s
```

The entire workspace with only that named test filtered passes:

```sh
cargo test -j 2 --workspace -- --skip the_portal_going_away_lets_go_and_reconnects_when_it_is_back
```

Final results include 289 main-daemon tests, 46 window tests, 179 input
tests, 149 protocol tests, and all remaining integration/doc tests. Existing
ignored hardware/manual tests remain ignored. All three required clippy
commands pass without warnings:

```sh
cargo clippy -j 2 --workspace --all-targets
cargo clippy -j 2 --target x86_64-pc-windows-msvc -p one-mk-input -p one-mk-proto -p one-mk-ctl -p one-mk-app -p one-mk-tray --all-targets
cargo clippy -j 2 --target aarch64-apple-darwin -p one-mk-input -p one-mk-proto -p one-mk-ctl -p one-mk-app -p one-mk-tray --all-targets
cargo build -j 2 -p one-mk -p one-mk-app
```

These commands require a clean environment as described above. Windows and
macOS results are **compile-only**, not execution results.

Pure tests cover thresholds, both axes, dead zone, finite/bounded hostile
motion, duplicate transitions, click replay, once-per-press outputs and
cancellation. Deterministic randomized streams exercise the engines,
parser inputs, and shared mapper held-output invariants. These are bounded
randomized property tests, not an exhaustive proof or a sustained fuzzing
campaign. Paused Tokio time checks the active-only deadline. In-process
two-daemon tests use real loopback transport and null backend handles to
inject motion/buttons, observe target scroll/actions and verify release on
visit end, reload, lock, simulated display sleep and controller abort.

Mutation check: removing the pre-interception mapper release on button-up
causes the reload/reserved-button regression to fail; restoring it passes:

```text
test app::tests::tuning::reserving_a_held_remapped_button_on_reload_still_releases_its_output ... FAILED
timed out: Elapsed(())
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 289 filtered out; finished in 5.01s
```

## Real executable checks

Two real null-backend executables connected over loopback. A previous
0.1.12 executable first wrote settings and paired trust stores. The new
build preserved the old pointer/key settings and the trust-store byte
hashes through configuration updates. The previous executable's precise
source commit was not established; this is migration evidence for those
actual files, not all historical formats.

The harness exercised help, listing, edits and failures for autoscroll and
gesture subcommands; scope/app changes, preset/remove/clear; visits and live
reload; receiver SIGKILL mid-visit; reconnect and switching; graceful sender
restart; lock-pointer; status, doctor and keymap check. Pairing used the
same-machine interface because invitations reject loopback by policy;
subsequent daemon transport used loopback. Actual output excerpts:

```text
autoscroll --speed NaN: exit=1
one-mk: autoscroll speed must be between 0.1 and 10
gesture threshold 0: exit=1
one-mk: gesture threshold must be between 1 and 10000 pixels
REAL BINARY PASS: previous-build migration, CLI success/failure, pairing, visits, reload, kill-9 peer, reconnect, graceful sender restart
All test processes stopped.
```

Invalid direction returns clap's usage error (exit 2); missing gesture
removal returns exit 1 with `one-mk gestures list` recovery guidance.
External null daemons do not expose an input-injection command. Therefore
physical-motion-to-scroll/action paths were verified by the in-process
two-daemon tests, **not injected into these external executables**. Actual
suspend, physical network/Bluetooth loss and native input recovery remain
unverified.

## Idle footprint and timer

Five-second `/proc` samples of the paired sender, final executable run:

```text
IDLE OFF, paired: 0 CPU ticks in 5.00s; 0.000% CPU; RSS 30.30 MiB
IDLE ON but inactive, paired: 1 CPU ticks in 5.00s; 0.200% CPU; RSS 30.30 MiB
```

A prior clean run sampled 0 ticks in both states (28.42/28.55 MiB RSS).
Whole-daemon samples include networking/background activity and scheduler
granularity; they do **not establish absolute zero idle CPU**. No inactive
timer/task is introduced by 5F. Only active autoscroll adds a 16 ms deadline,
including when displacement is inside the dead zone. Stop/reload/visit end
remove that deadline and discard fractional scroll remainders. Gestures
have no timer. No release build or LTO measurement was made.

## Real window review

The real debug window was captured and visually inspected on private Xvfb:
configured, empty, offline, saving, invalid speed, narrow (820×900), HiDPI
(3600×4400), and twelve long app-scoped rules with scrolling. Reduced motion
was enabled. Keyboard focus and Space toggled autoscroll; Tab/Return invoked
the preset and removed one rule. An invalid speed preserved the old setting
and displayed its error inline. Saving disabled the new mutation controls.
Actual harness output:

```text
KEYBOARD toggle after Shift+Tab + Space: False
PRESET + keyboard Remove result: 1 rules
Private Xvfb keyboard test processes stopped.
```

New action controls have button roles; toggle, speed and descriptions have
accessible names, and each Remove names its gesture. Palette calculations
give secondary text 7.30:1 on the panel, 5.70:1 on pills and 5.32:1 on lit
buttons (disabled controls excluded). Native screen-reader operation and
real desktop scaling are **untested**. Xvfb keyboard checks explicitly
focused the private window because Xvfb ran without a window manager.

## Remaining qualification

Native GNOME/wlroots, Windows/macOS execution, real suspend, Bluetooth and
hardware button behavior are untested; status/doctor and the feature docs
say so. The desktop preset uses OS-default chords that may differ from a
user's desktop bindings. Advanced gesture editing is CLI/config-only.
Resolve the pre-existing portal reconnect test before claiming a green
workspace. Run the [manual hardware acceptance steps](INPUT-TUNING.md#troubleshooting-and-acceptance)
on each supported platform before shipping this feature as stable.

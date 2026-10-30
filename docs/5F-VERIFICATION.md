# 5F hostile-review remediation — 2026-10-30

Baseline: `wave5`, `5b1cd2d` (Save the Claude reviews of the Codex wave 5
work). Branch: `t3/fix-wave5-mouse-mode-defects`. This record supersedes the
earlier 5F acceptance claims. It covers **Part B only** of
`reviews/WAVE5-CLAUDE-REVIEW-5A-5F.md`; Part A is not fixed or qualified here.
This is not a claim of bug-free software or native-platform qualification.

## Findings and decisions

| Finding | Status | Change / deliberate policy |
| --- | --- | --- |
| B1 | Fixed | Both initial broadcast visits and retunes strip autoscroll and gestures. Middle/side button down/up reach both targets through ordinary mappings. The broadcast Tune has no mouse-mode deadline. |
| B2 | Fixed | Both engines and CLI reject primary left/right buttons, including aliases. Existing invalid settings are skipped, not installed. `keymap check`, doctor and the Mouse card show actionable warnings. The card does not portray those rules as active, including numeric `1`/`2` aliases. |
| B3 | Fixed, policy chosen | A release replays a click only while displacement has always stayed below `min(16, smallest threshold / 2)` backend motion units on both axes. Crossing slop permanently suppresses replay for that press, even for a wrong-direction/short swipe or return to origin. |
| B4 | Fixed | Selecting a device in For: hides the mode controls and explains that they require global/OS/computer scope. Other per-device controls remain available. |
| B5 | Fixed | `Mapper::click` interrupts pending tap/hold before emitting the replayed button, so a pending Ctrl hold becomes Ctrl+click, not click then Esc. |
| B6 | Fixed | Compare resolved autoscroll config and compiled gesture rules independently. Preserve unchanged engines; replace only the changed engine. Send ScrollStop only when active autoscroll actually stopped. |
| B7 | Fixed | Hand-written device-scoped mode rules are explicitly ignored, warned about, and excluded from active UI rows. They are not silently applied to all mice. |
| B8 | Fixed with documented distinction | The cancelling physical Esc down/repeats/up are consumed. A previously held Esc still releases remotely. Remap-generated Esc deliberately remains ordinary remote input, not a mode command; physical Esc is the cancellation control. |
| B9 | Fixed | Overlap warnings use resolved/inherited buttons instead of defaulting each override to middle. Direction parsing/removal accepts capitalized config directions. Deadline comment describes both timers. |
| B10 | Fixed by documentation, no coordinate conversion | CLI, README and tuning docs say backend motion units, not logical pixels. Keep the legacy `threshold_px` field for compatibility. Cross-OS/scaling equivalence is explicitly not claimed. |
| B11 / test notes | Fixed | Replace vacuous lifecycle assertions with in-place reload/sleep motion/release checks and retained-Tune cleanup tests; mutation-check cancellation. Commit the real-binary keyboard/screenshot/CPU harness, with asserted saved values instead of the old ambiguous `False` printout. |

## Regression and mutation evidence

Initial red runs reproduced primary-button capture, missed-swipe replay,
pending tap/hold click ordering, broadcast interception, unconditional retune
cancellation, and capitalized CLI removal. Self-review added a numeric UI
alias regression, observed failing on `1`, then fixed it. Additional config
and UI coverage was added during implementation; no claim is made that every
config/UI assertion had a separately recorded pristine-baseline red run.

`mouse_modes_cancel_on_relevant_reload_and_display_sleep` keeps the visit
alive, changes actual mode rules on reload, then moves/releases after gesture
cancellation. It no longer immediately leaves the visit or merely observes a
dropped controller. The retained-Tune test exercises both cancellation and
release, then explicitly tries to tick, fire, and replay the cancelled mode.
Lock/disconnect destroy the visit state; the old branches were not proof of
the extra 5F cancellation calls and are no longer presented as such.

Each deliberate mutation below was tested separately, observed failing, and
restored before final validation:

| Deliberately removed behavior | Regression that failed |
| --- | --- |
| Gesture cancellation inside Tune cancellation | `mouse_modes_cancel_on_relevant_reload_and_display_sleep` |
| Display-sleep call to cancellation | `mouse_modes_cancel_on_relevant_reload_and_display_sleep` |
| Replacement of changed gesture rules on reload | `mouse_modes_cancel_on_relevant_reload_and_display_sleep` |
| Mouse-mode cancellation from Tune release | `retained_tune_cannot_emit_gesture_or_scroll_after_release` |

Other focused coverage includes held-Esc repeat/release safety, cancelling Esc
followed by an ordinary Esc, independently changing each mouse mode, unchanged
retunes, unsafe existing config, device-scope warnings/resolution, inherited
button warnings, slop/return-to-origin, and real two-target broadcast routing
before and after reload. Existing held-output and randomized engine/mapper
tests remain; they are not exhaustive fuzzing or hardware proof.

## Automated checks

All run with a clean environment, disposable HOME / ONE_MK_HOME and a 0700
runtime directory. DISPLAY points only to a private Xvfb; no Wayland or D-Bus
session variables are inherited. The owner's daemon/window/config/clipboard
are never used. Every cargo command uses `-j 2`; no `cargo fmt` was run.

```sh
cargo test -j 2 --workspace
cargo clippy -j 2 --workspace --all-targets -- -D warnings
cargo clippy -j 2 --target x86_64-pc-windows-msvc -p one-mk-input -p one-mk-proto -p one-mk-ctl -p one-mk-app -p one-mk-tray --all-targets -- -D warnings
cargo clippy -j 2 --target aarch64-apple-darwin -p one-mk-input -p one-mk-proto -p one-mk-ctl -p one-mk-app -p one-mk-tray --all-targets -- -D warnings
cargo build -j 2 -p one-mk -p one-mk-app
```

The full workspace passes **without filtering the portal test**. The former
portal failure in this document is not a current failure. Existing ignored
hardware/manual tests remain ignored. All three clippy commands pass with
warnings denied. Windows/macOS results are compile-only, not native execution.

## Reproducible private-window and binary QA

`scripts/test-mouse-modes.py --output PATH` creates its own private Xvfb,
isolated homes/runtime directories, an X11 sender and null-backend peer,
loopback pairing, and windows. Requires Linux /proc, Xvfb, XTest, xwininfo and
Pillow. It starts no owner-session processes and stops its own children.
Build the two binaries first with the command above. `--skip-idle` repeats UI
checks without replacing the CPU measurements; `--seconds` controls each
sampling interval (default five seconds).

Inspected screenshots: off, on, primary-button rejection, ignored device
config, device selected, narrow and narrow-scrolled, high-density fixture,
long-rule list and long-rule-list-scrolled, saving, success, error, offline,
and keyboard off/on. Text and controls remain readable in the scrolled Mouse
card; device selection shows an explanation instead of doomed controls.
Saving/error are real window command-job states, using an isolated sibling
CLI wrapper that delays writes two seconds and optionally simulates a write
failure. That is UI fault injection, not evidence of an actual disk failure.

The harness focuses the speed text field, sends Shift+Tab twice to the toggle,
then Space. It repeats in both directions, asserting actual `settings.toml`
values, and captures the resulting states. It also clicks Remove on a
capitalized `Left` gesture and asserts that the stored rule disappears.
This replaces the old uncommitted harness / unasserted `False` claim.

## Idle CPU and broadcast

Linux debug executable, private Xvfb capture, one loopback peer, discovery off.
Both modes disabled for OFF; autoscroll enabled and side-swipe preset for ON.
Middle and side clicks are injected during broadcast. Two seconds settle,
then approximately five seconds sampled per state. `/proc/PID/stat` measures
user+system ticks; CLK_TCK=100. Percent is one core. RSS was 36.27 MiB.

| State | OFF ticks / CPU | ON ticks / CPU |
| --- | --- | --- |
| Before broadcast | 0 / 0.00% | 0 / 0.00% |
| Broadcast, after middle/side clicks | 10 / 2.00% | 11 / 2.20% |
| After broadcast ends | 1 / 0.20% | 0 / 0.00% |

These are short, quantized measurements, not a promise of absolute zero CPU.
Active X11 broadcast itself has a measurable baseline even with modes OFF;
this change does not eliminate that separate work. The important deterministic
assertion is **no mouse-mode deadline in broadcast**, not that the daemon's
entire `broadcast_deadline()` is None: transport keepalives still need a
deadline. No 16 ms mode deadline survives the broadcast middle click or exit.

## Hostile self-review and remaining qualification

Checked initial broadcast construction as well as retunes, preservation of
each mode independently, primary aliases/numeric buttons, skip-versus-warning
behavior, mapper release ordering, consumed-Esc repeats and already-held Esc,
case-insensitive removal, sticky click slop, and the distinction between
mouse deadlines and transport deadlines. Numeric UI aliases were found and
fixed during this pass. No unresolved Part B defect is knowingly retained.

Still unqualified: native Windows/macOS capture/injection, physical multi-mouse
devices, scaled-monitor motion feel, portal/wlroots live sessions, desktop
switch shortcuts across real desktop environments, and long-duration CPU/RSS
soaks. The high-density screenshot is a fixture, not a physical scaled-display
test. Opt-in status remains. Part A and other pre-existing features are outside
this remediation; green tests do not establish enterprise release readiness.

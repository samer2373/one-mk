# Wave 6 independent adversarial audit

Date: 2026-10-31. Repository: one-mk 0.1.15. Base: `main` at `ff0d44b`.
Branch: `t3/wave6-adversarial-audit`.

## Result and qualification

**Six new findings: 0 critical, 0 high, 5 medium, 1 low. All six fixed.**
Every fix was preceded by a failing regression or, for packaging, the failing
existing packaging check. Required Linux tests/clippy and the requested Windows
and macOS cross-clippy gates pass. Additional deterministic stress and historical
fixtures are committed in `792d5fa`.

This is not an enterprise certification or a promise of no bugs. Native desktop,
hardware and distribution acceptance gaps remain below. Cross-compilation, null
backends, simulated input, fixtures and software-rendered Xvfb do not establish
Windows/macOS/Wayland hardware correctness. No newly discovered defect is left
unfixed; pre-existing limitations and unverified release criteria are not counted
as newly discovered defects.

## Isolation and method

Tests used disposable homes and runtime directories, `env -i`, null backends or
private Xvfb, and daemon `--no-discovery --no-notify`. The final reproducible runner
is `scripts/audit-wave6-isolated.sh`: private user/mount/network/PID namespaces,
chroot, private `/proc`, disposable `/h`, `/r` (0700), and `/tmp`; no inherited
DISPLAY, WAYLAND_DISPLAY or DBUS_SESSION_BUS_ADDRESS. Its Xvfb wrapper selects a
display above 10000 and Mesa software rendering. Pairing used private loopback;
a namespace-only TEST-NET dummy interface allowed invitation construction without
LAN access. Tray smoke used a new `dbus-run-session`, never the owner's bus.

No owner clipboard, GNOME session, X server :0/:1, or `~/.config/one-mk` was used.
The sole build target was `.audit-wave6/target`; Cargo used two jobs. No cargo fmt
was run. Temporary logs contained only disposable test data and were removed at
completion rather than retaining pairing tokens or large raw diagnostics.

Prior Wave 5 QA/behaviour/native-remediation and Wave 6 typing/M1 reports were
reviewed as leads, not accepted as proof. Fresh live checks reverified mirrored
layout offset clearing, pairing, clipboard privacy/recovery, and UI behaviour.
Workspace tests reverified broadcast release/disconnect, layout convergence,
guard and update-safety claims. Unsupported native character injection and the
preparatory wallpaper policy are not presented as newly completed runtime work.

## Findings and failing-before evidence

### W6-01 — Medium: unknown wallpaper settings lost on unrelated save

Custom wallpaper deserialization bypassed the unknown-field tracking used when
round-tripping settings. A future wallpaper table survived load but disappeared
on an unrelated settings update. This violated promised forward-field retention.

- Before: `settings::tests::unknown_wallpaper_tables_survive_unrelated_settings_edits`
  failed when reading the preserved table after save.
- Fix: `c33f658`, explicitly collect unknown wallpaper keys while preserving
  the fail-closed interpretation of unrecognised wallpaper policy.
- After: regression and full workspace gate pass. Existing table fields and
  future tables survive unrelated edits.

### W6-02 — Medium: unbounded settings reads before parsing

A local oversized settings file was fully read before TOML validation. This
allowed unnecessary memory consumption at startup and on settings reload; it is
not an unauthenticated remote daemon exploit.

- Before: `hooks::tests::oversized_settings_are_rejected_before_parsing` failed
  because the oversized file was not rejected by the size guard.
- Fix: `a87d2d9`, enforce a 4 MiB bound using metadata plus a bounded read that
  also covers file growth after the metadata check.
- After: regression and full workspace gate pass; rejected originals remain
  untouched. The size limit is documented in README troubleshooting.

### W6-03 — Medium: hostile benchmark timestamp panics the benchmark client

A benchmark QUIC peer could echo impossible timestamps. Unsigned subtraction in
the benchmark latency calculation panicked rather than ignoring that sample.
This affected the CLI benchmark against a hostile target, not normal daemon
packet decoding or an unsolicited remote-daemon crash.

- Before: `bench::tests::hostile_probe_timestamps_do_not_panic_or_replace_valid_samples`
  failed with `attempt to subtract with overflow` and a panicked-task JoinError.
- Fix: `76c7bed`, validate probe identity and use checked timestamp subtraction.
- After: the adversarial QUIC regression rejects the bad reply and accepts a
  subsequent valid sample; workspace gate passes.

### W6-04 — Medium: nonempty clipboard history unusable in the window

The app still parsed an old TSV representation while invoking the CLI's human
history output, which now includes a heading and relative ages. A real populated
window showed “Could not read history. Update one·mk and try Refresh.”

- Before: `daemon::tests::history_accepts_the_daemons_stable_json_metadata`
  failed with that error; `../screens/wave6/history-before.png` captures it live.
- Fix: `e6b2f70`, request the existing stable JSON metadata mode and parse its
  bounded, validated schema. Preserve hidden previews/origins and truncate text
  only at UTF-8 character boundaries.
- After: the regression and live window history list, Show/Hide, mouse copy and
  Enter restore all pass. See `../screens/wave6/history/normal.png` and
  `../screens/wave6/history/revealed.png`.

### W6-05 — Medium: human status suppresses saved-history corruption errors

Status deliberately hid routine privacy guidance, but also hid actual saved
history read failures. During live corruption recovery it misleadingly reported
only an empty history, even though the damaged file was retained.

- Before: `status::invitation_tests::history_failures_remain_visible_in_human_status`
  failed its assertion that the rendered status contains the read error.
- Fix: `1cfaab2`, display failure notes while continuing to omit routine guidance.
- After: regression passes; live damaged-history test sees the diagnostic,
  verifies original bytes remain unchanged, and recovers through explicit Clear.

### W6-06 — Low: packaging smoke rejects correct window help

The packaging test expected `one-mk-app --help` to fail. Correct successful help
therefore failed the unpacked-package gate; this was a test defect, not a broken
installed help implementation.

- Before: the existing CLI packaging smoke failed on unpacked app help.
- Fix: `550486e`, assert successful usage text, exact version and an exit-2 usage
  error for an unknown argument instead.
- After: the packaging smoke reports `all checks passed`; Ruby syntax validation
  is explicitly skipped because Ruby is not installed.

## Execution ledger

All final commands below ran in the clean isolated runner. Raw scratch logs were
inspected during the audit and deleted at cleanup; this ledger retains their
results, not a claim that deleted logs remain available.

| Gate | Observed result |
| --- | --- |
| `cargo test --offline -j 2 --workspace` | 1187 passed, 0 failed, 17 ignored (sum across test binaries/doc tests) |
| `cargo clippy --offline -j 2 --workspace --all-targets -- -D warnings` | Pass |
| Cross clippy, proto/input/ctl, `x86_64-pc-windows-msvc`, all targets, `-D warnings` | Pass |
| Cross clippy, proto/input/ctl, `aarch64-apple-darwin`, all targets, `-D warnings` | Pass |
| `audit-wave6.py` | Pass: 208 CLI invocations, matching-code pairing, two real null daemons, transferred file SHA-256 matches |
| `test-clipboard-history.py` | Pass: two real daemons and private X11 clipboards; crash/corruption recovery and live window interactions |
| `test-mouse-modes.py` | Pass: private X11 broadcast and settings interactions, save delay/error UI, screenshots |
| `two_null_daemons_receive_a_simulated_edge_drop --ignored` | Pass: simulated arm/drop through real daemon transport and received bytes |
| `audit-wave6-ui.py` | Pass: 11 live/explicitly synthetic screenshot cases |
| `test-idle-wakeups.py` | Pass: all six null/X11 idle scenarios |
| Packaging smoke | Pass; Ruby-only syntax check skipped |
| Tray/private D-Bus | StatusNotifierItem registered; process alive, lock present, no daemon auto-start or stderr |

The 17 ignored workspace tests are not silently counted as executed. The edge
drop test was run separately. Manual portal/native-session and resource-heavy
helpers remain outside the aggregate gate; the maximum 256 MiB history-save
input-latency helper was not run. The passing ordinary crash-save test is not a
substitute for that maximum-size latency measurement.

Wire stress: `crates/proto/tests/adversarial_decode.rs` sends 100,000 deterministic
buffers across all five decoder entry points, sweeps tags and lengths through
512 bytes, adds 65535-byte patterns, and mutates/truncates valid control frames.
Successful decodes are round-tripped where applicable. Settings stress performs
2048 deterministic byte/truncation mutations plus nesting depths 64/256/1024;
rejection must not modify input and accepted saves must reload. These are bounded
mutation tests, not coverage-guided libFuzzer campaigns or a proof of totality.

## Feature coverage and limits

“Live” means actual subprocess/transport/UI work in the isolated lab, not native
hardware acceptance. “State tests” means the workspace's automated pure/simulated
daemon tests were actually run, not merely read.

| Feature | Fresh evidence | Remaining qualification |
| --- | --- | --- |
| Clipboard history, fetch, transfer | Live notification capture, local restore, opt-in/rejected pull, no automatic paste, PNG/HTML, app/sensitive filtering, prune/dedup/delete, reconnect, persistence | Native Wayland/Windows/macOS clipboard and real sensitive-app detection |
| File Send/drop zone | Live 96 KiB file with hash check; real two-daemon simulated edge-drop test | Native OS drag gesture and external desktop drop source |
| Latency graph | Live connected pages and explicit synthetic loss/spike/long-name screenshot; 100-probe CLI run and hostile QUIC regression | Real impaired network and long-duration graph soak |
| Broadcast | Live CLI toggles and private X11 traffic after mouse-mode buttons; random press/drop/release state tests | Multiple physical computers and heterogeneous OS injection |
| Pointer wrap/layout learning/profiles | Live wrap and profile lifecycle/SSID configuration; mirrored offset and clear verified on peer; learning/convergence/guard state tests | Physical edge calibration, monitor hotplug and real network auto-profile switching |
| Autoscroll/gestures | Live config and unsupported trigger/device rejection; private X11 mode interactions; Remove and keyboard toggle saved TOML; tuning/release state tests | Positive physical gesture/autoscroll acceptance on supported hardware |
| Per-app/per-device rules/remap | Live add/check/remove, explicit device fingerprint persistence, tuning/button presets and reset; per-computer/broadcast/tap-hold/release state tests | Physical device fingerprint attribution and real foreground-app transitions |
| Global hotkeys | Live on/off/config/help and hotkey conflict/dispatch state tests | Positive OS-global registration and physical activation, especially Wayland portal |
| Switch/lock/gaming/edge guards | Live switch/home/takeback/lock/gaming toggles, invalid/offline failures; held-key/guard/wrap state tests | Secure desktop, real lock/suspend and fullscreen application transitions |
| Pairing/invites/join/free placement | Live matching numeric-code pair and invitation/join via clipboard harness; malformed invite rejection; placement/trust state tests | Real LAN discovery intentionally disabled; physical multi-peer placement |
| Doctor | Live human/JSON diagnostics and history/backend platform coverage; decision state tests | Native OS diagnosis and remediation acceptance |
| Media/hooks/power sync | Real pointer-leave command marker; live hook/power/lock settings and null-backend lock-all; media release and power state tests | Real media applications, screensaver inhibition and OS suspend propagation |
| Signed updates (gated) | Live default check rejection; local signed download/replay/mutation, kill/recovery, hostile framing and replacement state/integration tests | Actual publisher key/release feed, installed distribution and native installer acceptance |
| Windows service pure logic | Workspace state tests and requested input/ctl/proto cross clippy | SCM installation, service account permissions and interactive-session bridge |
| Packaging/Send-to | Real isolated integrate/remove and install/uninstall writes; package content/help/version/desktop/AppDir/Flatpak/Homebrew checks | Ruby syntax, native MSI/DMG install/upgrade and Windows Explorer Send-to |
| Window/tray | Live private X11 window interaction/screenshots, long-name/narrow/HiDPI/error cases; private D-Bus registration | Real tray-panel rendering/menu interaction, accessibility and native desktop integration |
| CLI | Recursive nested `--help`, live getters/mutations and malformed/offline paths in 208 invocations | Every help branch exercised; not every possible argument combination or interactive native workflow |
| Settings/migration | Representative historical 0.1.13/0.1.14 fixtures load/save; unknown quoted tables survive edits; size/mutation stress | No actual historical binary upgrade or owner's config used; unknown fields inside array entries remain a documented non-preservation limitation |

Version 0.1.12 predates this settings.toml implementation in repository history;
its applicable upgrade case is an absent settings file loading defaults, not an
invented 0.1.12 TOML schema. Fixtures are representative schema examples, not
claims to have recovered historical end-user files. Upgraded history remains off
and non-persistent until opted in. Rejected settings and corrupt saved history
are not overwritten automatically.

## Idle and UI observations

20-second settle, then 20-second sampling, discovery off. Values are main-thread
voluntary context switches per second, **not CPU utilisation or timer wakeups**.
Every clipboard/history/X11/drop worker measured zero voluntary and involuntary
context switches in these windows; all measured main threads had zero involuntary
context switches.

| Backend | Alone | Offline paired peer | Connected peer |
| --- | ---: | ---: | ---: |
| Null | 0.000/s | 1.000/s | 4.250/s |
| X11 | 0.000/s | 1.050/s | 4.350/s |

Separate history windows measured 0–1 daemon CPU ticks over five seconds and zero
clipboard-thread ticks, around 39 MiB RSS. Active X11 broadcast CPU was roughly
2.3% with modes off and 3.1% with modes on; this is active work, not idle cost or
a cross-machine performance guarantee.

Screenshots live in `docs/screens/wave6/`: live pages, explicit `fixture-*`
synthetic latency/layout/long-name cases, history loading/unresponsive/offline/
revealed states and mouse settings/error states. Inspected populated history,
unresponsive history, small latency/long broadcast and failed-save images show
readable errors and no observed clipping/overlap. The pre-fix screenshot is
historical evidence and is intentionally not regenerated as a current pass.

## Reproduction

Requires Linux namespaces/chroot permissions, cached Rust dependencies and both
requested cross targets; X11 checks also require Xvfb and the Python/X11 imaging
dependencies already used by the existing scripts. The wrapper never uses the
owner's displays. Run from repository root:

```sh
bash scripts/audit-wave6-isolated.sh cargo build --offline -j 2 --workspace
bash scripts/audit-wave6-isolated.sh cargo test --offline -j 2 --workspace
bash scripts/audit-wave6-isolated.sh cargo clippy --offline -j 2 --workspace --all-targets -- -D warnings
bash scripts/audit-wave6-isolated.sh cargo clippy --offline -j 2 -p one-mk-proto -p one-mk-input -p one-mk-ctl --all-targets --target x86_64-pc-windows-msvc -- -D warnings
bash scripts/audit-wave6-isolated.sh cargo clippy --offline -j 2 -p one-mk-proto -p one-mk-input -p one-mk-ctl --all-targets --target aarch64-apple-darwin -- -D warnings
bash scripts/audit-wave6-isolated.sh python3 -u scripts/audit-wave6.py .audit-wave6/target/debug/one-mk --report .audit-wave6/cli-report.json
bash scripts/audit-wave6-isolated.sh python3 -u scripts/audit-wave6-ui.py
bash scripts/audit-wave6-isolated.sh bash -c 'ONE_MK_AUDIT_SCREENS="$PWD/docs/screens/wave6/history" python3 -u scripts/test-clipboard-history.py'
bash scripts/audit-wave6-isolated.sh python3 -u scripts/test-mouse-modes.py --output docs/screens/wave6/mouse --seconds 10
bash scripts/audit-wave6-isolated.sh python3 -u scripts/test-idle-wakeups.py .audit-wave6/target/debug/one-mk --settle 20 --seconds 20
bash scripts/audit-wave6-isolated.sh bash -c 'ONE_MK_TEST_BINARY="$CARGO_TARGET_DIR/debug/one-mk" cargo test --offline -j 2 -p one-mk two_null_daemons_receive_a_simulated_edge_drop -- --ignored --nocapture'
bash scripts/audit-wave6-isolated.sh bash -c 'BIN_DIR="$CARGO_TARGET_DIR/debug" sh scripts/check-packaging.sh'
```

Do not use legacy screenshot scripts that inherit desktop variables for this
audit. Wait for every runner to exit before deleting `.audit-wave6`: while a
namespace is active its root contains bind mounts. Completed namespace exit tears
down mounts/processes; the runner then removes its root. At audit completion the
build target, scratch roots/logs and Python bytecode caches were removed, and
test processes were checked for termination.

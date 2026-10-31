# Wave5 daemon/CLI implementation follow-up

Date: 2026-10-31. Base: `wave5` (`2fb27d6`, independent QA report).
Branch: `t3/fix-wave5-qa-findings`.

The complete `WAVE5-QA-REPORT.md` and its reproduction commands were reviewed.
README, `crates/app`, screenshots, and the sibling window/documentation task
are deliberately untouched. This is not a native-desktop qualification or a
claim of zero known defects.

## Finding disposition

| Finding | Disposition and evidence |
| --- | --- |
| H1 | Fixed for updated peers. `Layout.clear_offset` encodes explicit absence with a single trailing byte, distinct from legacy omission. Clear participates in the same version/fingerprint tie-break as a concrete offset. Actual two-daemon QUIC tests set/clear from either side, restart/reconnect both using retained trust, and exercise equal-version conflicts in both directions. Legacy peers intentionally retain previous behavior. |
| H2 | Fixed. CLI pair sends a bounded optional two-byte service-port extension after confirmation, records the actual daemon port, saves trust, then requests daemon reload. Two running null-daemon processes and two interactive pair processes connect immediately using private nondefault ports. A real loopback exchange using the literal pre-extension confirmation algorithm verifies compatibility. Doctor running-daemon advice contains no `run --connect`. |
| H3 | Keyboard fallback supplied; native own-window suppression consciously not implemented. Shared capture APIs do not expose a reliable common native own-window exclusion. Status supplies `broadcast_stop_hint`; the sibling window task must remove the misleading clickable stop promise. No `crates/app` changes here. Until that integration is present, the original clickable UI remains unsafe on the reproduced X11 path. Other own-window clicks are not filtered by this patch. |
| M1 | Fixed. Ordinary status no longer prints the signing-key gate; doctor reports it as informational/skip, never warning. Explicit update checks retain it. |
| M2 | Fixed. Takeback, switch, lock-pointer, lock-all, broadcast, clip-get and clip-send use a shared actionable missing-daemon error and nonzero exit. Stop remains intentionally idempotent. CLI regression covers the six originally contrasted commands. |
| M3 | Fixed reported paths and expanded the IO audit. Settings open/UTF-8/read/write/rename errors name their action and path. Extension installation requires an explicit absolute regular nonempty renderer and wraps failures with source/destination. Identity/trust/control endpoint/send traversal errors gain action/path context. Tests cover read-only settings, invalid UTF-8, renderer omission/emptiness and hostile missing-send paths. This is not an assertion that every possible platform IO failure has been exercised. |
| M5 | Fixed CLI and status contract. `clipboard history on|off` works while stopped and reloads a running daemon. Off is distinguished from empty. Status exposes history enabled and fetch-shortcut enabled flags for the window; existing JSON history arrays retain their shape. |
| M10 | Fixed warnings and partial preservation. Each load warns with unknown key and file; unknown table keys survive save, including quoted dotted keys and flattened sections. Unknown fields inside array entries are warned but not preserved; retaining/reassociating them safely through rule edits would need a larger representation change. Release notes disclose this limitation. |
| M11 | Fixed conditional registration and startup notice. Fetch chord registration tracks local permission plus connected peers' advertised permission, including live settings changes and disconnects. Tests verify disabled chord passthrough and two-daemon permission changes. Existing settings receive a persistent one-time shortcut notice; a two-start CLI regression verifies it. Release notes name the defaults. Older peers do not advertise permission; explicit CLI fetch remains usable. |

## LOW findings

| Report item | Disposition |
| --- | --- |
| 1 | Fixed invitation relative expiry, grouped code, and join instructions for already-running computers. |
| 2 | Fixed typing preference output to use the computer name. |
| 3 | Fixed text history header, relative recorded time, explicit CLI show instruction; JSON timestamps remain machine-readable. |
| 4 | Fixed autoscroll off instructions and mouse-button errors. |
| 5 | Fixed disabled-only autoscroll status/qualification noise; other active rules still report normally. |
| 6 | Fixed wallpaper duplicate prefix/plural and informational deliberate no-discovery state. Doctor skips multicast probing when the daemon reports no discovery. |
| 7 | Fixed extension-uninstall success claim when nothing exists. |
| 8 | Fixed fullscreen allow/block names: bounded length, no control characters. |
| 9 | Fixed invalid control targets before transport and escaped missing-send/traversal paths. Failing-first CLI test caught and removed an earlier raw-path send precheck. |
| 10 | Fixed stderr logging and non-TTY ANSI; regression captures an actual wallpaper warning, not merely an empty log. |
| 11 | Fixed empty staging-directory cleanup after successful transfers. Cleanup shares the reservation lock with creation to avoid racing another transfer; interrupted nonempty staging directories remain for resume. Concurrent-transfer regression checks removal. |
| 12 | Fixed drag zone parser bounds and daemon-X11 availability wording rather than relying on the CLI environment. |
| 13 | Fixed no-argument gaming/lock-sync queries. |
| 14 | Fixed nested local-scope parentheses and removed misleading immediate-application text for Linux local remapping. |
| 15 | Not changed: sibling window scope. |
| 16 | Not changed: README/window performance scope and existing documented wakeups; no new periodic poll added. |
| 17 | Fixed new trust/certificate writes to Unix 0600; existing certificate permissions tightened on load. Tests cover certificate and saved trust permissions. |

## Failing-first and hostile review

The offset-reset regression and initial CLI regressions failed before their
fixes. Additional failing-first checks reproduced invalid control targets,
autoscroll off advice, world-readable certificate creation, running-daemon
doctor advice, and terminal controls in missing-send errors. Focused tests
were rerun after each fix. Checks were not retroactively weakened to hide
these failures.

Hostile self-review caught two material issues in the initial implementation:
splitting unknown TOML paths at dots would lose quoted dotted keys (replaced
with exact serde path components), and unconditional capability broadcasts
on every settings reload would perturb unrelated peer handshakes (now sent
only when capabilities actually change). It also caught the raw send precheck
ahead of the already-escaped transfer traversal. Bounds, legacy confirmation,
legacy layout omission, cleanup concurrency, and disabled-shortcut passthrough
are covered by tests. A further failing-first regression caught a CLI settings
save before the first upgraded daemon start swallowing the shortcut notice;
an on-save pending marker now preserves that notice even when defaults were
serialized by an earlier CLI edit. No owner's input,
clipboard, display, notifications, or
configuration was used for runtime validation; existing backend tests use
private fixtures.

## Reproduction and verification commands

All application processes use fresh homes and private ports, null input,
`--no-discovery --no-notify`, and no host clipboard. Child processes are killed
and waited by RAII. Runtime directories are explicitly 0700. Build output uses
the existing disk-backed shared target; a short private temporary path avoids
the Unix-domain-socket path-length limit. No `cargo fmt` was run.

From the checkout, create fresh directories (replace the build cache paths if
needed):

```sh
mkdir -p .qa/home .qa/runtime /tmp/mkqa-a-60af
chmod 700 .qa/home .qa/runtime /tmp/mkqa-a-60af
qa_cargo() {
  env -i PATH="$PATH" HOME="$PWD/.qa/home" \
    XDG_RUNTIME_DIR="$PWD/.qa/runtime" ONE_MK_HOME="$PWD/.qa/home" \
    TMPDIR=/tmp/mkqa-a-60af CARGO_HOME=/home/samer/.cargo \
    RUSTUP_HOME=/home/samer/.rustup \
    CARGO_TARGET_DIR=/home/samer/.t3/projects/one-mk/target cargo "$@"
}
qa_cargo test -j 2 -p one-mk clearing_offset_converges_from_either_daemon
qa_cargo test -j 2 -p one-mk equal_version_clear_and_offset_converge_from_either_side
qa_cargo test -j 2 -p one-mk fetch_shortcut_tracks_peer_permission_without_restart
qa_cargo test -j 2 -p one-mk-net service_port_extension_accepts_a_legacy_confirmation
qa_cargo test -j 2 -p one-mk --test qa_regressions
qa_cargo test -j 2 --workspace
qa_cargo clippy -j 2 --workspace --all-targets -- -D warnings
qa_cargo clippy -j 2 --target x86_64-pc-windows-msvc \
  -p one-mk-input -p one-mk-proto -p one-mk-ctl -p one-mk-app \
  -p one-mk-tray --all-targets -- -D warnings
qa_cargo clippy -j 2 --target aarch64-apple-darwin \
  -p one-mk-input -p one-mk-proto -p one-mk-ctl -p one-mk-app \
  -p one-mk-tray --all-targets -- -D warnings
```

Windows/macOS checks are cross-compilation/lint only, not native execution.
The exact final results and any intermittent upstream test failures are
recorded in the handoff below.

## Handoff validation

- Final `cargo test -j 2 --workspace`: **passed**, 1,132 tests passed and 14
  ignored across 38 test/doc-test suites. The 12 added CLI integration
  regressions also passed independently. The final run includes the real
  warning/non-TTY regression, escaped-send-path correction, and CLI-save-before-
  first-start upgrade-notice regression. Settings round-trip tests also pass.
- `cargo clippy -j 2 --workspace --all-targets -- -D warnings`: **passed**.
- Both specified Windows/macOS cross-target clippy commands: **passed**.
- `git diff --check`: **passed**. No formatter was run.
- Residual test risk: two earlier default-parallel runs failed the untouched
  `portal::tests::keysym_visit_release_failure_closes_both_sessions` at
  `crates/input/src/portal_tests.rs:252`. Both sessions were closed and symbols
  were empty, but the assertion expected three keysym calls and observed two:
  `[(122, 1), (122, 0)]` versus an additional `(122, 0)`. The isolated test and
  final default-parallel workspace run passed. No input-backend code or test
  assertion was changed to conceal this intermittent result. A serial
  diagnostic run was interrupted by the initially overstrict logging test
  (which exposed stdout rather than stderr logging and was then corrected);
  it is not reported as a passing full-suite run.
- H3 still needs sibling window integration and native qualification. M10
  array-entry preservation remains explicitly limited. These are not claimed
  fully resolved by the passing automated checks.

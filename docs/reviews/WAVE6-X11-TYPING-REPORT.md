# Wave 6: X11 character typing qualification

Date: October 31, 2026. Workspace version: 0.1.15.

## Decision

**Not safely fixed; do not enable production TEXT.** The requested fallback is
used: preserve physical-mode behavior and report reproducible negative evidence.
The acceptance condition of at least 50 strict lossless runs was not satisfied.
There is no production remapping implementation in this change.

The X11 backend still returns `keyboard: None`, rejects Text as unsupported,
and handles CharacterKey as physical fallback. The daemon only adds TEXT when
its backend supplies a keyboard descriptor. `caps::LOCAL` still excludes TEXT.
Debug null fixtures remain behind their existing test-feature/debug guards.
Portal/Wayland advertisement and Windows/macOS code are unchanged.

## Experiments and observations

All injection used independently started private Xvfb servers, never the owner's
`:0` or `:1`. The injector, XKB observer and real Xlib `xev` client use separate
connections. xev is stopped with SIGSTOP and confirmed stopped with waitpid
before injection, then resumed with SIGCONT. Its stdout is drained concurrently.
Each XTest event has a server sync; mapping changes are checked. The stopped
client deliberately cannot translate events during those server round trips.

### Immediate restoration

The existing counterexample still passes as a negative regression: 1/1 and
32/32 raw presses arrive, but 0/1 and 0/32 translate to the intended characters.
xev reports NoSymbol. Core and full XKB maps restore exactly.

Fifty separate exact test invocations under the clean/private runner with
`--require-delivery` produced **0 successes, 50 failures**. Every failing log
contained the expected delivery assertion, with `left: []`, `right: [305]`
(U+0131), rather than a startup/dependency failure. Each test process started
its own additional Xvfb. The loop's success means that all fifty *known
failures were verified*, not that delivery passed. The strict test exits 101.
It stops at the first-character failure; this does not claim fifty complete
32-character acceptance runs.

### Fixed-symbol retention control

The existing four-slot/four-symbol experiment was rerun:

- Keep mappings until the known test readers translate: 200/200 correct.
- Restore after 1.6 seconds before resuming readers: 0/200 correct.
- Both cases restore full core/XKB maps and preserve the other client's hold.

The passing test deliberately asserts the counterexample as well as the
positive control. It is not a passing strict-delivery gate.

### Retained-slot reuse, new regression

`retained_pool_reuse_corrupts_a_delayed_burst` injects 10,000 characters from
`ı ł @ z λ 🦀 é` through four spare keycodes. Unlike the fixed-symbol control,
seven symbols force actual slot reassignment. The private server uses
`setxkbmap -layout us -variant intl`; another injector client's letter and Shift
are held throughout the burst. All levels of each selected keycode are mapped
to its current intended Unicode keysym. Every pre-injection native snapshot
resolves the intended character.

Mappings remain installed until **both observers have translated every event**.
Nevertheless, all 10,000 presses translate using each slot's **final** mapping:
only **1,432/10,000** match the originally intended sequence, with **8,568 wrong
symbols**. The independent XKB reader verifies the exact final-slot sequence;
xev separately verifies the exact keysym on each of 10,000 ordered presses and
their corresponding releases, not merely the presence of some keysym.

The regression passes by asserting this corruption. Adding `--require-delivery`
fails with exit 101, as it must. Neither a negative-regression pass nor receipt
of every raw event is text-delivery acceptance.

After consumption, the test verifies all of the following:

- Every selected keycode is released.
- The unrelated letter and Shift remain held until explicitly released by their
  owning test client.
- The entire core keyboard mapping, including width, equals the original.
- The full `xkbcomp -xkb` dump equals the original.
- The entire down-key bitmap equals its original state.

These are normal-return/test-guard cleanup checks, not SIGKILL recovery.
The reader checks keysyms on a dead-key layout, not dead-key composition,
XIM commits, supplementary-plane glyph rendering, or actual editor content.
The burst is rapidly submitted but includes per-event server round trips; it
is not a throughput benchmark or an unthrottled application-event test.

## Why the investigated approaches are not qualified

1. **Serialize each character and sync:** already present in the counterexample.
   It acknowledges server request handling, not a stopped application's lookup.
2. **Sleep or quiet/LRU retention:** a stopped client can remain stopped beyond
   the chosen deadline. The 1.6-second experiment demonstrates that failure.
3. **Retained pool with local generations:** the new experiment fails before
   retirement. A delayed unmodified client looks up the keycode's current
   mapping; an injector-side generation counter does not supply it an old map.
4. **Never reuse or restore slots:** retains only a finite fixed alphabet and
   conflicts with bounded state, arbitrary Unicode and keymap-restoration
   requirements. It is not an acceptable production fallback.
5. **Read/modify XKB instead of the core map:** native XKB snapshots in these
   experiments also see the current map. This does not establish a delivery-safe
   XKB injection design; no such alternative was implemented or qualified.

This is not a proof that every possible X11 input-method/client-cooperation
design is impossible. A replacement needs a concrete destination-consumption
contract or an injection mechanism not dependent on mutable global keymaps,
then independent client, layout, modifier, crash and performance qualification.
Adding a journal only repairs state after a crash; it cannot recover characters
already translated against an incorrect mapping.

## Changes shipped

- Add the ignored 10k retained-slot reuse counterexample and `--pool-reuse`.
- Distinguish translated-symbol count from correct-character count in output.
- Verify full down-key bitmap restoration as well as core/XKB restoration.
- Reject reserved desktop display numbers before making any test connection;
  the Rust Xvfb owner also tears down its child if this check panics.
- Fix the runner's single Cargo target directory to the checkout's
  `target-x11-typing`, and redirect test temporary files into its disposable
  TMPDIR. HOME/state/runtime remain throwaway and runtime has mode 0700.
- Update keyboard-layout status and feature-plan row 5E; leave production off.

## Quality gates

Run through `bash scripts/test-x11-typing-safety.sh <command>`:

```sh
cargo test --offline -j 2 --workspace
cargo clippy --offline -j 2 --workspace --all-targets -- -D warnings
cargo clippy --offline -j 2 -p one-mk-proto -p one-mk-input -p one-mk-ctl --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo clippy --offline -j 2 -p one-mk-proto -p one-mk-input -p one-mk-ctl --all-targets --target aarch64-apple-darwin -- -D warnings
```

Results: workspace tests **1,149 passed, 0 failed, 15 ignored**; workspace
all-targets clippy and both cross-target all-targets clippy commands passed with
warnings denied. Cross clippy is compilation/linting, not Windows/macOS runtime
acceptance. The explicit immediate, fixed-retention and new reuse counterexample
tests passed their negative-regression assertions; strict delivery did not pass.
`bash -n` and `git diff --check` passed. No `cargo fmt` was run.

## Isolation and remaining gates

The runner uses `env -i` with an explicit PATH, throwaway HOME,
XDG_RUNTIME_DIR/ONE_MK_HOME/TMPDIR, existing Cargo/Rustup caches and one worktree
target directory. No owner DISPLAY, WAYLAND_DISPLAY or session bus address is
inherited. Only the runner's own display is supplied to Cargo; the X11 regression
starts its own server and addresses it explicitly. No real desktop daemon was
launched. Existing daemon subprocess tests use the null backend with discovery,
notifications and clipboard disabled. Existing portal tests use private fake
buses; no owner session D-Bus was used. No owner clipboard or configuration was
accessed. The runner kills/reaps its Xvfb and removes its scratch tree; test
guards kill/reap their Xvfb/xev children. The build directory is removed after
validation; transient logs are summarized here rather than retained as artifacts.

Not completed: a safe production injector, fifty successful strict runs,
two-real-daemon layout/CharacterKey/shortcut acceptance, full modifier/layout
matrix, editor/compose commits, SIGKILL/restart recovery, concurrent user keymap
changes, native latency/idle measurements or real GNOME/KDE portal tests.
Production TEXT must remain disabled until those applicable gates pass.

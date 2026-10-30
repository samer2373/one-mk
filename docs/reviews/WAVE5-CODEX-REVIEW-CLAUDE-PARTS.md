# Wave 5 hostile review: Claude parts 5B / 5C / 5D / 5I

Reviewed by Codex, 2026-10-30. **Verdict: do not release this snapshot.** Two
independent device-removal paths can leave keys/buttons held. Passing the existing
tests is not sufficient evidence for the no-stuck-input guarantee.

## Snapshot and method

- Reset this review worktree with `git reset --hard wave5`, as requested. The
  inspected/tested HEAD is `3389e12ce58dffe72fffb3a659a4cc08f73aff4d`,
  `Merge keyboard layout character mode foundations (5E) into wave5`.
- Used `git log --oneline main..wave5` and scoped `git diff main...wave5 -- ...`,
  then followed the merged call paths, not just individual feature commits.
  The shared `wave5` ref advanced during review to `5b1cd2d`; the findings and
  line numbers below refer to the frozen HEAD above. A drift check found no
  changes in the primary scoped broadcast, push, tune, devcorr, rtt and links
  implementation files in those later commits.
- Repository source was not edited, including for mutation checks. Temporary
  Rust harnesses were assembled in memory and compiled through rustc stdin
  against the actual source modules and Cargo-built dependency artifacts.
  Generated binaries are in ignored `target/debug` (the first small pure probe
  is in the throwaway `/tmp` area). Only this report is committed.
- Test commands ran with `env -i PATH="$PATH" HOME=/tmp/one-mk-review-home
  XDG_RUNTIME_DIR=/tmp/one-mk-review-runtime ONE_MK_HOME=/tmp/one-mk-review-state`;
  runtime directory mode 0700. Cargo/rustc additionally received
  `CARGO_HOME=/home/samer/.cargo` and `RUSTUP_HOME=/home/samer/.rustup` for cached
  dependencies/toolchains. No Wayland or session-bus environment was inherited
  by tests. No real backend, clipboard, or owner settings were intentionally
  opened. Socket-dependent tests ran with approval outside the socket-restricting
  sandbox, still with this clean environment. X11 tests create their own Xvfb.
- Safety exception: a failed report-writing shell command incorrectly nested
  shell quotes around Markdown. Backtick expansion inadvertently invoked
  `one-mk takeback` outside the intended clean-environment wrapper. It returned
  `Operation not permitted (os error 1)` in the sandbox; no successful daemon
  control operation was observed. This was a tooling mistake, not a test, and
  is disclosed rather than claiming perfect isolation. Subsequent report writing
  uses a literal heredoc directly to `env -i ... apply_patch`.
- Cargo invocations used `-j 2`; no `cargo fmt`. Null-backend probe daemons were
  stopped and awaited **before** asserting the expected regression failure.
  No review cargo, probe, or Xvfb process remained at the process check.

## Ranked findings

### 1. P1 — Broadcast skips unplug cleanup and device-map refresh

**Locations:** `crates/one-mk/src/app/tune.rs:188`, especially `:194` and `:208`;
`crates/one-mk/src/app/broadcast.rs:424`, `:514`, `:726`;
`crates/one-mk/src/app.rs:1112`.

`on_devices` releases disappearing devices only from `self.remote`. Broadcast
deliberately keeps that field empty between events; its real `Remote` objects
are in `Broadcast::On.visits`. `Event::Devices` is not routed through
`broadcast_input`/`each_visit`, so the disappearance is recorded globally but
none of those mappers sees `device_gone`. The final `retune()` is also a no-op
without `self.remote`. Unlike settings reload (`app.rs:2174`), this path never
calls `broadcast_retune()`.

**Deterministic reproduction, executed:** Three real daemon state machines over
loopback QUIC with null input backends, using the existing broadcast `Trio`:

1. Report keyboard `DeviceRef(7)` on A and start broadcasting to B and C.
2. Send `Key { code: 30, down: true, device: Some(DeviceRef(7)) }` on A. Confirm
   both B and C inject the down transition.
3. Report `Event::Devices(vec![])` on A, simulating unplug/sleep mid-press.
4. Require a key-up at both receivers within 300 ms. **Neither arrives.**

The added, stdin-only test was:

```rust
#[tokio::test]
async fn review_broadcast_unplug_must_release() {
    let mut trio = Trio::start("review-unplug", Swap::Auto, |_| {}).await;
    trio.a.h.devices(vec![one_mk_input::fake_keyboard(7, "Review Keyboard")]);
    trio.on(871).await;
    trio.a.h.key_from(KEY_A, true, Some(one_mk_input::DeviceRef(7)));
    assert_eq!(trio.both().await, (key(KEY_A, true), key(KEY_A, true)));
    trio.a.h.devices(vec![]);
    let released = tokio::time::timeout(
        Duration::from_millis(300), trio.both()
    ).await;
    trio.stop().await;
    assert_eq!(released.ok(), Some((key(KEY_A, false), key(KEY_A, false))));
}
```

Compiled by inlining `main.rs`/`app.rs` module roots in memory, resolving their
module paths to the original files, appending this test inside the existing
`app::tests::broadcast` module, and using the dependency fingerprints of
`one-mk-996f3520b1a0990c`. Running
`target/debug/review-daemon-probes review_broadcast_unplug_must_release --nocapture`
returned 101: `left: None`, expected two key-up injections. No production logic
was replaced. The disconnect messages in that run occur during the explicit
trio shutdown after the timeout, not as the cause of the missing release.

**Impact:** A modifier or mouse button can remain held at every recipient while
another device continues input. Active broadcast visits continue their existing
keepalives (`app.rs:47`, `:308`), so transport silence is not device-removal
cleanup; ongoing input also prevents the receiver idle timeout. Broadcast stop
eventually cleans up, but unplug must not require the user to discover that.
The same omission means a newly connected/re-numbered device does not get its
per-device rules on active broadcast visits until another retuning trigger.
The latter follows from the call path; the executed regression tests the missing
release, not the hotplug remapping consequence separately.

**Suggested fix:** Factor device-gone release/held-state reconciliation into a
per-visit operation and apply it to both the ordinary remote and every broadcast
visit before rebuilding their maps. Refresh every visit on device arrivals as
well. Merely adding `broadcast_retune()` is insufficient: retuning intentionally
preserves held presses. Add key/button unplug, tap-hold unplug, replug with a new
ID, and hotplug-rule tests while another device keeps broadcast alive.

### 2. P1 — Correlator emits an attributed press after its device-removal event

**Locations:** `crates/input/src/devcorr.rs:142` and `:157`;
`crates/input/src/windows_devices.rs:78`;
`crates/input/src/macos_devices.rs:223`.

`Correlator::forget` removes entries from `seen` and `motion`, but leaves
`waiting` untouched. A press that already matched a device can still be queued
behind an earlier unmatched hook event. Windows enumeration and macOS removal
callbacks forget the device and publish the new device table. The queued press
can then escape on the correlator deadline, **after** the daemon has processed
the only removal notification that could have released it.

**Deterministic pure reproduction, executed against the real module:**

```rust
let mut corr = devcorr::Correlator::default();
let mut out = Vec::new();
let device = DeviceRef(7);
corr.hook(Event::Key { code: 31, down: true, device: None }, 0, &mut out);
corr.device_event(devcorr::Token::Press { code: 30, down: true },
                  device, 1, &mut out);
corr.hook(Event::Key { code: 30, down: true, device: None }, 1, &mut out);
assert!(out.is_empty());
corr.forget(device); // Backend publishes Devices([]) at this point.
corr.tick(8, &mut out);
assert!(!out.iter().any(|event| matches!(event,
    Event::Key { code: 30, down: true, device: Some(id) } if *id == device
)));
```

That safety assertion fails. Actual output after `forget`:

```text
[Key { code: 31, down: true, device: None },
 Key { code: 30, down: true, device: Some(DeviceRef(7)) }]
```

The standalone harness re-exported `one_mk_input::{Bus, DeviceRef, Event}` and
included `crates/input/src/devcorr.rs` by absolute `#[path]`; it was compiled as
`rustc --edition=2024 --test - -L dependency=target/debug/deps` with the matching
Cargo-built input/proto/ctl rlibs. `review_unplug_must_not_emit_a_late_device_press`
failed without starting any input backend.

**Impact:** For a normal remote visit as well as broadcast, device cleanup can
run before the mapper has seen the press. The delayed down is then mapped and
held with no physical up and no further removal event. It is enough for another
device to keep that visit active. This is independent of finding 1; fixing
broadcast dispatch alone does not fix this ordering bug. Windows/macOS hardware
behavior was not exercised: the verified failure is in their shared pure
correlator, and the removal ordering is established by source inspection.

**Suggested fix:** Establish an ordering fence between pending input and the
device-table removal. Either emit pending transitions before reporting removal
so mapper cleanup follows them, or cancel removed-device queued transitions
with balanced handling of anything already emitted. Clearing identity to `None`
does not solve the late-down problem. Test removal with an unresolved FIFO head,
an already-correlated later down/up, and subsequent timeout/capture finish.

### 3. P2 — Explicit recipient takeback is undone after 58 seconds of quiet

**Locations:** `crates/one-mk/src/app/broadcast.rs:29`, `:585`;
`docs/BROADCAST.md:197`;
`crates/one-mk/src/app/tests/broadcast.rs:389`.

The sender guesses why `Revoked` arrived from the time since its last input.
At or beyond `IDLE - 2 seconds`, **every** matching revocation means `Out::Rest`,
including an explicit `one-mk takeback` by the recipient. The next input silently
re-enters that computer instead of respecting its decision to leave broadcast.

**Concrete reproduction:** A broadcasts to B/C, then sends no input for 59
seconds. Before the 60-second idle expiry, B runs `one-mk takeback`. The next
keystroke on A re-enters and types on B. An application/permission state changed
by B in the meantime can now receive unexpected input. This is not an unpaired
network-authentication bypass: it is failure to preserve an explicit local
revocation in an already paired session.

**Evidence:** This is a documented limitation, not a newly concealed behavior.
The existing passing test `idle_let_go_enters_again_with_the_next_input` shortens
the threshold to zero, issues the actual `Request::TakeBack`, and explicitly
expects B to remain in broadcast and accept the next key. That test ran in the
suite; I did not separately wait 59 real seconds. The source establishes the
production threshold. Calling a manual takeback an idle event in the test hides
the distinction that matters to safety. Documentation does not meet the stated
enterprise/no-known-bugs requirement when it merely describes the violation.

**Suggested fix:** Add a capability-gated explicit idle-revocation reason, and
only auto-rejoin a recipient that reported that reason. Legacy/unknown
revocations must fail closed: remove the recipient until the controller
explicitly restarts broadcast. Cover manual takeback on both sides of the time
threshold, old-peer revocations, and actual receiver idle revocation separately.

### 4. P2 — Reopening a latency graph charges old losses to the current window

**Locations:** `crates/app/src/rtt.rs:51` (loss delta before eviction), `:91`;
`crates/app/src/daemon.rs:257` (unchanged status excludes link counters), `:265`.

The watcher intentionally does not deliver periodic graph samples while the
graph is hidden and status otherwise stays unchanged. `Ring::push` preserves
`lost_seen` across that gap, computes a lifetime-counter delta, and assigns all
of it to the first fresh sample **before** evicting the old window. Thus losses
that happened long before the displayed two minutes are reported as recent.

**Deterministic reproduction, executed:**

```rust
let mut ring = rtt::Ring::default();
ring.push(0.0, LinkStats { rtt_us: 1000, lost: 0, sent: 100 });
// Graph hidden. Suppose these losses happened at t=100.
ring.push(600.0, LinkStats { rtt_us: 1000, lost: 100, sent: 1000 });
ring.push(601.0, LinkStats { rtt_us: 1000, lost: 100, sent: 1001 });
ring.push(602.0, LinkStats { rtt_us: 1000, lost: 100, sent: 1002 });
assert_eq!(ring.summary(602.0).unwrap().lost, 0);
```

Actual summary has three 1 ms samples and **100 lost packets**. The quality
becomes `Slow` despite no loss in the displayed `[482, 602]` seconds. The
unobserved interval cannot establish when those packets were lost; it must not
be presented as measured loss within the current history. This is misleading
diagnostics, not an input-safety issue. The same stdin-only pure harness ran
`review_old_loss_must_not_be_counted_in_a_new_window`, which failed as above.

**Suggested fix:** Reset the counter baseline when the prior sample is outside
the window, or represent an unobserved interval explicitly and exclude its loss
delta from claims about the displayed window. Consider shorter observation gaps
as well. Test hidden/minimized/tab-away histories with losses entirely outside
the new window; existing eviction tests use zero loss and miss this case.

## Test execution and actual results

The requested workspace suite was attempted in the clean environment. A first
sandboxed invocation failed on denied UDP binds (`app.rs:2803`), then poisoned
shared test mutexes; those cascading failures are environmental, not product
findings. The same command was rerun with approved loopback access:

```sh
cargo test --workspace --offline -j 2
```

| Component | Result in approved workspace run |
| --- | --- |
| Daemon unit/integration-style tests | 360 passed, 2 ignored |
| Idle wakeup integration | 1 passed (41.55 s) |
| Real-binary invitation CLI integration | 2 passed |
| Window app | 62 passed |
| Clipboard | 17 passed, 1 ignored |
| Control/status library | 50 passed |
| Input library | 200 passed, **1 failed**, 3 ignored |

The input failure was
`x11::tests::devices_are_listed_and_input_says_which_one`: a three-second event
wait timed out in `x11_tests.rs:215`. An approved isolated rerun passed in 0.18 s:

```sh
RUST_BACKTRACE=1 cargo test -p one-mk-input --offline -j 2 \
  x11::tests::devices_are_listed_and_input_says_which_one -- --exact --nocapture
```

**The full suite was not green.** The isolated pass does not erase the original
failure. Root cause remains unresolved (intermittent Xvfb/test ordering/load is
possible, not established). Because Cargo stopped there, the remaining packages
were explicitly run, rather than silently claiming they passed:

```sh
cargo test --workspace --exclude one-mk --exclude one-mk-app \
  --exclude one-mk-clipboard --exclude one-mk-ctl --exclude one-mk-input \
  --offline -j 2 --no-fail-fast
```

Results: net 23 passed; invitation integration 10 passed; loopback integration
7 passed; proto 186 passed; remap benchmark 1 ignored; tray 12 passed; wall-sync
integration 12 passed. Their doc tests passed (zero tests). No Windows/macOS
target was installed/checked during this review. Do not interpret Linux passing
tests or the feature docs' historical compile-check claims as fresh cross-target
qualification by this reviewer.

## Mutation checks, without source edits

For both modules the unmodified stdin harness first ran all **20 existing
devcorr/rtt tests successfully**. I then compiled separate copies modified only
in memory, with the same dependencies:

1. Replace the nondecreasing-counter loss delta in `Ring::push` with zero.
   Existing `any_loss_shows_and_a_lot_is_slow` **fails**: actual
   `(0, Some(Smooth))`, expected `(1, Some(Stalls))`.
2. Remove `self.motion.retain(|m| m.0 != device)` from `Correlator::forget`.
   Existing `motion_is_attributed_only_when_one_device_moves` **fails**:
   actual `Some(DeviceRef(2))`, expected `None` after removal.

These checks show those specific assertions are effective, not that the suites
cover all failure paths. Both original modules pass their existing tests while
the new regression assertions in findings 2 and 4 fail. The broadcast unplug
test in finding 1 also fails against unmodified production logic.

## Verified OK, and qualification limits

- Existing loopback tests exercise broadcast delivery, stopping mid-hold,
  emergency/lock shutdown, target loss, independent visit state, and tuning.
  They passed on Linux with null backends. This verifies those test scenarios,
  not every physical backend or kill/network ordering. Broadcast device-removal
  coverage was missing, as the new regression demonstrates.
- Ordinary single-visit device removal is covered by
  `app/tests/tuning.rs:535` and passed. Its late-release section (`:569`) batches
  removed-device and surviving-device ups before checking the output; that is
  not an ordering-sensitive proof that a removed-device up does nothing. Add
  an assertion before the surviving device releases.
- Push tests for opposite-edge wrapping, preserving real/offline neighbors,
  approach guards/lock/owner restrictions, suggestions and mirrored auto-layout
  passed (`app/tests/push.rs`). Inspection found no additional substantiated 5D
  defect. There is no new idle polling timer in the reviewed push state machine;
  the idle wakeup regression passed. This is not proof of literally zero CPU on
  every backend, nor a new footprint/latency benchmark.
- Graph storage is capped and old samples are evicted; missing links clear
  their rings, status handles absent older-daemon link data, and geometry/
  hostile-input tests passed. The graph reads existing QUIC stats rather than
  adding a network probe loop. Finding 4 concerns attribution across gaps, not
  those bounds. Existing app polling is still present; no claim of zero window
  CPU is made.
- Device identity is explicitly an OS-reported vendor/product/serial-or-name
  selector (`proto/src/device.rs:111`), **not cryptographic authentication**.
  A locally attached device can impersonate these fields; do not use the rule
  selector as an authorization boundary. I did not find evidence that an
  unpaired network host can directly choose the local backend's DeviceRef.
  Passing net/proto tests is not a full adversarial mTLS/protocol audit.
- Settings/old-status compatibility tests in the exercised packages passed.
  The legacy `Revoked` ambiguity is not resolved by compatibility tests; see
  finding 3. Native Windows/macOS input/notification behavior remains unverified.
- Screenshot hooks/scripts enumerate graph and broadcast states, but I did not
  render or visually inspect screenshots, drive keyboard navigation, or qualify
  long translated text, screen-reader behavior, or all DPI/window sizes. No
  visual/i18n/accessibility sign-off is implied. Platform limitations in
  `docs/BROADCAST.md` and `docs/EDGE-PUSH.md` are appropriately labeled, but their
  no-stuck-input explanations do not cover the two reproduced removal bugs.

Release gate: fix findings 1–2 with ordering-sensitive regressions, make an
explicit fail-closed decision for finding 3, correct finding 4, and resolve the
input-suite intermittent failure. Then rerun the full suite and qualify the
native input paths/visual states separately; this report is not a no-known-bugs
certificate.

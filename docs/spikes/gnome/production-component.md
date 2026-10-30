# GNOME component implementation checkpoint — NOT a playback qualification

October 31, 2026, Asia/Amman. Branch `t3/production-gnome-wallpaper`, based on
`ad0f430` (local wave5). This is a **partial delivery** of the production-component
request, not completion of 6D-M1. The spike's successful lifecycle measurements
are not inherited as production acceptance.

## Delivered surface

- Independent MIT OR Apache-2.0 extension source:
  `extensions/gnome/one-mk-wallpaper@one-mk.org`. No copied wallpaper-project code,
  decoder, network/media parser, test methods, arbitrary argv, unsafe-mode switch,
  FrameReady, image upload, or user-provided path channel in its D-Bus interface.
- `metadata.json` is the single exact-version qualification-list source, starting
  with **50.1**, the actual laboratory Shell. GNOME's own major-version validation
  remains on; the additional exact-version guard refuses all unlisted builds.
  There is no `unlock-dialog` session mode. Matching this list means only that
  the Shell build has laboratory evidence, **not** that playback/hardware passed.
- Exact section 4 wire XML lives in `bridge.xml`; Rust includes this same file.
  Hello `(u) -> (uua(iiiid))`, SetState `(u) -> ()`, Layout `(ua(iiiid))` and
  PauseHint `(u)` only; no properties. State/rate, nonzero serial, finite scale,
  checked edges, 16-monitor, 16384-side, 64-million-pixel and 4096-byte bounds.
  Empty output sets are valid. Layout changes coalesce for 100 ms.
- Spawned-PID plus UID authentication, one pending bounded credential lookup,
  first-Hello name binding, unicast signals, non-replaceable/nonqueued bus name,
  fail-closed name loss. Rust pins the bridge to Shell's PID **and** the inherited
  Wayland peer, subscribes before Hello, waits for the pause snapshot, and refuses
  owner changes/invalid signals. SetState duplicates coalesce in Rust.
- Source windows use connection ownership, DESKTOP before map, re-hide enforcement,
  one clipped clone per monitor, bounded retiring-window lifetime and replacement
  rate, geometry admission on damage, hidden-until-playing-and-damaged, stale
  watchdog, 2-second Hello and 5-second first-frame deadlines. All pause reasons
  hide first and terminate the renderer; uncover/unlock starts a fresh instance.
  Retry is 1/2/4 seconds, at most three starts per rolling minute; immediate unlock
  uses the **same** cap and never clears exhaustion. The latch survives extension
  rebase within a Shell session. Retry currently requires a fresh Shell session.
- CLI `one-mk wallpaper extension install|uninstall|status`, optional
  `install --renderer /absolute/path`. Sources are embedded in the CLI; installation
  requires an existing renderer binary. Only explicit install copies into
  `$XDG_DATA_HOME/gnome-shell/extensions/UUID`, or `$HOME/.local/share/...`.
  It never enables; it prints `gnome-extensions enable one-mk-wallpaper@one-mk.org`.
  Install refuses an occupied path. Uninstall refuses enabled, unmanaged, modified
  or extra-file trees rather than deleting user content. Pristine managed trees
  are fully removed. GNOME enabled-extension settings are never changed by it.
- Status distinguishes installed, enabled, qualified and restart-limit-reached.
  Unreachable Shell means **unknown**, not false. Doctor prints the extension state
  and the literal **restart limit reached** when the Shell-owned runtime snapshot
  says so. Snapshots carry Shell's unique owner so an old session is ignored.
  Import and `doctor --runtime` dispatch in `main.rs` remain unchanged.
- Release workflow has a separate **non-publishing** component artifact with
  `libexec/one-mk-wall`, extension source beneath `share/gnome-shell/extensions/`,
  licenses and SHA256SUMS. It is not attached to a GitHub release or auto-installed.
  `scripts/package-wallpaper-gnome.sh` reproduces packaging without installation.

## Explicitly unfinished — required before the request can be called complete

1. **Owner-authenticated daemon presentation-plan IPC and subscriptions are not
   implemented.** The production bridge's authorization stays false; no user-facing
   method or arbitrary argv override can turn it on. Enabling the extension alone
   therefore leaves it dormant, even if wallpaper policy is enabled.
2. **The production renderer is not connected to a qualified media/scene pipeline.**
   `one-mk-wall --present=gnome` explicitly refuses before opening a surface or
   decoder. `gnome-bridge` builds the production client library, not a fixture
   disguised as playback. The fixture/risk probe and conformance executable remain
   separate features/examples and are not packaged in the component artifact.
3. Source-window presentation, real damage/replacement, coverage, lock/unlock,
   power/battery/manual restrictions, display-power-off notifications, serial-wrap
   re-handshake, current/latest pending signal coalescing and failure re-probes
   still need production end-to-end integration/qualification. The current bridge
   rejects serial exhaustion until re-enable instead of transparently restarting
   the session; the client signal queue is bounded to 16 rather than a single
   latest slot. GPU-specific limits and physical/HiDPI import are not probed.
   Do not silently promote these partial implementations to section 4 compliance.
4. Teardown uses immediate force-exit and async reap, not the proposed cooperative
   SIGTERM/two-second escalation. The cap is observable, but a user-facing explicit
   Retry operation is not delivered. Cold-install recognition without logout/login
   is not qualified. No on-host system or extension validation setting was changed.
5. Whole-runtime licensing/sandbox/decoder package acceptance, Windows/macOS native
   host acceptance, physical display quality and power are still outstanding M1
   work. A dry-run artifact is not a shippable optional playback package.

## Reproduction and evidence boundary

Use only the checked-in headless supervisor, not a live session:

```sh
cargo build --offline -j 2 -p one-mk
cargo build --offline -j 2 -p one-mk-wall --features gnome-bridge --example bridge-conformance
python3 docs/spikes/gnome/qualify-component.py
```

The supervisor creates private HOME/XDG directories, a private session bus and
private system bus under the ignored worktree `target/gnome-component-qa` directory.
Only its test-only harness enables Eval. It copies the conformance executable to
the fixed renderer path in the **private** extension tree; it changes authorization
only through isolated Eval. None of those mechanisms exist in shipped source.
It exercises CLI install/uninstall with private XDG data, authenticated real Rust
calls, layout serials, unicast isolation, pause-bit rejection, state/version/type/
rate negatives, GC enable/disable, name squatting, the actual failed-spawn cap and
doctor output. Renderer children/session services and copied source trees are
cleaned in `finally`, including failed attempts. No surface/damage/decoder pass
is claimed by this test.

Normal-session warnings are compared, including stack records, against the
unchanged M1a-2 baseline; deliberate version-refusal errors are retained in a
separate negative-test log. The version negative removes the lab version from
the private extension's in-memory list: it tests refusal logic, not a second
actual GNOME version. Small final logs/results and validation are retained in
`component-evidence`; generated binaries and temporary trees are not evidence.

## Estimate and owner acceptance

Keep **16–26 engineer-weeks remaining for M1, 74–118 total**, low confidence.
This checkpoint does not justify reducing those estimates: the authorization,
production media integration and native/physical gates dominate. Re-estimate after
the integrated production renderer passes the same six stress runs and matrix as
the spike, plus new protocol/CLI and real power tests.

The owner should not enable this as a working wallpaper yet. After those gates are
implemented, test explicit cold install/enable/disable/uninstall and logout/login
on their exact GNOME build, DING/other extensions, lock/unlock, coverage with several
windows, display off, AC/battery/manual pause interactions, fractional scaling,
rotation/hotplug/replug, suspend, crash/hang cap and Retry, permissions, real frame
damage and GPU/power behavior. Keep the native static background untouched and
verify cleanup/no focus or input theft. No owner-machine result is inferred from
the private headless session.

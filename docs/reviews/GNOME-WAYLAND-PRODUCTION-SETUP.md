# GNOME Wayland: offline setup slice, production video NOT delivered

Date: November 2, 2026. Base: `origin/polish-glass-friction`.

## Outcome and scope

**The requested end-to-end GNOME Wayland video feature is incomplete.** This
change supplies consented extension setup, truthful setup status and offline
component packaging. It does not connect `live_wallpaper` to the GNOME renderer,
replace the laboratory authorization with production playback consent, or
qualify production video. Installing/enabling this component cannot make video
play in this build. The window says that explicitly, including when active.

No owner desktop was used for testing, and no extension was installed/enabled in
the owner's GNOME session. No real wallpaper was changed. No FFmpeg, libmpv or
x264 was added. No `cargo fmt` was run. All cargo invocations used at most four
jobs and the worktree's `target`; that build directory is removed before handoff.

## Implemented architecture

- `wallpaper-player` adds a backwards-compatible optional `Status.gnome` with
  installation/log-out/disabled/active/unsupported/unavailable/error/unmanaged
  stages. The normal video availability check still refuses Wayland. GNOME
  explains the missing production connection; other Wayland compositors are
  explicitly unsupported. Picture support is unchanged.
- `live_wallpaper` obtains a bounded asynchronous GNOME snapshot, at most once
  per two seconds when status is queried. Setup does not block the actor. Its
  local `GnomeExtension` command takes a separate path before wallpaper
  generation changes, worker cancellation or peer messaging. Setup cannot be
  deserialized as a peer `Wire` command. Video selection now checks availability
  before preparation/import can start a decoder on an unsupported platform.
- `wallpaper_extension` uses GNOME's supported extensions service,
  `org.gnome.Shell.Extensions` at `/org/gnome/Shell/Extensions`, and
  `GetExtensionInfo`, `ShellVersion`, `UserExtensionsEnabled`, `EnableExtension`.
  The prior service/object addressing was incorrect. Private GNOME 50.1
  introspection and the qualification tests verify the corrected address.
- The managed installer copies embedded bridge sources and a bundled renderer,
  stages atomically and records hashes. Occupied or differently loaded copies
  are not overwritten. Modified/extra/unmanaged files are preserved, and an
  active extension cannot be uninstalled by this command. No remote extension
  download, subprocess shell command, global enable-user-extensions setting,
  private Eval authorization or consent grant is used by product setup.
- `.deb` staging adds `/usr/lib/one-mk/one-mk-wall` and extension/XML/metadata/
  licences under `/usr/share/one-mk/gnome-extension/`. It never installs into
  GNOME's discovery directories or enables anything in a maintainer script.
  CI and Linux release builds include only the `gnome-bridge` renderer feature,
  not `m1-lab`. Linux tarballs include a sibling renderer and component payload.
  AppImage/Flatpak component integration is not implemented.

The release renderer remains dormant and `--present=gnome` exits with status 3.
The existing bridge still has default-off authorization, spawned PID/UID and
Wayland peer authentication, admission/resource bounds, child teardown and
restoration behavior. None of those guards was relaxed to make setup appear
to be video support. X11/macOS/Windows retain their existing player path.

## What the user sees

1. On qualified GNOME Wayland, open **Wallpaper**. The **GNOME video setup**
   section explains both the current setup stage and that production video is
   not connected. Opening it only reads status.
2. **Install GNOME extension** explicitly copies bundled files to the user's
   extensions folder. It downloads nothing and does not enable the extension.
3. The window says **Log out and back in once to finish**. It does not offer
   Enable while Shell has not discovered the new extension.
4. After login, **Enable GNOME extension** is a separate explicit action. The
   window polls for actual active state; it does not equate an accepted request
   with activation. If GNOME globally disables user extensions, the window
   explains that and does not change that global setting.
5. An active bridge still says video playback is not connected to the production
   service in this build. Selecting a video fails before preparation; pictures
   continue through the normal product path.

The qualification allowlist remains exactly Shell **50.1**. Other versions
are not silently treated as qualified. The private test confirms that a freshly
copied extension is absent from Shell's information before restart and is
discoverable, initially disabled, afterwards. This models a new login, not GDM's
complete real-login environment.

## Evidence from this worktree

The retained [qualification summary](../spikes/gnome/product-setup-evidence/result.json)
contains the observed setup/component results and the failed step-A outcome.
Raw command logs are retained in `/tmp/one-mk-gnome-*.log`; transient build and
harness output under `target` is intentionally deleted before handoff.

| Check | Result |
| --- | --- |
| `cargo clippy -j 4 --workspace --all-targets -- -D warnings` | Pass |
| `cargo test -j 4 --workspace` | 1,398 passed, 0 failed, 18 ignored across 49 result groups |
| Requested seven-package cross-target clippy, all targets | Pass for `aarch64-apple-darwin` and `x86_64-pc-windows-msvc` |
| `BIN_DIR=target/debug sh scripts/check-packaging.sh` | Pass; extracted `.deb` component files, dormant renderer refusal, no automatic GNOME installation; AppDir/Flatpak/Homebrew/artwork checks pass; Ruby syntax check skipped (Ruby absent) |
| `check-wallpaper-artifact.py --self-test` and full metadata/feature/helper/SBOM check | Pass; 102 dependency/closure records; decoder/import boundary retained |
| `test-wallpaper-release-gates.py` | Both daemon `wallpaper-m1-lab` and renderer `m1-lab` refused by optimized release checks |
| `test_m1_gate.py` / `test_presentation_cleanup.py` | 6 / 1 tests pass |
| `qualify-extension-setup.py` | Pass on private GNOME Shell 50.1; four check groups: install and enable refusals, offline install/log-out notice, restart/discovery/explicit enable, disable/managed uninstall; private background URI unchanged |
| `qualify-component.py` | Pass on private GNOME Shell 50.1; 100 authenticated paused-fixture child cycles, 0 new warning records, every child reaped; background URI unchanged; authentication/type/version/rate/name-squatting/restart-limit negatives pass |
| `qualify-m1-presentation.py` with separately copied debug lab binaries | Failed before authenticated presentation: historical step-A runner does not supply the playback configuration now required by the step-B renderer. It expects `no usable authority record`, but the renderer exits `playback unavailable: no laboratory playback configuration`. Finalizer reports no remaining lab processes. Not counted as a pass. |
| `qualify-m1-playback.py` / `qualify-m1-lifecycle.py` | Not run: no explicitly delegated decoder cgroup supplied; execution sandbox exposes a read-only cgroup mount. Audited decoder runtime/controller provisioning remains a production gap. No arbitrary cgroup or real desktop session was modified to bypass this. |
| `cargo deny` | Not run: cargo-deny absent locally; CI's pinned installation/check remains unchanged |

Socket-dependent Rust tests and headless Mutter initially stalled/failed inside
the network sandbox. They were rerun with permission outside that restriction,
using temporary HOME and removed desktop bus/display variables for Rust tests,
and the harness's private session/system buses for GNOME. The component harness
now uses short private socket paths and a real temporary monitor removal:
step C correctly ignores an unchanged `monitors-changed` notification. Its
diagnostic assertion uses the normal `extension status`, not obsolete `doctor`.

The 100 cycles above are **paused fixtures**, not video or restoration after
visible playback. No production CPU/GPU/hardware-decode numbers were measured.
Historical step B/C resource measurements are not new production evidence.

## Required follow-up before video can ship

1. Wire an authenticated normal-daemon authority into `live_wallpaper`, with
   explicit owner playback consent and revision/epoch revocation. Installing or
   enabling the extension is not a peer or renderer authorization grant.
2. Adapt the production plan's Fit, shared start time and measured clock offset
   to the admitted presentation wire and renderer. Preserve the frozen canvas,
   absent-computer reserved slices, monotonic sequence/generation and imported
   media hash checks. The current lab payload cannot represent all this.
3. Provision an audited decoding runtime and an explicitly delegated bounded
   cgroup in the normal launch path, and ship/verify the required offline
   runtime files. Do not replace the lab sandbox with the unsandboxed existing
   helper or remove limits. Certify hardware decode separately; the laboratory
   VP9/AV1 software allowlist is not hardware-decode acceptance.
4. Drive pause/Off/Fit, battery/lock/sleep, plan replacement and multi-computer
   clock synchronization through that authority. Report actual admitted/visible
   playback and failure status back to peers, not mere extension activation.
5. Reconcile the step-A runner with the step-B configuration contract, rerun
   real playback and lifecycle qualifications, measure paused/covered CPU and
   hardware decode, then test the full native GNOME/GDM session.

## Owner's short manual checklist (setup only)

- Install an updated `.deb`, open Wallpaper and confirm the video limitation is
  visible. Merely opening the page must not create or enable an extension.
- Choose **Install GNOME extension**. Confirm the log-out/back-in notice and
  that the current wallpaper remains unchanged. Log out and back in once.
- Open Wallpaper and choose **Enable GNOME extension**. Confirm it becomes
  active, without changing the wallpaper or claiming video is playing.
- Select a video: expect an explicit unavailable message, not playback. Test a
  picture and Off through the existing path. Use GNOME Extensions to disable
  the dormant bridge when finished; modified user files must not be overwritten.

Real video across this laptop's monitors, Fit, peer clock alignment, lock and
battery pause/restoration, covered CPU and hardware decode remain **unverified
and unimplemented in the product**. They are not acceptance checks this branch
can honestly pass.

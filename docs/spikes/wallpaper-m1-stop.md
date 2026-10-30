# 6D-M1a: configure warning resolved in a diagnostic; lifecycle still STOPPED

Date: 2026-10-30. Branch: `t3/wallpaper-mutter-presentation`.
Baseline: `76534af` (`Merge wallpaper M1 probe and stop evidence into wave5`).
Owner authorized all milestones. **Neither M1a qualification nor M1 delivery
is complete. Do not deploy this binary into the owner's desktop.**

## Decision and primary evidence

Retain the own-xdg-surface/appsink path as the next presentation experiment,
not a shipping renderer. It removes the original initial-content warning,
but the strict full-session gate still fails: Mutter stack/allocation warnings,
retained managed windows, a late watchdog, and no bounded restart cascade.
Do not suppress these warnings or mark qualification complete. No decoder
sandbox, untrusted importer, production bridge or new receiver authority is added.

The [original M1 report](gnome/m1-presentation-original.json) is unchanged.
New evidence: [final matrix](gnome/m1a-evidence/matrix.json),
[complete final log](gnome/m1a-evidence/shell.txt),
[comparison runs](gnome/m1a-evidence/comparison.json),
[wire trace](gnome/m1a-evidence/configure-trace.txt),
[45-capture contact sheet](gnome/m1a-evidence/contact-sheet.webp).
Individual captures are downscaled lossless WebP; JSON retains original PNG
dimensions, SHA-256, scheduled/request/completion times and sampled pixels.
Generated full-size PNGs, fixture, plugins and session files are removed.

## Root cause and candidate paths

Host GNOME/Mutter 50.1, GStreamer 1.28.2. The extension's DESKTOP window and a
NORMAL-window control both reproduce the warning. DESKTOP conversion is
**not necessary** for this initial-content warning.

The wire trace shows `xdg_surface.configure(1)`, `ack_configure(1)`, then
multiple **empty root commits**, followed by the first video-subsurface and
root-border buffer commits. There really is an ack on the wire; saying
"waylandsink never acknowledges configure" would be incorrect.

Matching upstream sources inspected during diagnosis (not vendored):

- GStreamer tag `1.28.2`, `gst-libs/gst/wayland/gstwlwindow.c`,
  `handle_xdg_surface_configure` (line 185): acknowledges, marks configured,
  calls `gst_wl_window_update_geometry` (line 1006), which commits root state
  before the first root buffer is attached.
- Mutter tag `50.1`, `src/wayland/meta-wayland-xdg-shell.c`,
  `meta_wayland_xdg_toplevel_post_apply_state` (around line 1036): window
  finish-move/resize processing needs a surface buffer.
- Mutter tag `50.1`, `src/wayland/meta-window-wayland.c`,
  `acquire_acked_configuration` (around line 1360): first nonempty state has
  no pending ack, pending configurations exist, and no last-acked configuration
  exists, so it warns and works around the missing state.
- Stable `xdg-shell.xml`: initial empty commit precedes configure; acknowledge
  before content commit; never acknowledge a serial twice. This identifies
  the installed Mutter/GStreamer state interaction, not a conclusive protocol
  conformance verdict about every compositor.

| Candidate | Actual experiment | Decision |
|---|---|---|
| `waylandsink` | Warning reproduced with DESKTOP and NORMAL controls | Reject standalone path on measured stack |
| `gtk4paintablesink` in GtkWindow | [No such plugin](gnome/m1a-evidence/gtk4-availability.txt) in this lab; no GtkWindow playback test possible | **Not runtime-tested**; provision privately for subsequent comparison |
| Rust `wayland-client` xdg root + GStreamer appsink | Real renderer, one VP9 decoder; initial empty commit → configure → ack immediately with first nonempty root commit | Original warning absent; retain diagnostic only; complete log still fails |

The extension owns the connection through `Meta.WaylandClient`, identifies
windows using `owns_window`, sets DESKTOP before initial map/configure, hides
the source actor, and clones above native wallpaper/below DING. NORMAL windows
are not a shipping fallback: work-area constraints break the spanning extent.
The Rust prototype uses an empty input region, two release-tracked memfd
buffers, current-surface configure serials and frame callbacks. Layout changes
destroy the old role before replacement; this is not seamless hot-plug.

New dependencies `wayland-client 0.31.15` and `wayland-protocols 0.32.13`
declare MIT. GStreamer/appsink and GLib/GIO remain dynamically linked LGPL
laboratory dependencies; no GPL upstream implementation is copied into this
repo. Selected plugins and forbidden-library string checks are still **not**
a release SBOM/licence audit. Fixed 3840×1080 BGRx fixture, CPU nearest-neighbor
scaling, copies and dynamic FFI are unqualified: no GPU zero-copy, physical
HiDPI, hostile-media security or production unicast/PID-binding claim follows.
The feature remains default-off and Linux-only.

## Final measured lifecycle

Run began `2026-10-30T18:47:01.029908+00:00`, optimized real Rust binary,
three headless virtual 1920×1080 outputs. First-frame figures mean **supervisor
observation of Shell damage**, including IPC/polling, not panel scanout.
CPU is process tick delta over three seconds, percent of **one core**; short
windows/host contention limit interpretation. Shell PID is found only among
this run's private leader descendants, not by a reused global runtime path.

| Case | Measurement | Qualification |
|---|---|---|
| Startup, three outputs | First damage **159.947 ms**; 5760×1080 buffer, three clones; DING rather than source has focus | Narrow startup pass |
| Scale 1, 1/2/3 outputs | 1920/3840/5760×1080 buffers, 1/2/3 clones | Logical extent observed |
| Scale 2, 1/2/3 outputs | 960/1920/2880×540 buffers, 1/2/3 clones | Not physical sharpness qualification |
| Fractional scale 1.25, three outputs | 4608×864 buffer, three clones | Rotation/mixed scales not tested |
| Unplug | Video in first capture, request 82.41 ms / completion 176.80 ms | No static gap sampled; not proof of zero gap |
| Replug | Static in first capture (89.48–347.95 ms request/completion); video next (425.32–530.64 ms) | Visible still gap; exact duration not recoverable from IPC captures |
| Overview pause, two outputs | Renderer **1.00%**, Shell **1.33%** | Actual PAUSED pipeline; residual polling is not zero-work |
| All-covered, two fullscreen windows | `paused:true`, `reason:covered`; renderer **0.67%**, Shell **0.67%** | Coverage actually triggered |
| Playing, three outputs | Renderer **60.99%**, Shell **9.67%** | Software copies/scale; not an efficiency pass |
| Hang (`SIGSTOP`) | Static fallback observed **3732.27 ms**; Shell event stale 3617 ms | **Fails 3 s**; one-second watchdog sampling overshoots |
| Live `SIGKILL` | Fresh damage/two clones verified after SIGCONT, then killed; static first capture completed 255.55 ms through last at 2925.54 ms | Fallback observed; manual next start **185.579 ms** to damage, not automatic recovery |
| Lock/unlock | `locked:true`, mode `unlock-dialog`; clones removed; lock frames through 2093.18 ms; static desktop unlock captures completed 370.32–2926.48 ms | Manual restart after burst **170.501 ms**; not an automatic-unlock gap measurement |
| Restart cascade/cap | 6B has no automatic bounded restart implementation | **Not qualified**; manual starts are not a cap test |
| Disable/enable ×100 | 100 old renderer PIDs exited; damage min/median/max **154.560 / 164.570 / 279.350 ms** | Managed wallpaper-window count **1–24**, not invariant one |
| Final leak census | No named one-mk actors/stage clones, bridge absent; **four managed wallpaper windows remain after one second** | **Fails window-leak gate**, not dismissed as animation |
| DING | Enabled state 1; `Desktop Icons 1/2` in final managed-window list | Runs; private missing GVfs trash/metadata support still logged |
| Complete Shell log including teardown | **31** WARNING/CRITICAL/Gjs-error matching lines; **zero** original initial-content warnings | **Fails strict gate**, no allowlist |
| Process/session cleanup | Children reaped; private media/files/socket removed; host-namespace exact-runtime scan found **zero processes** | No remaining private processes; does not erase retained-window failure |

Screenshot targets are 0/250/…/2000 ms. Screenshot/snapshot IPC sometimes
exceeds 250 ms: kill/unlock bursts finish about 2.9 s. JSON retains actual
request/completion times, not fictitious exact intervals. Pixel (500,700)
distinguishes this fixture's black video background from blue wallpaper;
black lock-screen pixels are **not** called video. These are coarse visible
observations, not presentation-time feedback or precise gap bounds.

Earlier CPU values sampled a stale private Shell PID and are excluded. Earlier
kill-9 captures taken while already stale are not the live test above. Early
baseline cleanup permission failures left private processes; they were later
matched by UID/exact private runtime and terminated. No live owner Shell or
one-mk was touched. Earlier DING failures coincided with UID inotify exhaustion;
DING runs in the final session after cleanup, without claiming all GVfs works.

## Strict harness and reproduction

`m1-risk-probe.py` fails on **all** WARNING/CRITICAL/Gjs-error/JS-error lines,
including startup services and teardown; `test_m1_gate.py` tests classification
without an allowlist. Matrix invariants independently fail late watchdog,
retained windows and absent restart cap. Final exit **1**, explicit failures
in JSON. GTK comparison is an explicitly missing test, never a mock pass.

```sh
env -i HOME=/home/samer PATH=/home/samer/.cargo/bin:/usr/bin:/bin \
  cargo build --offline -j 2 --profile bench-opt -p one-mk-wall \
  --features gnome-risk-probe
env -i HOME=/home/samer PATH=/usr/bin:/bin PYTHONDONTWRITEBYTECODE=1 \
  python3 docs/spikes/gnome/m1-risk-probe.py --matrix --sink own \
  --binary target/bench-opt/one-mk-wall --label matrix-qualified-attempt
# Original sink comparison; add --normal-window for the type control:
env -i HOME=/home/samer PATH=/usr/bin:/bin PYTHONDONTWRITEBYTECODE=1 \
  python3 docs/spikes/gnome/m1-risk-probe.py --sink wayland --trace \
  --label desktop-trace --binary target/bench-opt/one-mk-wall
```

Only the existing isolated harness is used: clean environments, private session
and system buses, HOME/XDG, memory settings, headless/no-X11 Mutter and private
extension copies. Restricted CLI environments may need private-socket approval.
Never target the live session or owner's extension directory. Generated fixture
hash/private-peer guards are laboratory guardrails, **not** a hostile-media sandbox.

## Next-best design and re-estimate

**Stop qualification here: no warning-free complete path demonstrated.** This
does not prove such a path impossible. Next-best design is the owned xdg root
with acknowledged first buffer, still one decoded scene/N Shell clones, with
native static wallpaper as default/failure state. Before product work resumes:

1. Diagnose retained managed windows and `meta_window_set_stack_position_no_sync`
   assertion with Meta.WaylandClient/window/actor lifetimes; add minimal own-root
   close/destroy control, distinguish Shell stack bugs from extension operations.
   Do not hide/delete ghosts to fake a passing census.
2. Privately provision `gtk4paintablesink`; compare real GtkWindow with the same
   connection-owned DESKTOP adoption, licence and one-decoder checks. Toolkit
   lifecycle management remains a candidate, not evidence.
3. Give the private lab required services/settings **without** attaching to the
   live system bus; resolve rather than filter AccountsService/logind,
   experimental-setting, portal and Clutter warnings. Fake GDM alone is not a
   representative lock/session environment.
4. Implement/test the design's bounded restart budget and 3 s damage deadline;
   rerun lock/unlock cascades, live crash, captures, leak census and 100 cycles.
   Investigate residual paused polling, then GPU copies/physical HiDPI/power.
5. Only after warning-free lifecycle qualify hostile-media isolation, native
   hosts, LGPL replacement/artifacts and all physical design §11 acceptance.

Planning allowance: **M1 remaining 16–26 engineer-weeks**, versus 14–22 after
the original stop. M2 10–16, M3 16–24, M4 10–16, M5 14–22, reserve 8–14:
**74–118 remaining engineer-weeks total**, not elapsed calendar dates. Extra
2–4 weeks cover lifecycle/toolkit/lab uncertainty, not a promise of a small fix.
Frozen receiver authority, default-off and physical acceptance remain unchanged.

## Validation

Clean environments, offline Cargo with `-j 2`; no formatter. [Transcript](gnome/m1a-evidence/validation.txt).

| Check | Result |
|---|---|
| Feature-enabled `one-mk-wall` tests | Pass: 2 unit + 3 real-binary tests |
| Default `one-mk-wall` / `one-mk-wall-sync` | Pass: 3 binary + 12 sync integration tests |
| Workspace clippy, all targets, `-D warnings` | Pass |
| Feature-enabled wallpaper clippy, all targets, `-D warnings` | Pass |
| Windows MSVC / Apple ARM cross clippy | Pass, including wallpaper crates and enabled risk feature; native hosts untested |
| Python strict-log gate tests | Pass: 2 tests |
| Real final private Mutter matrix | **FAIL**, as detailed above; not green acceptance |

Cross targets: `x86_64-pc-windows-msvc`, `aarch64-apple-darwin`; packages
`one-mk-input`, `one-mk-proto`, `one-mk-ctl`, `one-mk-app`, `one-mk-tray`,
`one-mk-wall`, `one-mk-wall-sync`. Earlier full-workspace runtime tests had
unrelated daemon/environment failures (see prior revision); not rerun or
claimed green here. No physical multi-host sync, sleep, 24 h soak or independent
security/legal review is supplied by this work.

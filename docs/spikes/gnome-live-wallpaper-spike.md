# GNOME 50 live wallpaper bridge: feasibility spike

Spike only, not product code · 30 October 2026 · base `9643605` (`main`) ·
GNOME Shell 50.1 / Mutter 18 typelibs on the owner's Ubuntu box · design:
`docs/WALLPAPER-DESIGN.md` on `t3/research-live-wallpaper-design` (sections
3, 4, 7, 9).

Every experiment ran in a disposable **headless** GNOME Shell that I started
myself, with private buses and private directories. Nothing was installed,
enabled or changed in the owner's live session (proof in
[Isolation](#isolation-and-the-live-session)).

## Verdict: GO WITH LIMITS

**Yes, it works.** A small GNOME Shell extension can show a one-mk
renderer's moving picture *behind every window and below the desktop
icons*, without any video decoding inside the Shell. The trick: the
extension starts the renderer itself as an ordinary Wayland app, makes the
app's own window invisible, and paints a live copy (a `Clutter.Clone`) of it
in the Shell's wallpaper layer, cut into one piece per monitor. In the test
session it ran at the full 60 frames per second, at about 3–10 % of one CPU
core for the Shell. One renderer window fed two monitors, and the picture
ran continuously across the seam.

The other approaches failed. Handing the Shell GPU buffers directly
(dmabuf) is not possible from extension JavaScript in GNOME 50. Copying
frames into the Shell in software works, but it costs 10–15 ms of the
Shell's main thread per 1080p frame, so the whole desktop would stutter.

**The limits the owner would actually notice:**

1. **Activities Overview shows the normal still wallpaper**, not the moving
   one. The live layer simply isn't drawn there, and the renderer pauses.
   Recommended mitigation: optionally set GNOME's still wallpaper to a
   poster frame of the current scene, so the Overview and lock screen
   match.
2. **Every screen lock restarts the renderer.** When the screen locks, GNOME
   turns off DING, and turning off any earlier extension makes GNOME
   switch every later extension off and on again. So our extension gets
   restarted and the renderer is killed. That happens even if our extension
   asks to stay on while locked. After unlocking, the still wallpaper shows
   briefly until the renderer is back: 0.27 s with the test client;
   a real GStreamer renderer is **UNVERIFIED**. The same restart happens
   whenever the owner switches off any extension that was enabled before
   ours.
3. **Adding or removing a monitor briefly shows the still wallpaper** while
   the renderer opens a new window at the new size. Mutter cannot resize
   the hidden window after it appears. The new window was adopted 7–13 ms
   after the monitor change; the visible gap was **not timed**.
4. **The power saving has to be done by the extension.** Mutter keeps
   drawing the wallpaper copy even under a fullscreen window, so a renderer
   that ignored the "pause" message kept running at 60 fps. The extension
   now hides the copy on any monitor that a maximized or fullscreen window
   covers, which stops the frames (≈0 fps) even for a misbehaving
   renderer. Coverage by *several* windows together is not detected yet.
5. **It relies on Shell internals**, mainly the private
   `Main.layoutManager._backgroundGroup`. Every GNOME release needs this
   spike re-run, and the extension must declare only the tested versions.
   Version validation behaved normally: a copy declaring only `"49"` was
   refused as `OUT_OF_DATE`.
6. **The owner has to install and enable an extension once**, through
   normal GNOME tooling. On Wayland, a newly installed extension normally
   needs a log out/in before it loads (**UNVERIFIED here**: every test
   session was started fresh).

What would turn this into NO-GO: a future GNOME removing or renaming
`Meta.WaylandClient.new_subprocess`/`owns_window`, `Meta.Window.set_type`,
or the background group; or real-hardware tests (below) showing that
cloned windows break with HiDPI/fractional scaling or on the NVIDIA GPU.
In that case the fallback is a still poster frame as the normal GNOME
wallpaper. A "pinned-below" normal window is **not** an acceptable
alternative on Mutter/Wayland. An ordinary client can't keep itself below
DING's icons or out of Alt-Tab and the Overview; my first run (before
setting the window type) showed a normal-type renderer window even took
keyboard focus while invisible.

| Step | Result |
|---|---|
| 1. Coloured actor below windows and DING icons | **GO**, screenshot + actor tree |
| 2a. Renderer window, hidden, cloned into the background | **GO**: 60 fps, Shell 3–10 % of a core |
| 2b. Shared dmabuf/GL texture imported by the extension | **NO-GO** in JS (no import API) |
| 2c. Software frames into `St.ImageContent` | Works, **NO-GO as a product path** (10–15 ms/frame on the Shell main thread) |
| 3. Hot-plug | **GO with limit** (renderer must open a new window per layout) |
| 3. Disable, client death, client hang | **GO** (clean; static fallback ≤ 3.1 s for a hang) |
| 3. Lock screen | **GO with limit** (covered; renderer restarted per lock) |
| 3. Workspaces / Overview | Workspaces **GO**; Overview shows the static wallpaper (**limit**) |
| 3. Coverage → pause signal over D-Bus | **GO** (and enforced by hiding clones) |
| 3. Version validation | **GO** (normal mechanism, never disabled) |
| 4. Bridge security | **GO** with the bounded protocol below |

## Isolation and the live session

`docs/spikes/gnome/isolated-session.sh start N` runs, under `setsid` and
`env -i` (so no `WAYLAND_DISPLAY`, `DISPLAY` or `DBUS_*` of the desktop leaks
in):

- `dbus-run-session` (private session bus);
- a **private system bus** (`dbus-daemon` with a minimal config, no service
  activation). This was necessary: reading `misc/loginManager.js` and
  `ui/screenShield.js` in the 50 sources shows that a stock Shell would
  attach to the owner's logind session via the real system bus (lock hint
  `SetLockedHint`, sleep inhibitor, polkit agent) and would call GDM's
  `RegisterDisplay`. The log confirms the private bus caught it:
  `Error registering display with GDM: … ServiceUnknown`;
- `HOME`, `XDG_RUNTIME_DIR`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME`,
  `XDG_CACHE_HOME`, `XDG_STATE_HOME` under `/tmp/omk-gs-spike`;
  `GSETTINGS_BACKEND=memory`;
- `gnome-shell --wayland --headless --no-x11 --wayland-display spike-wl-0
  --virtual-monitor 1920x1080` (twice for two monitors).

GNOME 50's `gnome-shell` has no `--unsafe-mode` flag, so the test-only
extension `spike-harness@one-mk.test` sets `global.context.unsafe_mode`
*inside the isolated Shell* to allow `Eval`/`Screenshot`. Extensions were
enabled only through `org.gnome.Shell.Extensions.EnableExtension` on the
isolated bus, from the isolated `XDG_DATA_HOME`.

Evidence (`transcripts/01-isolation.txt`, `00-live-session-before-after.txt`):
the isolated bus has its own `org.gnome.Shell`, and
`org.freedesktop.systemd1`, `org.gnome.SettingsDaemon.Power` and
`org.gnome.SessionManager` are **absent** there but present on the live
bus. Read-only snapshots of the live session at 18:36, 19:09 and after the
last run all match:

| Live session item | Before | After |
|---|---|---|
| `org.gnome.shell enabled-extensions` | snapd-search, web-search, ding, ubuntu-dock, tiling-assistant | identical |
| `disable-extension-version-validation` | `false` | `false` |
| background `picture-uri` | Snowy_Ubuntu_Light.webp | identical |
| live `gnome-shell` PID (started 21 Oct 16:06) | 52867 | 52867 |
| owner's `one-mk run` PID (started 29 Oct 02:40) | 759937 | 759937 |
| owner's `one-mk-app` | not running | not running |
| `~/.local/share/gnome-shell/extensions` | absent | absent |
| `~/.config/dconf/user` mtime | 18:27:50 | 18:27:50 |
| `org.onemk.WallpaperBridgeSpike1` on live bus | — | not owned |

Cleanup: `isolated-session.sh stop` kills the session's process group plus
any process whose environment points at the isolated runtime dir, then
deletes `/tmp/omk-gs-spike`. A controlled start/stop showed it creates
exactly one `/tmp/dbus-*` socket and removes it. Many stale `/tmp/dbus-*`
sockets exist on the box; 120 predate this work and belong to other
sessions. No spike process remains.

## Step 1: layering below windows and DING

Source facts (GNOME 50 `ui/layout.js`, `ui/background.js`; read-only
`gresource extract` of `/usr/lib/gnome-shell/libshell-18.so`):

- `layout.js:322-324`: `_backgroundGroup = new Meta.BackgroundGroup()` is
  added to `global.window_group` and pushed below all siblings.
- `background.js` `_createBackgroundActor()`: every Shell
  `Meta.BackgroundActor` is added and then
  `set_child_below_sibling(actor, null)`, so the static wallpaper always
  ends up at the bottom *inside* that group. A container we append stays
  above the static wallpaper and below every window actor, even after
  background changes or monitor reconfiguration.
- DING (read only; GPL-3, nothing copied) shows its icons in ordinary
  Wayland windows that it spawns through `Meta.WaylandClient`, sets to
  `Meta.WindowType.DESKTOP` and keeps lowered. They are window actors in
  `global.window_group`, hence above the background group.

Actor tree from the running isolated Shell (`js/stack.js`):

```
Meta_BackgroundGroup [Shell _backgroundGroup: Meta_BackgroundActor, one-mk-wallpaper-layer]
MetaWindowActorWayland "Desktop Icons 1" type=1 (DESKTOP)
MetaWindowActorWayland "Test window over the layer" type=0 (NORMAL)
```

DING ran fine in the isolated session (it found `~/Desktop/spike-icon.txt`).
`shots/step1-color-below-ding-and-window.png` shows the magenta layer under
the Home and spike-icon.txt icons and under a normal GTK4 window.

## Step 2: getting frames in without a decoder in the Shell

### (a) Hidden renderer window + `Clutter.Clone`: works

Mechanism, all in `ext/wallpaper-spike@one-mk.test/extension.js`:

1. `Meta.WaylandClient.new_subprocess(global.context, launcher, argv)`
   spawns the renderer on a private Wayland connection, and
   `wc.owns_window(win)` identifies its windows. No title or app-id
   matching is involved; an impostor with the same title and app-id was
   left alone (see step 4).
2. On `window-created`: `hide_from_window_list()`,
   `set_type(Meta.WindowType.DESKTOP)` (before the first configure; this
   exempts it from the work-area constraint and keeps it out of focus),
   size it to the canvas, then on a `BEFORE_REDRAW` later `stick()` +
   `lower()`, and `actor.hide()`.
3. Per monitor: a clipped container in our layer holding
   `new Clutter.Clone({source: windowActor})` offset by
   `bufferRect − monitorRect`.

Results (`transcripts/02`, `05`, `13`, `14`, `04`):

- Not in Alt-Tab (`get_tab_list(NORMAL_ALL)`), not in running apps
  (`Shell.AppSystem.get_running()`), not in the Overview or workspace
  thumbnails: they filter on `skip_taskbar`, which is `true` (`altTab.js:56`,
  `workspace.js:1331`, `workspaceThumbnail.js:461`). On all workspaces.
- Pointer: `stage.get_actor_at_pos` at an empty desktop spot after a
  virtual click returns DING's surface; focus stays with DING. With the
  NORMAL window type (first run), the invisible renderer **did** take
  keyboard focus and Mutter held it under the top bar (1920×1048), so
  `set_type(DESKTOP)` is required.
- `shots/step2a-clone-of-hidden-renderer-below-ding.png`: live pattern
  under the DING icons; `shots/step2-two-monitors-one-source.png`: one
  3840×1080 window feeding two cropped clones, continuous at x=1920.

Frame-rate and CPU. These are the renderer's own frame counter, and
`utime+stime` of the isolated `gnome-shell` and renderer over wall time,
with one core = 100 %. Settings: 1920×1080@60 virtual monitor (Mutter
created GBM renderers on both the Intel and NVIDIA render nodes; which
GPU composited is **UNVERIFIED**). The renderer is a Python GTK4 stand-in
that redraws on every frame callback. The host was shared with other
sessions' builds (load 2.6–4.5), so treat the CPU figures as ranges:

| Condition | Renderer fps | Shell CPU | Renderer CPU |
|---|---|---|---|
| A: source hidden + clone (6 runs) | 59.8–60.0 | 3.1–10.2 % | 5.5–15.1 % |
| B: source opacity 0 + clone | 59.1 | 3.8–13.0 % | 6.7–18.8 % |
| C: source visible at the bottom + clone | 59.9–60.0 | 5.4–6.8 % | 11.1–13.1 % |
| D: source hidden, **no** clone (control) | 0.0–0.1 | 0.1–0.2 % | 0.0–0.1 % |
| 2 monitors, 3840×1080 source, 2 clones | 59.8–60.0 | 5.8–10.4 % | 11.0–18.2 % |
| Maximized/fullscreen window on top, clone left visible, renderer ignores hint | 60.0 | 3.8–6.4 % | 6.4–13.7 % |
| Same, but the extension hides the covered clone | 0.0–0.1 | 1.1 % | 0.0 % |
| Activities Overview open | ≈0.3 | 0.2 % | 0.0 % |

Key answers:

- **Does the hidden window keep rendering?** Yes, at full rate, *because*
  the clone is painted. With no clone painted (D), Mutter stops its frame
  callbacks and the renderer stalls at ≈0 fps. Hiding the clone is
  therefore a reliable compositor-enforced pause.
- **Does Mutter throttle a covered wallpaper?** No (row "clone left
  visible"). Power saving must come from the extension. `hide` (A) is
  preferred over `opacity 0` (B): opacity-0 actors can still be picked for
  input in Clutter (**UNVERIFIED** here, not needed).

### (b) Shared dmabuf / GL texture imported by the extension: NO-GO

Checked against the GNOME 50 typelibs offline
(`gjs` + `GIRepository` on `/usr/lib/x86_64-linux-gnu/mutter-18`):
`Cogl.DmaBufHandle` offers only `get_fd/mmap/munmap/sync_*` (export side),
and `Cogl.Texture2D` only `new_from_bitmap/new_from_data/new_with_size/
new_with_format`. There is no EGLImage or dmabuf import. Clutter only has
`TextureContent.new_from_texture`. Importing a renderer's dmabuf would need
native code loaded into the Shell, which the design forbids. The window
clone in (a) *is* the zero-copy GPU path: Mutter already imports the
client's dmabuf/shm buffer as a texture, and the clone samples it.

### (c) Software frames into the Shell: works, NO-GO as a product path

The spawned feeder writes ready-made 1920×1080 RGBA frames to a fixed file
in the runtime dir and calls `FrameReady` (caller-PID-checked). The
extension reads the file (exact size check) and calls
`St.ImageContent.set_bytes` (`transcripts/12-software-path.txt`):

| Requested rate | Shell CPU | Mean Shell main-thread time per frame |
|---|---|---|
| idle baseline | 0.0 % | — |
| 15 fps | 20.0 % | 14.7 ms |
| 30 fps | 36.5 % | 10.3 ms |

10–15 ms per frame on the thread that draws the whole desktop (a 60 Hz
frame is 16.7 ms) means visible desktop stutter. It's acceptable only as a
still-image fallback (one frame), never for video.

## Step 3: behaviour that decides viability

All via `transcripts/03`–`11`, `15`.

- **Monitor hot-plug** (`06-hotplug.txt`, via the isolated bus's
  `org.gnome.Mutter.DisplayConfig.ApplyMonitorsConfig`, method 1 =
  temporary). First attempt **failed**: after unplug Mutter shrank the
  DESKTOP-type window to the work area (1920×1048), and
  `allows_resize()` is `false` for that type, so it never grew back on
  replug and monitor 2 stayed empty. As NORMAL type it is held to the work
  area. Fix: the extension emits `Layout`, the renderer opens a *new*
  toplevel at the new canvas size, and the extension adopts the newest
  owned window. Re-run: 2→1→2 monitors, windows 1920×1080 then 3840×1080,
  clones re-cropped, 59.2–60 fps after each change.
- **Extension disabled** (`08-disable-cleanup.txt`): at the first sample
  (≤ 0.5 s) our layer, both clones and the bus name are gone. The
  renderer process had exited (SIGTERM) by the +3 s check, and its
  already-hidden window was unmanaged within ~2 s. Three rapid enable/disable cycles leaked nothing.
  The Shell log has 0 lines mentioning the extension. An earlier run *did*
  log `Gjs-CRITICAL … already disposed` when the client was killed; that
  was fixed by using the Shell's destroy-aware
  `connectObject`/`disconnectObject`, plus deferring `lower()` to a later
  (it had tripped `meta_window_set_stack_position_no_sync` assertions).
- **Renderer dies** (SIGKILL, `07-hang-and-death.txt`): `unmanaged` →
  clones removed → the Shell's static wallpaper shows (screenshot at +1 s:
  `shots/step3-after-kill.png`). The process exit was noticed 42 ms later
  via `wait_async`.
- **Renderer hangs** (SIGSTOP): the last frame stays up to 3 s, then the
  damage watchdog drops the clones and shows the static wallpaper (3068 ms
  measured; `shots/step3-hang-6s.png`). On SIGCONT the next commit
  restored it. A bounded re-probe (5 s doubling to 60 s, failure state
  only) covers renderers that only recover once painted again. The
  watchdog is a 1 Hz timer that runs only while a renderer is adopted;
  `check_alive` (Mutter ping) is called on stall. No `is-alive` change was
  seen within 6 s (**UNVERIFIED** timeout). A first version deadlocked
  (no clone → no frames → never "fresh"); that's why the re-probe exists.
- **Lock screen** (`09-lock-screen.txt`; the Shell only creates its lock
  screen if GDM answers on the system bus, so a fake GDM `Version` was
  put on the *private* system bus). Locking covers everything
  (`shots/step3-locked-shield-covers-everything.png`, black shield). The
  extension is disabled on lock, and so is the renderer. With
  `"session-modes": ["user","unlock-dialog"]` it was **still**
  disabled+re-enabled because of DING; the Shell's
  `_callExtensionDisable` (`extensionSystem.js` ~190–235) "rebases" all
  extensions after the one being disabled. The final extension hides its
  layer and sends `PauseHint(true, locked)` when it is alive during lock,
  and starts with the layer hidden if (re-)enabled while locked.
- **Workspace switch** (`04`): the layer stays fixed like the normal
  wallpaper while windows slide; the renderer kept 59 fps on an empty
  workspace (`shots/step3-workspace2-live-layer.png`).
- **Overview**: shows the Shell's static wallpaper in the workspace
  previews (`shots/step3-overview-shows-static-wallpaper.png`); the live
  layer is not painted there. The extension now reports
  `PauseHint(true, overview)`; after 8 s in the Overview there was no false
  stale fallback, and 59.8 fps within 1 s of leaving.
- **Fullscreen/maximized coverage → renderer** (`03-coverage-pause.txt`):
  one D-Bus signal path proven end to end. A maximized or fullscreen
  normal window → `restacked`/`in-fullscreen-changed` → 100 ms debounce →
  `PauseHint(true, "covered")` on `org.onemk.WallpaperBridgeSpike1` → the
  renderer logged `hint paused=1 reason=covered` and stopped (0 fps). On
  close: `paused=0`, back to 60 fps. Coverage rule in the spike: one
  visible NORMAL window on the active workspace that is fullscreen on, or
  whose frame covers, the monitor (64 px top-bar allowance). The union of
  several windows is **not implemented**.
- **Version validation** (`10-version-validation.txt`):
  `disable-extension-version-validation` was never touched. The spike
  declares `"shell-version": ["50"]` and loads as `ACTIVE (1)`; a copy
  declaring only `["49"]` was refused as `OUT_OF_DATE (4)`.

## Step 4: security of the bridge

What was tested (`11-security-probes.txt`):

| Attempt by another local process | Result |
|---|---|
| Call `Hello` / `FrameReady` | `AccessDenied: not our renderer` (sender PID ≠ spawned PID) |
| Take the bridge's bus name while the Shell holds it | `RequestName` → `EXISTS` (not replaceable) |
| Emit a forged `PauseHint` from its own connection | Renderer ignored it (subscribed by well-known name, so GDBus matches the name's current owner) |
| Open a window with the renderer's exact title and app-id | Not adopted; stays an ordinary visible window (not `owns_window`) |
| The real renderer's `Hello` | `(1, [(0,0,1920,1080),(1920,0,1920,1080)])` |

### Proposed product interface (minimal, bounded)

Name `org.onemk.WallpaperBridge1`, path `/org/onemk/WallpaperBridge1`, owned
by the extension without `ALLOW_REPLACEMENT`. No properties.

```
method Hello(u protocol) -> (u protocol, u layout_serial, a(iiiid) monitors)
method SetState(u state)       # 0 starting, 1 playing, 2 paused, 3 failed
signal Layout(u layout_serial, a(iiiid) monitors)   # x, y, w, h, scale; ≤ 16 entries
signal PauseHint(u reasons)    # bitmask: covered, locked, overview, (later) display-off
```

Rules:

1. The extension spawns the renderer itself
   (`Meta.WaylandClient.new_subprocess`) with a **fixed argv** (installed
   binary path, `--present=gnome`). Nothing on D-Bus can choose what is
   executed, a path, a URL, a media item, a shader or code.
2. Callers are accepted only if
   `GetConnectionUnixProcessID(sender) == subprocess PID`. After the first
   successful `Hello`, the bridge binds to that unique bus name and rejects
   every other sender without a round-trip; this prevents a call flood from
   costing the Shell bus round-trips. Clear the PID and the binding when the
   process exits (the spike doesn't yet; that would be a PID-reuse window).
3. Signals are sent **unicast** to the bound renderer (`emit_signal` with
   a destination), not broadcast. The spike broadcast them, so any
   eavesdropper could learn "user is locked / in fullscreen".
4. Only owned windows are adopted, at most one current window, and only if
   its buffer size equals the layout's canvas (or per-monitor) size within
   a cap (e.g. ≤ 16384 px per side), so a bad renderer can't make the Shell
   hold giant textures. Extra windows are ignored and stay normal windows.
5. Fixed-size arguments, enum/bitmask values, version check (unknown →
   `NotSupported`), and a restart cap (e.g. 3 restarts/min, then static
   fallback + `one-mk doctor` message).
6. The renderer, for its part, accepts `Layout`/`PauseHint` only from the
   bridge name *and* checks that this name's owner PID equals
   `org.gnome.Shell`'s owner PID.

What a malicious local process could still do:

- **Same user, outside the bridge:** anything the user can, e.g. replace a
  per-user extension or the per-user renderer binary in `~/.local`, which
  the Shell would then spawn. The bridge adds nothing here; a system-wide
  install under `/usr` narrows it.
- **Via the bridge, if not the renderer:** nothing beyond being refused.
  Before rule 3 it could learn coverage/lock state and monitor geometry;
  geometry is public via `DisplayConfig` anyway.
- **A compromised renderer** (e.g. a decoder exploited by a malicious
  file): it is an ordinary Wayland client. It can draw anything as the
  wallpaper, including a fake desktop under real windows; it can't put
  anything *above* windows through the bridge. It can send the bounded
  messages, churn windows (capped), or stop drawing (watchdog → static).
  It cannot run code in the Shell. Sandboxing the renderer (no network,
  read-only media dir) is recommended but **UNVERIFIED**.

## Recommendation for the product bridge

1. **Shape:** on GNOME, the extension spawns `one-mk-wall --present=gnome`
   and presents its window exactly as in this spike: hidden actor, DESKTOP
   type, `skip_taskbar`, sticky, lowered, per-monitor clipped
   `Clutter.Clone` in a layer appended to `Main.layoutManager._backgroundGroup`.
   Decoding, clock sync, networking and downloads stay in one-mk-wall
   (design §4–7). The Shell side stays a few hundred lines of
   presentation and window-observation code.
2. **Accept restarts as normal.** Because lock and other extensions'
   toggles bounce the extension, one-mk-wall must start fast and rejoin
   the group timeline at the agreed position (design §5 "preserve the
   group timeline while hidden, then rejoin"). Keep media on disk and
   measure GStreamer pipeline start: it is the visible gap after each
   unlock (**UNVERIFIED**).
3. **Layout:** one toplevel per layout serial. Use one canvas window for
   uniform-scale setups (verified, equal scale only); for mixed HiDPI or
   fractional scales, one window per monitor is probably safer
   (**UNVERIFIED**).
4. **Pause:** the extension hides the clones of covered monitors (hard
   guarantee) *and* sends `PauseHint`, so the renderer stops its decoder
   and clock rather than stalling in a frame wait. Add multi-window
   coverage (union of opaque window rectangles) before release.
5. **Overview/lock:** offer an opt-in "use the current scene's poster as
   my GNOME wallpaper" so the Overview and unlock dialog match. Changing
   the owner's background setting is an explicit consent decision.
   Mirroring clones into the Overview's per-workspace backgrounds is
   possible in principle but leans on more private code (**not tried**).
6. **Packaging:** ship the extension with its own UUID, pinned
   `shell-version`, no `unlock-dialog` session mode (it bought nothing
   because of the rebase), and `one-mk doctor` reporting the extension
   state (`OUT_OF_DATE`, `ERROR`) and the frame rate it observes.

## What GNOME 45–49 would need (from source/API reading, not run)

- **45+** use ES-module extensions with the `Extension` base class, as
  here. 44 and older would need the legacy format and are out of scope.
- **Spawning:** DING's compatibility code (read only) shows three APIs.
  `Meta.WaylandClient.new_subprocess(context, launcher, argv)` came from
  Mutter MR 4491 and is what 50 has. Earlier releases used
  `Meta.WaylandClient.new(global.context, launcher)` +
  `client.spawnv(global.display, argv)`, and the oldest used
  `Meta.WaylandClient.new(launcher)`. `owns_window` is used on all paths.
  Exactly which release changed which is **UNVERIFIED**.
- **Hiding from lists:** `Meta.Window.hide_from_window_list()` on newer
  versions; `waylandClient.hide_from_window_list(window)` on older ones.
- **Window type:** DING calls `window.set_type?.(DESKTOP)`, i.e. it is
  missing on some supported versions. Without it, this spike showed (on
  50, as NORMAL type) that the window is held under the top bar and steals
  keyboard focus. Older versions would need clone scaling to the monitor
  and focus handling, re-validated per release (**UNVERIFIED**).
- **Maximize checks:** `Meta.Window.is_maximized()` is new in 49 (DING
  branches on it); 45–48 use `maximized_horizontally &&
  maximized_vertically`. The coverage code must branch.
- **X11 sessions** (still present in 45–49, removed in 50): no
  `Meta.WaylandClient` there; the renderer could be an X11 window and
  still be cloned. Untested.
- `Main.layoutManager._backgroundGroup` and the `skip_taskbar` filters are
  long-standing, but they are private; re-check per version.

## Not verified here (do not overclaim)

Real hardware and real scanout (headless virtual monitors only; the
owner's hybrid Intel+NVIDIA box may behave differently), HiDPI and
fractional scaling, mixed refresh rates, GPU memory use, a real Rust/
GStreamer renderer (the stand-in is Python GTK4 drawing every vblank, so
its CPU numbers are not one-mk's), the visible gap during hot-plug and
after unlock, multi-window coverage, Mutter ping timeout, log-out/in
requirements after install, GNOME 45–49 behaviour, and licensing. The
extension runs inside GPL-2.0-or-later GNOME Shell; extensions.gnome.org
expects a GPL-2.0-or-later-compatible licence. "MIT OR Apache-2.0" offers
MIT, which is commonly considered GPLv2-compatible, but that is not a
legal review. DING (GPL-3) and Shell sources were only read; no code was
copied.

## Reproduce

Only on a disposable machine or with this isolation; never point it at a
live session. From `docs/spikes/gnome/`:

```sh
./up.sh 1                    # isolated Shell, 1 virtual monitor; enables harness, DING, spike
./S.sh eval -f js/stack.js   # actor tree (step 1)
B='Main.extensionManager.lookup("wallpaper-spike@one-mk.test").stateObj.bridge'
./S.sh eval "$B.setMode('color')"; ./S.sh shot step1        # PNG in /tmp/omk-gs-spike
./S.sh eval -f js/start-renderer.js                         # step 2a
./measure.sh "A" 10                                         # fps / CPU / damage
./S.sh eval "$B.setMode('off','hide')"; ./measure.sh "D" 10 # control: no clone
./isolated-session.sh run python3 client/test_window.py Max --maximize &   # coverage
./up.sh 2; ./S.sh eval -f js/start-renderer.js              # two monitors
./isolated-session.sh run python3 client/displayconfig.py apply 1          # unplug
./S.sh ext disable wallpaper-spike@one-mk.test; ./S.sh eval -f js/leaks.js
FAKE_GDM=1 ./up.sh 1; ./S.sh eval 'Main.screenShield.lock(false)'         # lock
./isolated-session.sh stop
```

Shell JS sources were read with
`gresource extract /usr/lib/gnome-shell/libshell-18.so <path>` into
`/tmp/gsjs` (deleted afterwards), and typelib APIs were listed with `gjs` +
`GIRepository` against `/usr/lib/x86_64-linux-gnu/mutter-18`.

## Files

- `docs/spikes/gnome/ext/wallpaper-spike@one-mk.test/`: the spike bridge
  extension (throwaway).
- `docs/spikes/gnome/ext/spike-harness@one-mk.test/`: test-only unsafe-mode
  switch for isolated sessions; never install on a real desktop.
- `docs/spikes/gnome/client/`: `wall_spike_client.py` (stand-in renderer),
  `test_window.py`, `frame_feeder.py` (step 2c), `displayconfig.py`,
  `fake_gdm.py`, `shell_eval.py`, `small_png.py`.
- `docs/spikes/gnome/isolated-session.sh`, `up.sh`, `S.sh`, `measure.sh`,
  `js/`: harness.
- `docs/spikes/gnome/transcripts/00–15`: command transcripts;
  `docs/spikes/gnome/shots/`: small screenshots.

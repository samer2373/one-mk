# GNOME Wayland production video — 2026-11-03

Qualified compositor: **GNOME Shell 50.1 only**. This replaces the setup-only
checkpoint and the earlier authority-record/decoder-cgroup laboratory design.
No other Shell version is admitted by the production extension or player probe.

## User flow and architecture

Wallpaper offers explicit Install, a log-out/log-in discovery notice, then
explicit Enable. Opening the page and enabling the extension start no renderer.
Selecting a video uses the existing distributed Plan, reports and helper launch
path. Pause/play are stdin commands to the same process; Off terminates it.
GNOME replacements reap the prior renderer before launching the next one because
the bridge admits one process; other compositors keep their seamless handover.
Pictures still work without the extension. An inactive extension produces setup
instructions rather than a permanent GNOME video refusal.

`one-mk-wallpaper` shares the layer-shell appsink/wl_shm decoder, Fit mapping,
per-monitor canvas slices, clock correction and battery holds. On GNOME it
creates an ordinary xdg_toplevel per matching output instead of a layer surface.
Output changes recreate affected surfaces. No decoding runs inside Shell.

The small extension authenticates the D-Bus sender's kernel PID/UID, requires
the desktop UID, and admits only that PID's windows carrying the launch's random
128-bit token and valid monitor geometry. The player verifies that the bridge
bus name belongs to the same owner as `org.gnome.Shell`. There is one registered
renderer, at most sixteen adopted windows, and a five-second presentation wait.
Ready is reported only after the extension acknowledges damage on every selected
surface, not merely after the helper starts.

Adopted windows become sticky desktop windows, outside the normal window list
and Alt-Tab. Their source actors stay hidden; nonreactive monitor-clipped clones
paint in Shell's background group. The Wayland input regions are empty. Neither
GNOME background settings nor native background actors are replaced. Off, sender
loss, process death and extension disable remove clones, revealing the original
desktop. Disabling wakes even a paused renderer through a bus-name-loss signal.
Paused/idle playback blocks in poll, rather than running a timer or decoder loop.

## Safety boundary and simplification

The production path removes authority files, attestation and dedicated decoder
cgroups. Credentials and a per-launch token bind presentation to the renderer
process; compositor and decoder remain separate processes. A renderer crash
cannot directly crash Shell, and the lifecycle tests verify Shell survives it.
Same-UID malicious processes are not a new OS security boundary: this does not
claim executable attestation or a media-decoder sandbox. System GStreamer codecs
run with the existing helper's user privileges, as on the layer-shell path.
Shell still executes extension code, so qualification is deliberately exact.
The laboratory crate retains its old XML schema locally; it no longer includes
the incompatible production extension's protocol as though they were identical.

## Reproducible private-session evidence

`docs/spikes/gnome/isolated-session.sh` uses a temporary HOME and XDG directories,
private session and system buses, memory settings and a private Wayland socket.
No owner-session extension, settings, bus, display or oneMK home/port is used.
Install and Enable use oneMK's production CLI. The test-only spike harness grants
Eval/screenshot access solely inside this private session; product code uses
neither Eval nor unsafe mode.

After building `one-mk` and `one-mk-wallpaper-helper` with `--profile bench-opt`,
install/discover/enable the production extension through its CLI and enable the
private spike harness. A fresh private session can be prepared with:

```sh
export SPIKE=/tmp/one-mk-gnome-video-session
mkdir -p /tmp/gnome-video-ext-source /tmp/gnome-video-evidence
cp -R docs/spikes/gnome/ext/spike-harness@one-mk.test /tmp/gnome-video-ext-source/
EXT_SRC=/tmp/gnome-video-ext-source bash docs/spikes/gnome/isolated-session.sh start 2
bash docs/spikes/gnome/isolated-session.sh run "$PWD/target/bench-opt/one-mk" wallpaper extension install
bash docs/spikes/gnome/isolated-session.sh restart 2
bash docs/spikes/gnome/isolated-session.sh run "$PWD/target/bench-opt/one-mk" wallpaper extension enable
bash docs/spikes/gnome/isolated-session.sh run gdbus call --session \
  -d org.gnome.Shell.Extensions -o /org/gnome/Shell/Extensions \
  -m org.gnome.Shell.Extensions.EnableExtension spike-harness@one-mk.test
bash docs/spikes/gnome/isolated-session.sh run gst-launch-1.0 -q \
  videotestsrc num-buffers=300 pattern=smpte75 '!' video/x-raw,width=1920,height=1080,framerate=30/1 \
  '!' timeoverlay halignment=center valignment=center 'font-desc=Sans 72' \
  '!' vp8enc deadline=1 cpu-used=4 threads=4 target-bitrate=500000 \
  '!' webmmux '!' filesink location=/tmp/gnome-video-evidence/clip.webm
bash docs/spikes/gnome/isolated-session.sh run python3 "$PWD/docs/spikes/gnome/qualify-production-video.py"
bash docs/spikes/gnome/isolated-session.sh stop
node --test docs/spikes/gnome/test_production_video.mjs
```

Retained evidence is `/tmp/gnome-video-evidence/report.json`, screenshots
`00-desktop-before.png` through `11-monitor-returned.png`, helper/daemon logs,
the Plan, clip and JSONL Playback events. Screenshots come from the private
`org.gnome.Shell.Screenshot` interface, not fabricated images.

Checks cover production set/status/pause/play/Off with a temp `--home`, null
input backend and port 35991; two 1920×1080 outputs with one shared source and
continuous crops; advancing frames; pixel-identical paused frames; Alt-Tab and
sticky/desktop flags; a normal window covering video; exact original desktop
pixels after Off, SIGKILL and extension disable; and removal/re-addition of an
output. The null input backend advertises one monitor to the daemon, so the
two-monitor check supplies the same production helper a two-monitor Plan.
CPU is measured as process user+system ticks over three seconds, percentages of
one CPU core, with optimized software-decoded 1080p VP8. Exact measurements and
gate outputs are retained alongside the evidence.

| State | Renderer | Shell |
| --- | ---: | ---: |
| Playing, two 1080p outputs | 34.33% | 6.00% |
| Paused, same process and frames | 0.00% | 0.00% |

The fixture is ten seconds of 1080p SMPTE75 color bars with a large advancing
timestamp crossing the monitor seam. `02-two-monitors-frame-a.png` and
`03-two-monitors-frame-b.png` show the continuous canvas advancing;
`06-window-above-video.png` shows normal application occlusion. The all-Fit-mode
and video-replacement checks also cover the exclusive GNOME renderer handover.

## Final gates

| Gate | Result |
| --- | --- |
| Workspace clippy, all targets, `-D warnings`, `-j 4` | Pass |
| Wallpaper/player/helper/app clippy, `aarch64-apple-darwin` | Pass |
| Same requested packages, `x86_64-pc-windows-msvc` | Pass |
| Debug wallpaper-helper build | Pass |
| `cargo test -j 4 --workspace` | 1,411 passed, 0 failed, 18 ignored; 49 result groups |
| Production extension Node unit tests | 5 passed |
| `BIN_DIR=target/debug sh scripts/check-packaging.sh` | Pass; optional Ruby syntax check skipped because Ruby is absent |

The gate logs are retained in `/tmp/gnome-video-evidence/gate-*.log`.
The final private sessions, temporary homes and worktree build target are removed
after qualification; screenshots, clip, Plan and logs remain as evidence.

## Limits

No changes or tests were made in the owner's real login. GDM login, lock/unlock,
suspend/resume, fractional scaling, mixed-DPI/rotated physical outputs, desktop
icons supplied by other extensions and physical cross-computer synchronization
remain unqualified by this run. Fit and clock math retain existing unit coverage;
the private visual test uses Stretch. This change does not add a new GDM or
decoder sandbox claim. Historical laboratory tests are not counted as production
video evidence.

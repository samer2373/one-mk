<img src="assets/icon.svg" width="64" height="64" alt="">

# one-mk

Share one mouse and keyboard across your Linux, Windows and Mac computers.
Move the pointer off the edge of one screen and it appears on the next. The
clipboard and files follow. Setup takes about a minute.

> **Status: early.** Linux (Wayland: GNOME 45+ or KDE Plasma 6.1+) is tested
> end to end on real hardware. The Windows and macOS backends compile and
> are unit-tested, but have **not yet run on real Windows or macOS
> machines**: expect rough edges there. The X11 and wlroots (Sway,
> Hyprland) backends are new: see [Supported desktops](#supported-desktops).

**File edge drops:** hold a file drag over the thin peer-facing edge zone and release to copy into the other computer's Downloads inbox; X11 is verified with scripted XDND on private Xvfb, Windows/macOS adapters are compile-checked only, and Wayland edge drops are **not available on this desktop** (use `one-mk send`); configure with `one-mk drag --on|--off` / `one-mk drag --zone 12` or `[drag] enabled = true, zone_px = 12` in settings (defaults on only for supported desktops). See [feasibility and limits](docs/DRAG-FILES.md).

X11 edge drops reject XTEST/virtual-device arming but cannot establish user
intent against hostile local X clients; disable them in an untrusted X session.
The future Windows helper's pure policy defaults secure-desktop input **off**;
UAC opt-in carries a local-malware elevation risk. No service ships yet; see
[the Windows helper threat model](docs/WINDOWS-SERVICE.md#threat-model-secure-desktop-is-privileged-authority).

## Quick start

**Live wallpaper playback is not available.** M1 remains stopped at a compositor
protocol-warning gate. Receiver-owned policy scaffolding is available through
`one-mk wallpaper status|enable|disable|doctor` and
`one-mk wallpaper consent grant|revoke PEER` (each accepts `--json`). It defaults
off with no consent; enabling it does not start a renderer or accept media.
Grant consent on the receiving computer, using a directly paired computer's
name or full fingerprint; revoke it there at any time. Disabling preserves saved
grants but clears pending metadata. Forgetting a computer removes its grant.
The separate developer-only `one-mk-wall` probe is not linked into or launched
by the daemon. Wallpaper doctor exits 3 when enabled because no qualified
renderer is available, and exits 0 when off. See the
[canvas, wire and consent contract](docs/WALLPAPER-CANVAS.md).
Importing media with `one-mk wallpaper-import` needs the separately installed
optional `one-mk-wall-import` package (a sandboxed worker); without it the
command refuses. See [import limits](docs/WALLPAPER-IMPORT.md).
See [the stop report and troubleshooting](docs/spikes/wallpaper-m1-stop.md);
do not run the risk probe on your real desktop.

### Optional wallpaper import development package

`cargo build -j 2 -p one-mk -p one-mk-wall-import` builds the finite optional
import helper beside the CLI. It is **not linked into the daemon**, starts only
on an explicit wallpaper command, and is not included in release packages.
`one-mk wallpaper doctor --json` explains why import is unavailable without an
explicit audited runtime. The Linux developer import surface is:

```sh
one-mk wallpaper import animation.webm --runtime /absolute/audited-runtime \
  --output /absolute/existing-private-directory --json
```

Only VP9/AV1 WebM and animated GIF/APNG/WebP are admitted; other formats need
conversion first. No libav/FFmpeg or GPL encoder is permitted. This saves a
validated source, poster and content-addressed record, **not a normalized or
playable scene**. Windows/macOS import refuses until native isolation exists.
See [import threat model, limits, testing and remaining gates](docs/WALLPAPER-IMPORT.md).

Get it first. Every release has a file for each system (the
[latest release](https://github.com/samer2373/one-mk/releases/latest)):

| System | File | Notes |
|---|---|---|
| macOS, Apple silicon / Intel | `one-mk-aarch64-apple-darwin.dmg` / `one-mk-x86_64-apple-darwin.dmg` | Open it and drag one·mk onto Applications |
| Windows 10/11, 64-bit | `one-mk-x86_64-pc-windows-msvc-setup.exe` | Installs for you alone, into `%LOCALAPPDATA%\Programs\one-mk` |
| Debian, Ubuntu and relatives | `one-mk-x86_64-unknown-linux-gnu.deb` | `sudo apt install ./one-mk-x86_64-unknown-linux-gnu.deb`, then `one-mk install` as yourself |
| Any Linux | `one-mk-x86_64-unknown-linux-gnu.AppImage` | `chmod +x` it and run it; with no arguments it opens the window, otherwise it is `one-mk` |
| Other Linux, by hand | `one-mk-x86_64-unknown-linux-gnu.tar.gz` (and `.zip` on Windows) | Just the programs |

On macOS and Linux the one-line installer picks the right file:

```bash
curl -fsSL https://raw.githubusercontent.com/samer2373/one-mk/main/install.sh | sh
```

On a Mac this installs **one·mk** into Applications and opens it. Click
Start and turn on "Start at login" in the window, pair from the window, and
you never need the terminal. macOS asks for Accessibility permission the
first time: turn on one·mk in System Settings > Privacy & Security >
Accessibility. If an update was not signed with an Apple Developer ID,
macOS may ask again after it: remove one·mk from that list and add it back.

Or build it yourself (see Building). How each package is made, and which
are untested, is in [docs/PACKAGING.md](docs/PACKAGING.md).

The commands below do the same from a terminal (on Linux, run
`one-mk-app` to open the window). On **each** computer:

```bash
one-mk install     # start one-mk automatically whenever you log in
one-mk pair        # both computers show the same 6-digit code; confirm on each
```

That is all. Move the pointer off the screen edge toward the other
computer. (`one-mk run` does the same as `install` for the current session
only, and shows what is happening.)

The computer you pair from first is placed to the right. To change it, on
either computer:

```bash
one-mk layout "other-pc" left      # applies at once, no restart
```

The other computer is told, so it records the mirror image (you put it on
your left, so you are on its right). If the two ever disagree, for example
because one was switched off when you changed it, the newer change wins as
soon as they connect. If you change it on both computers in the very same
millisecond, the one with the larger certificate fingerprint wins and the
other adopts its layout, so they always end up as mirror images (a computer
that predates this rule converges too, as long as one side is up to date).

Screens of different sizes, or not lined up? Drag the computer next to one
of your monitors in the window's Layout tab, or say where its top-left
corner sits, in your pixels (see [Arranging the computers](#arranging-the-computers)):

```bash
one-mk layout "laptop" bottom --offset 300,1080   # below, its left edge 300 px right of yours
```

## Supported desktops

| Desktop | Backend | Edges and capture | Being controlled | Tested |
|---|---|---|---|---|
| GNOME 45+, KDE Plasma 6.1+ (Wayland) | `portal` | InputCapture portal (pointer barriers) | RemoteDesktop portal | On real hardware |
| Sway, Hyprland, labwc, river, Wayfire (wlroots) | `wlroots` | Invisible layer-shell strips on the edges, pointer lock, relative pointer | Virtual pointer and keyboard | Compiled and unit-tested only |
| X11: Xfce, MATE, Cinnamon, i3, Openbox, GNOME or KDE on X11 | `x11` | XFixes pointer barriers, pointer and keyboard grab, XInput2 raw events | XTest | Against a virtual X server (Xvfb) |
| Windows 10 and 11 | native | Low-level hooks | `SendInput` | Compiled and unit-tested only |
| macOS | native | Event tap | `CGEvent` | Compiled and unit-tested only |

On Linux one-mk picks the backend by itself: on Wayland the desktop
portals where the desktop has them, else the wlroots protocols; in an X11
session, X11. Running status reports the backend actually opened (including
`wlroots`); `one-mk doctor` uses that resolved backend for its permission checks;
`one-mk run --backend wlroots` or `--backend x11` forces one of those
(the default, `native`, picks). Notes:

Every backend (portal barriers, X11 barriers or fallback edge windows,
wlroots edge surfaces, the Windows and macOS hooks) arms only the edge
segments that lead to a peer, including inner monitor edges with nothing of
yours beyond them. A wall beside a partly touching peer stays local,
including after re-arming.

- **X11** asks for no permission. X11 has no display scaling, so the
  desktop is measured in real pixels (a HiDPI setup with `Xft.dpi` or
  `GDK_SCALE` only makes programs draw larger). Scrolling arrives in whole
  wheel clicks. Using this computer's own mouse or keyboard takes it back
  from a computer controlling it, as on Windows and macOS.
- **wlroots** needs the compositor to offer `zwlr_layer_shell_v1`,
  `zwlr_virtual_pointer_manager_v1`, `zwp_virtual_keyboard_manager_v1` and
  `zwp_relative_pointer_manager_v1` (doctor names any that are missing);
  `zwp_pointer_constraints_v1` keeps the pointer still while it is away.
  Typing uses this computer's keyboard layout.

### Different keyboard layouts or missing AltGr characters

Typing currently forwards physical keys: the receiving OS chooses the letters.
For German/US Z/Y, French AZERTY, or AltGr symbols that come out wrong, select
the same keyboard layout on both computers and check the receiving OS's Right
Alt/AltGr mapping. `one-mk layout` places computers; it does not set a keyboard
layout. Japanese kana and IME composition are not translated between computers.

`one-mk typing --keys physical|characters|auto [--peer NAME_OR_ID]` saves a
preference in `[typing]`; the default is `physical`. **Characters and auto are
not operational yet**: they currently fall back to physical, explicitly reported
by the command, status and doctor. The Advanced tab shows the saved preference
and this limitation. No clipboard paste or live keyboard-map rewriting is used.
The Linux follow-up adds tested X11 keymap snapshots and compose primitives,
but does not yet connect them to typing visits. It does not fix cross-layout
AltGr typing in the daemon; do not enable characters expecting that fix.
An isolated X11 remap/restore experiment also reproduces lost characters in a
delayed destination client, even with byte-identical keymap restoration. X11
character mode remains disabled; a crash-recovery marker alone does not fix
that delivery race. The private-Xvfb reproducer is
`bash scripts/test-x11-typing-safety.sh` (success confirms the blocker, not
working character injection).
The 5E-5 portal keysym helper passes private fake-portal protocol tests, but real
GNOME/KDE editor delivery and cross-session shortcut ordering are **UNTESTED**;
native TEXT remains disabled. The owner-only Text probe is
`bash scripts/test-portal-typing-manual.sh --owner-desktop de` (also `fr`, `pl`,
`tr`); it requires explicit desktop access and a disposable editor. A retained
X11 pool experiment still loses text when a client resumes after its retirement
deadline: `bash scripts/test-x11-typing-safety.sh --pool-retention`.
See [keyboard layout design and remaining work](docs/KEYBOARD-LAYOUT.md).

## Pair without sitting at both computers

On computer A, run `one-mk invite --expires 10m`. Privately transfer its
one-time link or QR code to B. On B, run `one-mk join` and paste the link
(prefer stdin: positional links are visible in process arguments and shell
history). B prints **Your code: 123456**. Compare that code with A's prompt
over a trusted channel; a familiar device name is not enough.

A shows B's name, full fingerprint and comparison code and asks for `y`
(`--confirm`, the default). B also shows A's full grouped fingerprint and
asks whether to trust A before confirming. Non-interactive B requires
`one-mk join --yes`, which skips B's trust prompt. For unattended A, use
`one-mk invite --no-confirm`: **anyone holding the link can join if they
use it first**. Skipping both prompts removes human verification. Headless
stdout/QR output may enter a journal. Treat the link like a password,
including in chat history.
It expires, works once, and dies when the invitation process closes.
Start sharing with `one-mk run` on both; use `one-mk forget <id>` to revoke.
See [invitation security and protocol](docs/PAIRING-INVITE.md).

## Everyday use

| Command | What it does |
|---|---|
| `one-mk status` | Is it running? Who is paired, online (with `rtt 4 ms`: how long an answer takes), where, and who controls whom |
| `one-mk takeback` | Take this computer back from the one controlling it |
| `one-mk switch <pc\|here\|next>` | Move the mouse and keyboard to another computer without crossing an edge (`here`: back to this one) |
| `one-mk lock-pointer on\|off\|toggle` | Keep the pointer on this computer: edges lead nowhere |
| `one-mk broadcast [on\|off\|toggle] [--idle-off MIN]` | Type on every connected computer at once (see below) |
| `one-mk hotkeys [options]` | Show or change the keyboard shortcuts (see below) |
| `one-mk edge [options]` | Rules for crossing edges, pointer wrap and layout learning (see below) |
| `one-mk gaming on\|off\|auto` | Raw mouse movement for games on another computer |
| `one-mk lock-all` | Lock this computer's screen and every connected one's |
| `one-mk lock-sync on\|off` | Locking any computer locks the others (those with lock sync on) |
| `one-mk hooks [--on-enter CMD] [--on-leave CMD] [--on-peer-connect CMD] [--on-peer-disconnect CMD] [--off]` | Run a command when the pointer arrives or leaves, or a computer connects or disconnects (see below) |
| `one-mk power [--block-screensaver on\|off] [--sleep-sync on\|off]` | Keep other computers awake while you control them; sleep displays together (see below) |
| `one-mk keymap [list\|add\|remove\|clear\|check]` | Change what keys do on the other computers: remaps, combos, tap/hold, layers, actions (see below) |
| `one-mk buttons [list\|map N TO\|preset side-back-forward\|clear]` | Remap mouse buttons on the other computers |
| `one-mk pointer [--speed X]` / `one-mk scroll [options]` | Pointer speed; scroll direction, natural scrolling, speed, notches, Shift-scroll |
| `one-mk input-devices [--json]` | The keyboards and mice this computer has seen, for rules for one of them (`--device`, see below) |
| `one-mk pointer-overlay [auto\|always\|never]` | Draw the pointer here while another computer controls this one and it has no mouse (see below) |
| `one-mk install` / `uninstall` | Start automatically at login (and add the file manager's "Send to" entries) / stop doing that |
| `one-mk integrate [--remove]` | Add or remove "Send to <computer>" in the file manager only (see below) |
| `one-mk pair` | Pair with another computer (run on both) |
| `one-mk layout <pc> <left\|right\|top\|bottom> [--offset DX,DY\|none]` | Say where a paired computer sits, and where its (0,0) is in your screen coordinates |
| `one-mk profile save\|use\|list\|delete <name>` | Named layouts, e.g. "home" and "office" |
| `one-mk profile auto <name> --ssid <wifi> --network <router-mac\|subnet>` | Switch layout by itself when this computer joins that network |
| `one-mk layout <pc> --swap-ctrl-cmd on\|off\|auto` | Ctrl/Cmd swapping (see below) |
| `one-mk send <files or folders> [--to <pc>]` | Send files; they arrive in `Downloads/one-mk` |
| `one-mk clip-send [--to <pc>]` | Send this computer's clipboard to one computer now |
| `one-mk clip-get [--from <pc>]` | Copy one computer's clipboard to this one now |
| `one-mk clipboard [--text on\|off] [--html on\|off] [--image on\|off] [--max-mb N]` | Show or change what the clipboard shares (applies at once) |
| `one-mk notify [on\|off]` | A desktop notification when files arrive |
| `one-mk devices` | List paired computers |
| `one-mk forget <pc>` | Unpair |
| `one-mk run` | Run in the foreground (`-v` for detailed logs) |
| `one-mk id` | This computer's name and id |
| `one-mk doctor` | Find out why computers aren't connecting, with a fix for each problem (`--json` for programs) |
| `one-mk bench` | Measure latency and CPU cost |

Plain `one-mk run` prints short messages such as "Connected to desk (on your
right)." and "desk disconnected; reconnecting automatically.". Add `-v` for
full logs when something is wrong.

**Clipboard.** It follows the pointer: text, HTML and images, sent once when
the pointer crosses and only if it changed (turn off with
`one-mk run --no-clipboard`). `one-mk clipboard` shows what is shared and
changes it, at once and for good:

```bash
one-mk clipboard --image off          # no images, either way
one-mk clipboard --max-mb 5           # nothing bigger than 5 MB (default 25, at most 256)
one-mk clipboard --html off --text on
```

A switched-off kind is neither sent nor accepted; a clipboard over the limit
is not sent (and a bigger one that arrives is refused before it is read).
You don't have to move the pointer to move the clipboard:

```bash
one-mk clip-send --to other-pc    # push mine to it now (--to is optional when only one is connected)
one-mk clip-get --from other-pc   # fetch its clipboard now
```

`clip-send` tells you if the other computer ignored it (a kind switched off
there, or over its limit). Others can only fetch yours if
`one-mk clipboard --allow-fetch on` **on the computer being read**. Fetching is
**off by default**; existing settings with `allow_pull = true` keep their opt-in.
`clip-send` still works without enabling fetching: the owner chooses to push.

### Clipboard history (off by default)

History is **OFF by default**, including upgraded settings without an explicit
`history_max`. The first clipboard summary/status after such an upgrade explains
the privacy decision once; the window continues to show the current OFF state.
Existing explicit retention settings are preserved. Opt in below to remember
copies; unmarked secrets may be retained. Lists and window rows are masked until
you explicitly choose **Show** (or CLI `show`). Open
**Clipboard & files → History** to restore an item here, or use:

```sh
one-mk clipboard history                  # newest first, stable item numbers
one-mk clipboard history list --json      # metadata and masked placeholder, no preview content
one-mk clipboard history show 7           # full text; terminal controls escaped
one-mk clipboard history show 7 --json    # exact text in JSON
one-mk clipboard history copy 7           # restore here only; no network send
one-mk clipboard history delete 7
one-mk clipboard history clear
one-mk hotkeys --fetch-clipboard 'ctrl+alt+v'
```

The fetch shortcut gets the next online computer's clipboard (name order,
wrapping after this computer), then reports **Got text/html/image from NAME**.
Repeated presses select that same peer: this is not a round-robin selector and
does not fall back if that peer refuses fetching. Use `clip-get --from NAME`
to select another source explicitly.
Windows currently prints this result to stderr: the existing notifier does
not implement Windows toast notifications.
It does **not** type Ctrl+V: press your normal paste shortcut afterward.
Linux/macOS default to Ctrl+Alt+V; Windows uses Ctrl+Alt+Shift+V to avoid
AltGr conflicts. Fetching stays **off by default** on every source computer;
enable it there with `one-mk clipboard --allow-fetch on`. `clip-get --from NAME`
selects a particular computer instead.

Configure retention in the existing `settings.toml` (restart or run
`one-mk clipboard --text on` to apply changes while keeping text enabled):

```toml
[clipboard]
history_max = 20           # 0 disables history; hard maximum 1000
history_max_mb = 16        # total retained bytes; hard maximum 256 MB
history_persist = false   # opt in only if you accept the disk privacy risk
history_skip_apps = ["keepassxc", "keepass", "1password", "bitwarden", "org.keepassxc.KeePassXC"]
```

Retention uses its own safety cap, not the sharing size limit. Lowering the
sharing limit does not delete history; restoring an oversized item requires
raising that limit first. Consecutive identical content
is deduplicated. Oldest images are evicted before text when a cap is reached.
History is **not written to disk by default** and is forgotten at shutdown.
With `history_persist = true`, text and HTML are **plain text on disk,
unencrypted**, alongside PNG images in `clipboard-history.bin` under the
settings folder. Saves replace the file atomically with mode 0600 on Unix.
Clear removes retained contents and the saved file, but does not secure-erase
disk blocks or backups. The window asks for a second click to confirm Clear.
Turning persistence or history off removes that saved file. Saves are debounced
and coalesced on a dedicated thread; a crash can lose changes not yet saved.
CLI Delete/Clear wait for their durable save/removal and report errors.

Sensitive/concealed copies are never retained, even with sharing's
`skip_sensitive` disabled. Native checks read X11's `x-kde-passwordManagerHint`,
Windows' `ExcludeClipboardContentFromMonitorProcessing` and
`CanIncludeInClipboardHistory`, and macOS' `org.nspasteboard.ConcealedType`.
The Wayland data-control path also checks the KDE MIME hint when available.
Only the X11 marker path was runtime-tested here; Windows/macOS are
compile-checked only, **runtime untested**. No mechanism detects all secrets.
X11 focus-based app
exclusion is best effort, without hooks; it requires the desktop to identify
the focused app. Native Wayland monitoring and Windows/macOS runtime behavior
are **untested**; those platforms must not be treated as verified. Windows/macOS
currently capture local contents on clipboard access, not every copy event.
Contents already held when one-mk starts enter history on the next access
or clipboard change, not immediately at startup.
See [validation and manual tests](docs/CLIPBOARD-HISTORY-QA.md).

**Troubleshooting:** missing items can mean a disabled format, size cap,
password-manager focus, or `history_max = 0`. Run `one-mk status` and
`one-mk doctor` for retention and platform coverage. If saved history is
corrupt, inspect the reported error, then run `one-mk clipboard history clear`
to discard it; the damaged file is not silently overwritten. If fetch is
refused, run `one-mk clipboard --allow-fetch on` on the named source computer.
With `--skip-sensitive on` (default), detected sensitive or skipped-app content
is withheld from pointer-follow pushes and `clip-get`. Explicit `clip-send`
overrides sharing protection, not history exclusion. App skipping is X11-only,
not a guarantee about the actual source app or Wayland-native focus.
History OFF avoids retaining ordinary local copies or PNG-encoding them solely
for history. Opt-in history may retain up to 16 MiB by default; this is not an
RSS ceiling (decoded images, encoder buffers and allocator overhead add memory).

**Files.** `one-mk send photos/ notes.pdf` goes to your only paired computer
(use `--to` if you have several). They land in `Downloads/one-mk` there
(change with `one-mk run --downloads <dir>`). An interrupted transfer resumes
when you send the same file again. While files move, `one-mk status` shows a
line like `Receiving 12 files from desk: 45% (12.3 MB of 27.1 MB), 8.1 MB/s,
3 s left` (and `Sending ...` on the other computer). When a transfer ends,
the receiving computer shows one desktop notification naming the files, with
**Show in folder** (Linux, over D-Bus; macOS shows the notification without
the button; Windows does not notify yet). Turn it off with `one-mk notify off`,
or for one run with `one-mk run --no-notify`. Nothing runs while idle.
Each send is limited to 10000 files and 1 TiB in total (also the per-file
limit); inconsistent or oversized incoming batches are rejected before progress
accounting.

**Send from the file manager.** `one-mk install` (or `one-mk integrate`) adds
**Send to <computer>** to the right-click menu: Nautilus and Nemo (a script in
`Scripts`), Dolphin (a service menu), Explorer (the *Send to* menu) and Finder
(a Quick Action, under *Services*). With several paired computers there is one
entry each, plus **Send with one-mk**, which takes the first. The entries are
rewritten whenever you pair or forget a computer, and `one-mk uninstall` (or
`one-mk integrate --remove`) removes them; files that are not one-mk's are
never touched. If sending fails, a desktop notification says why (Windows keeps
the console window open instead). On Linux the entry only appears for file
managers that are installed. Only Nautilus/Nemo/Dolphin scripts and menus on
Linux are tested on a real system; the Windows and macOS entries are written
and unit-tested but have not been seen in Explorer or Finder.

**Mac and PC keyboards.** When keys go between a Mac and a Windows or Linux
computer, Ctrl and Cmd trade places by default, so Ctrl+C on the PC keyboard
copies on the Mac and the other way round. The sending computer does the swap.
Set it per computer with `--swap-ctrl-cmd on`, `off`, or `auto` (the
default: swap only between a Mac and something else). It is stored
for each device on the computer where you set it.

**Emergency:** `Ctrl+Alt+Shift+Esc` always brings the pointer back to the
computer you are sitting at.

### Arranging the computers

Each paired computer sits on one side of this one. By default the pointer
crosses anywhere along that side: wherever you leave your screen, you
arrive at the same fraction of the way along its edge, whatever the two
sizes.

To place it exactly, drag it in the window's **Layout** tab next to one of
your monitors, or give `--offset`: where its (0,0), its top-left corner,
sits in your screen coordinates (logical pixels, what your display settings
call the resolution after scaling). The pointer then crosses only where
its screens touch yours, pixel for pixel, like two monitors of one
computer; beside that the edge is a wall, as beside a real monitor:

```bash
one-mk layout laptop bottom --offset 300,1080   # below a 1080-high screen, its left edge 300 px right of yours
one-mk layout laptop right --offset 1920,-200   # right of a 1920-wide screen, its top 200 px above yours
one-mk layout laptop --offset none              # anywhere along its side again (the side stays)
```

Moving a computer to another side without `--offset` drops its offset (it
means something else on another side). Offsets must be within
**-16777216 and 16777216** on each axis. `one-mk devices` shows each
offset. The other computer gets the mirror image (`1920,-200` here is
`-1920,200` there), and as with sides the newer change wins. A computer
whose screens touch yours nowhere is warned about in `one-mk status` and in
the window; `one-mk switch` still goes there, and all of its facing edge
leads back.

Several computers can share one side if their screens touch different
parts of it, for example two laptops below a wide monitor:

```bash
one-mk layout laptop-a bottom --offset 0,1440
one-mk layout laptop-b bottom --offset 1920,1440
```

**Several monitors.** The pointer crosses only through a monitor edge with
no monitor of yours beyond it: without an offset, the outer edge of the
whole desktop where a monitor reaches it; with an offset, wherever the
other computer's screens touch such an edge, even one set back from the
outer boundary. The pointer arrives where it left, measured along the
monitors of that edge, and comes back to the same point.

**Profiles.** Save the arrangement under a name and switch between them:

```bash
one-mk profile save home        # where every paired computer is now
one-mk layout laptop left       # rearrange at the office...
one-mk profile save office
one-mk profile use home         # back home: everything moves at once
one-mk profile list
one-mk profile delete office
```

A profile holds each computer's side and offset, nothing else: edge rules,
gaming mode and the other settings stay the same everywhere, because they are
about how you like to work, not where the computers stand. Using a profile
counts as a new change, so the other computers pick it up.
Names use 1-64 letters, digits, spaces or `._-`; paths, `..`, control characters
and TOML syntax are rejected.

To switch by itself when this computer joins a network:

```bash
one-mk profile auto office --ssid "Office WiFi"
one-mk profile auto home --network 192.168.1.0/24          # a subnet this computer is on
one-mk profile auto home --network aa:bb:cc:dd:ee:ff       # the router's MAC address
one-mk profile auto home --off
```

Repeated `profile auto` calls accumulate distinct Wi-Fi/network rules. Use
`--clear` to remove earlier rules before adding replacements, or `--off` to
remove every automatic rule for that profile.

The first profile that matches is used, once each time the network changes,
so a `one-mk layout` change afterwards stays until you move on. Set this up
on one of the two computers. On Linux it listens to NetworkManager, which
tells one-mk about every network change (it never checks on its own, so it
costs nothing while nothing changes); without NetworkManager (systemd-networkd
or iwd on their own), profiles only change with `profile use`. On Windows and
macOS automatic switching is not done yet. Profiles are kept in
`profiles.toml` next to `devices.toml`.

**Switching without an edge.** `one-mk switch desk` puts the pointer in the
middle of desk's main screen; `one-mk switch next` goes round every online
computer and back to this one; `one-mk switch here` brings the mouse and
keyboard back (and, if another computer is controlling this one, takes it
back, like `takeback`). Held keys are released on the computer left behind,
as with any crossing. The keyboard shortcuts below do the same.

**Keyboard shortcuts.** They work wherever the pointer is:

| Shortcut | What it does |
|---|---|
| `Ctrl+Alt+1` ... `Ctrl+Alt+9` (Windows: `Ctrl+Alt+Shift+1` ... `9`) | Jump to computer 1 to 9 |
| `Ctrl+Alt+PageDown` | The next computer (`one-mk switch next`) |
| `Ctrl+Alt+P` | Lock the pointer to this computer, or unlock it |
| `Ctrl+Alt+H` | Back to this computer (`one-mk switch here`) |
| `Ctrl+Alt+B` (Windows: `Ctrl+Alt+Shift+B`) | Broadcast to every connected computer, or stop |

Computers are numbered by name, this one included, so when every computer is
paired with every other, `Ctrl+Alt+2` means the same computer on every
keyboard; `one-mk hotkeys` shows the numbers. Only computers that exist get a
shortcut: with two, `Ctrl+Alt+3` and up stay free for programs. Change the keys with, for
example, `one-mk hotkeys --jump 'ctrl+shift+{n}' --cycle 'super+f12'
--lock-pointer none --home-key 'ctrl+alt+end'` (modifiers `ctrl`, `alt`,
`shift`, `super`, then one key: a letter, digit, `f1`-`f24`, an arrow,
`pageup`, `space` and so on; `none` turns one off), or turn them all off with
`--off`. They are kept in the `[hotkeys]` table of `settings.toml`.

While the pointer is on this computer the system delivers them: the
GlobalShortcuts portal on Linux (GNOME 48+, KDE Plasma 6), `RegisterHotKey`
on Windows, a keyboard event tap on macOS (the Accessibility permission
one-mk already has). While it is on another computer, one-mk recognizes them
in the keys it captures: a shortcut fires only when its key goes down with
exactly its modifiers and no other key held, and that key is never sent to
the other computer. Ctrl and Alt are already there by then (one-mk can't know
a shortcut is coming, and holding every Ctrl back would slow it down), so the
switch releases them: the computer left behind lets go of everything, and
the next one never saw them go down. The right Alt key doesn't count as Alt
there, because it is AltGr on many keyboards. While another computer controls
this one, this computer's own shortcuts are off (on Windows unregistered, on
macOS let through), so a chord typed over there reaches the program here; on
Linux the desktop still takes the chord, and one-mk ignores it.

- **Linux:** the portal accepts only installed programs, so run
  `one-mk install` once (it also writes a hidden desktop entry,
  `io.github.onemk.OneMk`), then restart one-mk. GNOME then asks you to
  confirm the shortcuts in a dialog and lets you change their keys under
  Settings > Keyboard (this dialog step is not yet tested). Where the desktop has no GlobalShortcuts portal, `one-mk status`
  and `one-mk doctor` say so: the shortcuts then work only while the pointer
  is on another computer, so bind `one-mk switch next` (and
  `one-mk switch here`) to a key in your desktop settings instead.
- **Windows:** Windows reports AltGr as Ctrl+Alt, and AltGr+digit types
  `{ [ ] }` on German keyboards and most of the digit row on French ones, so
  the jump shortcuts add Shift there by default. If AltGr+P or AltGr+H types
  a character on your keyboard (Polish and others), pick other keys for
  those, for example `one-mk hotkeys --lock-pointer 'ctrl+alt+shift+p'`. A
  shortcut another program already uses is skipped and `one-mk status` names
  it.
- Windows and macOS shortcuts are not tested on real hardware yet.

**Broadcast: type on every computer at once.** `one-mk broadcast on` (or
`Ctrl+Alt+B`, or the tray's or window's "Broadcast to all computers") sends
everything you type, click and scroll here to every connected computer at
the same time, for setting up several computers alike or running the same
command on all of them. Each computer gets its own pointer, starting at the
middle of its main screen, that moves by the same amount as your mouse and
stops at that computer's own screen edges; each gets its own Ctrl/Cmd swap
and key rules, as in a normal visit. It goes to the computers connected when
it starts (one that connects later is not added), and stops with the
shortcut, `one-mk broadcast off`, any switch, `Ctrl+Alt+Shift+Esc`, the screen
locking, the last computer leaving, or after 15 minutes without input
(`one-mk broadcast --idle-off 30` changes that; 0 = never). Nothing stays held
on any computer when it stops or when one drops out. `one-mk status`, the
tray, the window and a notification say it is on. The clipboard does not
follow during broadcast, and gaming mode and rules for apps don't apply. All
the details are in [docs/BROADCAST.md](docs/BROADCAST.md). Tested on Linux
(X11 and the test backend); untested on Windows, macOS, wlroots and the
GNOME/KDE portal.

**Lock to this computer.** `one-mk lock-pointer on` (like Scroll Lock in
Barrier) stops the edges from leading anywhere, for example while playing a
game here; `one-mk switch` still works. `one-mk run --start-locked` starts
that way. `one-mk status` shows it.

**Edge rules.** For every edge, or one edge with `--side left|right|top|bottom`:

```bash
one-mk edge --corner 20          # ignore crossings within 20 px of a screen corner
one-mk edge --dwell 250          # the pointer must rest at the edge for 250 ms
one-mk edge --modifier shift     # only cross while holding Shift (none|shift|ctrl|alt)
one-mk edge --fullscreen-guard on --fullscreen-allow vlc
one-mk edge                      # show the rules; --reset turns them off
```

While the pointer waits at an edge (dwell or modifier), moving away hands it
back at once (so does sliding along the edge past where the other computer
faces it), and typing or clicking there goes to this computer. Corners are
those of the desktop's outline: where an edge meets a neighbouring monitor
that sticks out, not where it hides behind one. A click
there is replayed as a whole click (press and release at once), so no button
can stay held whatever the system reports afterwards; the catch is that a drag
cannot start at such an edge: start it away from the edge. The
fullscreen guard keeps the pointer here while the focused window fills its
screen, except for programs in the allow list (`--fullscreen-block` never
leaves a program, fullscreen or not). Windows can always tell; on Linux only
X11 windows can be seen, which on Wayland means programs running through
XWayland (most games); macOS can't tell yet. Where it can't tell, the
pointer crosses as usual. Settings are kept in `settings.toml` and apply at
once.

**Edges where no computer sits.** Pointer wrap brings the pointer in at
the opposite edge when you push it out through an edge with no computer
(like "wrap mouse" in Mouse Without Borders); layout learning notices when
you push twice toward a computer that isn't placed yet:

```bash
one-mk edge --wrap on            # left and right edges wrap round (all: top and bottom too; off: the default)
one-mk edge --learn suggest      # the default: "Is laptop on your left? Run: one-mk layout laptop left"
one-mk edge --learn auto         # place it there (left or right only; above and below are suggested) and say how to undo
one-mk edge --dismiss            # hide the question shown now
```

Wrap keeps the height (on several monitors, the same row: out at the right
of the rightmost monitor, in at the left of the leftmost one on that row),
follows the edge rules above (dead corners, dwell, modifier, fullscreen
guard), never happens on a side where a paired computer sits (even an
offline one), while the pointer is locked, or while another computer
controls this one. Learning only ever looks at a computer whose place is
still pairing's guess: once you place it (`one-mk layout`, the window, a
profile) or cross to it, it is left alone, and no edge is watched for it any
more. It asks at most once per computer every 10 minutes; the question
shows in `one-mk status`, as a notification and on the window's Layout
tab ("Yes, place it" / "No"). Tested on X11; untested on the Wayland
portals, wlroots, Windows and macOS (`one-mk doctor` says so). Details:
[docs/EDGE-PUSH.md](docs/EDGE-PUSH.md).

**Gaming mode.** Games capture the pointer and want raw movement, not
positions. With `one-mk gaming on`, while the pointer is on another computer
the mouse sends relative movement, which never stops at that computer's
screen edges, so it doesn't lead back either: come back with
`one-mk switch here` (or the emergency shortcut). `auto` does this only while
the other computer has a fullscreen window in front, where it can tell (see
above). Both computers need this version of one-mk; with an older one,
positions are sent as before. On Windows, the pointer speed setting also
applies to these movements outside games.

**A pointer on a computer with no mouse.** A laptop used with the lid
closed, a headless box or a tablet often shows no pointer at all while
another computer controls it (Windows hides it when no mouse is installed).
There one-mk draws a small arrow of its own at the pointer while it is
controlled: `one-mk pointer-overlay auto` (the default) does this only when
the OS shows none for lack of a mouse, `always` whenever this computer is
controlled, `never` not at all. Nothing runs until it is needed, and the
arrow goes as soon as control ends or a mouse is plugged in. Windows and
macOS (written, not yet tried on real machines); on X11 the pointer always
shows, so it isn't needed (`always` still draws it); on Wayland the desktop
draws the pointer and nothing else can, so it isn't available (`one-mk
doctor` says which). Details in [docs/NO-MOUSE-POINTER.md](docs/NO-MOUSE-POINTER.md).

**Lock every screen.** `one-mk lock-all` locks this computer and asks every
connected one to lock too. With `one-mk lock-sync on`, locking this computer
in any way locks the others that have lock sync on too (each lock is passed
on once, so it never comes back); a computer with lock sync off follows only
`lock-all`. Anything held down is let go before a screen locks, and modifier keys
are released once more after it unlocks, so no key stays stuck. Locking uses
logind on Linux, `LockWorkStation` on Windows and the login window on macOS
(where neither the old `CGSession` tool nor the login framework's lock call
exists, `lock-all` says this Mac couldn't be locked instead of pretending);
noticing a lock uses logind's lock hint on Linux
and the system's lock notifications on Windows and macOS.

**Media keys.** Play/pause, stop, next and previous track, volume and mute,
brightness (and keyboard backlight), eject, and the launch and browser keys
cross like any other key, between any two systems that have them. They are
carried as Linux key codes and translated on arrival: Windows types them by
virtual key (`VK_MEDIA_*`, `VK_VOLUME_*`, ...) and macOS, where media keys are
`NX_SYSDEFINED` events rather than key events, captures and posts those. Windows
has no brightness, fast-forward or eject keys and macOS has no launch or browser
keys, so those only travel between the systems that have them (a Mac's
fast-forward and rewind skip tracks on Windows). Holding volume or brightness
repeats; play/pause and the other once-per-press keys never do, and a media key
is released like any other when the pointer comes back or the link drops. The
mapping table, with every key's Linux, Windows and macOS code, is in
`crates/proto/src/mediakeys.rs`. The macOS and Windows sides compile and are
unit-tested but have not been run on real hardware.

**Run a command when the pointer moves.** `one-mk hooks --on-enter 'notify-send
one-mk here' --on-leave '/usr/local/bin/lights off'` (or `[hooks]` in
`settings.toml`) runs a command when the pointer arrives on this computer
(`on_enter`: a peer took control, or your own pointer came back) and when it
leaves (`on_leave`), and `on_peer_connect` / `on_peer_disconnect` when a
computer connects or goes away. The command gets `ONE_MK_EVENT` (`enter`,
`leave`, `peer_connect`, `peer_disconnect`), `ONE_MK_PEER` (the other
computer's name), `ONE_MK_PEER_ID` and `ONE_MK_SIDE` (which side of this
screen the other computer is on). In the file a command is an array
(`on_enter = ["/usr/bin/notify-send", "one-mk", "hello"]`) or a string that is
split into words with shell-style quotes, **without a shell**: nothing is
expanded (`$HOME`, `~`, `*`, pipes and redirections are ordinary characters).
`one-mk hooks --shell on` runs strings through `sh -c` (`cmd /C` on Windows)
instead. Commands entered with the CLI retain their original text, so changing
`--shell on|off` also reinterprets existing commands and prints the resulting
invocation. Older CLI-created argv arrays are reconstructed when enabling the
shell, with POSIX-safe literal quoting of every unsafe word; review the printed
invocation. Windows refuses conversion when cmd metacharacters, quotes, line
breaks, grouping parentheses or a trailing backslash cannot be represented safely.
Explicit argv arrays otherwise run directly.
Shell-mode hooks see **sanitized** `ONE_MK_PEER`, `ONE_MK_PEER_ID` and
`ONE_MK_SIDE`: only letters (including non-ASCII letters), ASCII digits,
spaces and `._-` survive; every other character becomes `_`. Direct argv hooks
keep the exact peer name, except Windows `.bat`, `.cmd` and `.com` programs also
receive sanitized values (including `run:` actions). Rust may additionally refuse
batch-file arguments it cannot safely escape; one-mk does not bypass that refusal.
Commands are limited to 4096 bytes.
A command is killed after 5 seconds (`--timeout SECS`, at most 60),
runs on its own task so one-mk never waits for it, and runs at most once per event
per 500 ms (connect/disconnect limits and coalescing are per peer). Faster triggers
coalesce to the newest pending run. Enter/leave supersede older pending opposite
transitions and run serially in a pointer lane; peer events have their own serial
lane, so a slow connect hook cannot delay pointer work. Configuration generations
discard pending commands whose text, shell mode or timeout changed. Already-running
commands finish; disabling hooks cancels both lanes and kills their Unix process
groups. All commands share the eight-running-process cap. No timer runs when
nothing is pending.
**Hooks run with your
rights, from `settings.toml`:** anyone who can write that file can run programs
as you. On Unix all settings saves use a private mode-0600 temporary file.
The daemon reads and checks the same open settings file, refusing hooks and `run:`
actions if the settings file or its containing folder is
group/world-writable or owned by a different uid. It logs each distinct problem
once. Symlinked settings check both the link's folder and the resolved file and
target folders back to the config directory, opening the resolved file with
`O_NOFOLLOW` and checking its handle rather than reopening it for the permission
gate. Refusals appear in `status` and `doctor`. Fix permissions, or deliberately
override this protection with `one-mk hooks --trust-permissions`; the explicit
opt-in is stored in a separate private owner-owned marker, not in the untrusted
settings file. `hooks --off` also removes that permission override.
Windows ACL checking is **not implemented**: keep the settings folder's ACL private.

Settings now save `settings_version = 2`. A legacy, unversioned
`clipboard.allow_pull = true` is treated as the old default, not explicit opt-in:
fetching is off. Status/doctor show the migration notice once per home, recording
only a private empty `.clipboard-fetch-update-noticed` marker (not changing the
settings). If the marker cannot be written, the notice can repeat. The next normal
settings save persists version 2; `one-mk clipboard --allow-fetch on` explicitly
enables fetching and remains enabled in version-2 settings.

**Screensaver and sleep.** `one-mk power --block-screensaver on`: while this
computer controls another one, that one's screensaver, display blanking and idle
sleep stay off, and it lets go the moment control ends (the pointer comes back,
its own input takes over, the connection drops). `one-mk power --sleep-sync on`
(off by default; set it on every computer that should join in): when this
computer's display goes to sleep, or it suspends, the connected computers with
sleep sync on put their displays to sleep too. Each sleep is passed on once and
never comes back. Waking isn't synced: any input wakes a display, and the
pointer arriving on a computer is input. Both computers need a build that has these
features (older peers are not asked). Limits: on Linux, keeping awake uses
`org.freedesktop.ScreenSaver` and a logind idle inhibitor (GNOME and KDE
answer), turning a display off uses GNOME's Mutter (else `xset dpms force off`
on X11), and noticing a display sleep works on GNOME (Mutter's `PowerSaveMode`)
plus logind's suspend signal anywhere; a suspend is announced only as fast as
the network lets it, before the machine is down. On Windows it is
`SetThreadExecutionState`, `SC_MONITORPOWER` and the console display state
notification; on macOS a `caffeinate` child and `pmset displaysleepnow`, and a
Mac's own display sleep is not noticed (it can follow others, not lead). Only
Linux has been tried against a real session; Windows and macOS compile and are
unit-tested only.

**Taking a computer back.** When someone else's pointer is on your computer,
you can take it back:

- On Windows and macOS, just use your computer's own mouse or keyboard.
- On Linux this doesn't work: the Wayland portals one-mk uses can't tell
  your own mouse and keyboard apart from the input one-mk injects, so one-mk
  can't notice you. Run `one-mk takeback` (or `one-mk switch here`, or press
  `Ctrl+Alt+H`) instead. Without the shortcut portal, bind it to a keyboard
  shortcut in your desktop's settings (GNOME: Settings > Keyboard > Custom
  Shortcuts; KDE: System Settings > Shortcuts).
- On every system, a computer that sends no input at all (no key, click,
  scroll or pointer movement) for a minute is let go by itself; its pointer
  goes home and its next movement comes back in. So a forgotten session
  never locks you out for long.

### Remapping keys and tuning the mouse

Rules change the input this computer sends to the others, for every
computer, for those running one OS (`--os mac`), or for one (`--peer
laptop`); the most specific wins. The other computers need nothing new.

```sh
one-mk keymap add capslock --tap esc --hold lctrl   # tap: Esc, hold: Ctrl
one-mk keymap add lalt lmeta --os mac               # Alt is Cmd on Macs
one-mk keymap add ctrl+h backspace                  # a combo
one-mk keymap add ralt layer:nav                    # hold AltGr for a layer...
one-mk keymap add h left --layer nav                # ...where h is Left
one-mk keymap add f13 'switch next'                 # an action (also lock-pointer, lock-all, run: CMD, scroll down)
one-mk keymap add f2 ctrl+a ctrl+c                  # a sequence
one-mk buttons preset side-back-forward             # side buttons: back/forward (Cmd+[ / Cmd+] on a Mac)
one-mk buttons map 3 button1                        # middle click is a left click
one-mk scroll --natural auto                        # natural on Macs, traditional elsewhere
one-mk scroll --modifier shift=horizontal --modifier alt=x5
one-mk scroll --device wheel --mode discrete --speed 2 --peer desk
one-mk pointer --speed 1.5 --peer laptop
one-mk keymap add ctrl+space f13 --os mac --app com.microsoft.VSCode   # only in VS Code on the Mac
one-mk pointer --speed 0.6 --app 'steam*'           # per app: also scroll and buttons
one-mk keymap check                                 # typos, conflicts, rules that never apply
```

Key names are case-insensitive with the usual aliases (`esc`, `ctrl`
= `lctrl`, `altgr`, `cmd` = `win` = `super` = `meta`, `f1`..`f24`, `kp0`,
media keys like `volume up`, `button4`), or `code:58` /
`evdev:KEY_CAPSLOCK`. The rules live under `[input]` in `settings.toml`
and apply at once, also in the middle of a visit; keys held while they
change are released as what they were pressed as.

Good to know:

- Keyboard shortcuts (`Ctrl+Alt+H`...) and `Ctrl+Alt+Shift+Esc` see the keys
  as you press them, before any remapping, so a keymap can't lock you out.
  Ctrl/Cmd swapping applies after the keymap.
- Nothing stays held: every key the keymap presses is released, also when
  the pointer comes back mid-press, a layer ends or the rules change.
- A Shift (or Alt) that changes scrolling is released on the other
  computer once you scroll with it, so it never sees Shift+scroll.
- `natural auto` assumes this computer scrolls its OS's default way
  (natural on a Mac); otherwise use `--reverse-vertical on`.
- Remote rules change only input sent to other computers. Local keyboard rules
  (`scope = { local = true }`, or `one-mk keymap add capslock esc --local`)
  have experimental Windows and partial macOS native adapters: **COMPILE-CHECKED
  ONLY, native Windows/macOS UNTESTED**, not production-accepted (5H incomplete).
  `--local --device NAME` stores a rule for one attached keyboard;
  local+device rules override local rules, never peer rules. The default-off
  `[input] local_rules` preference (`one-mk keymap local on|off`) opts in;
  local runtime status requires a healthy worker fence, not just the preference.
  That worker never gates core crossing/switching/hotkeys or repeat; off/empty/
  unsupported rules detach interception, and off/empty status is silent.
  macOS refuses Caps Lock/Fn/media/function-key rules; its CGEventPost receipt
  down-receipt path and modifier flags are natively UNTESTED. Release posts and
  capture handoff are best-effort, not proof of native delivery. Settings/device
  reloads can resume mapping; it is not explicit-reload-only. See `docs/LOCAL-REMAP.md` for
  platform restrictions, manual acceptance and emergency/SSH/permission recovery.
  Linux status says `local key rules are not available on Linux`; its
  Wayland portal only supplies keys while forwarding. Local rules never
  change keys sent to a peer. See `docs/INPUT-TUNING.md` for the safety gates.
- `--app NAME` rules follow the app that has the focus on the computer you
  are typing on: a program file on Windows (`chrome.exe`), a bundle id on a
  Mac (`com.apple.Terminal`), the window class on Linux (`firefox`); `*`
  matches anything, `!code` means every app but that one. That computer
  says which app it is (only to the computer controlling it, and only while
  rules need it), after a click or Alt+Tab. When it can't tell, the rule
  doesn't apply and `one-mk status` says so: Wayland windows on GNOME and
  KDE can't be named (X11 windows can), and older one-mk versions don't
  say.
- `--device NAME` rules are for **one keyboard or mouse** (on `keymap
  add/remove`, `buttons`, `pointer`, `scroll`): `one-mk keymap add capslock
  esc --device "Travel Keyboard"` makes Caps Lock Esc on that keyboard only,
  wherever it types; add `--peer laptop` for "that keyboard, when it types
  on the laptop". The rule is saved on the computer the device is plugged
  into (cable, Bluetooth or USB receiver) and applied there. `one-mk
  input-devices` lists the devices seen, also ones asleep right now, with
  the name or fingerprint to use. A device is known by what it is (vendor,
  product, name, serial or Bluetooth address), so its rules survive sleep,
  reconnecting and another USB port; two identical keyboards without a
  serial share their rules. This works in an **X11 session** (tested);
  on **Windows and macOS it is built but untested** on real hardware; on
  **GNOME, KDE and wlroots Wayland it is not available**: the desktop
  mixes all keyboards and mice together before one-mk sees them, and
  `input-devices`, `doctor` and the window say so. Input whose device
  isn't known only ever gets the rules without a device. A keyboard that
  falls asleep or is unplugged mid-press lets go of what it was pressing.
- Pointer acceleration: one-mk sends positions computed on this computer,
  so this computer's acceleration applies once. Turning it off or a custom
  curve needs raw mouse motion, which no backend reads yet; turn
  acceleration off in this computer's settings instead.

The window's **Keys & mouse** tab does all of this by clicking (and a key
press), and can try the rules before they are used. Details and the design:
[docs/INPUT-TUNING.md](docs/INPUT-TUNING.md).

## Why it feels good

- **Smooth mouse.** Every mouse report is forwarded immediately at the
  mouse's native rate, with no batching to 60 Hz.
- **No stutter on packet loss.** Motion travels as QUIC datagrams carrying
  absolute positions; a lost packet is replaced by the next one, so nothing
  is retransmitted and nothing waits.
- **Clicks land where you aimed.** Each click carries the pointer position
  it was made at.
- **Idle position repeats.** While the pointer is on another computer and
  not moving, its position is repeated every 20 ms. This tells the other side
  you are still there and tends to keep Wi-Fi radios from dozing (which can
  add noticeable stalls). It is a few bytes, and only while the pointer is
  remote.
- **Same speed on every screen.** Acceleration is applied once, by the
  computer the mouse is plugged into; positions are logical pixels, so mixed
  DPI and scaling work.
- **Autoscroll and gestures (opt-in, not hardware-qualified).**
  `one-mk autoscroll --on` makes middle-click toggle continuous scrolling
  on the other computer; physical Esc (consumed) or another click stops it. `one-mk gestures
  preset side-swipe` makes a held side button switch desktops when swiped;
  short clicks still click once. The Mouse view has toggle/speed and
  gesture preset/list controls. Primary left/right buttons cannot be reserved;
  attempted swipes never replay a click after leaving the small click slop.
  These modes are disabled during broadcast and do not support device scopes.
  Thresholds use backend motion units, not portable logical pixels.
  Only active autoscroll arms a timer; unrelated reloads preserve it.
  See [input tuning](docs/INPUT-TUNING.md#autoscroll-and-gesture-buttons-5f)
  for scopes, actions, precedence, troubleshooting and hardware acceptance.
  Linux null-backend routing and private-Xvfb capture/UI coverage; Windows/macOS compile-only, native
  desktop and Bluetooth behaviour untested.
- **No stuck keys.** Held keys are tracked and released when the pointer
  leaves, a computer disconnects, traffic stops for 750 ms, or control is
  taken back. However busy the network, these deadlines are checked first.
- **Light.** No CPU use while idle: nothing polls, and the command-line
  tools reach the running program through a local socket it only wakes for.
  Networking runs on one thread, with one thread each for input capture and
  injection.
- **Easy.** No server/client roles, no IP addresses (mDNS discovery), no
  certificates to copy, no root and no udev rules on Linux.

## The window (optional)

`one-mk-app` shows your computers standing on a desk, as they are arranged.
Open it from the tray ("Open one·mk…", or a left click on Linux) or run
`one-mk-app`. In it you can:

- drag a computer to the side it is really on (right-click it for above or
  below); the change applies at once, on both computers;
- pair a new computer: the code appears on both screens of the desk and in
  a panel with **Codes match, pair** and **They don't match**;
- drop files or folders onto a computer to send them;
- see how fast the connection is (measured, not estimated: click a computer
  to measure again);
- watch the connection to each computer live: a small graph of the last two
  minutes under the selected computer on the Desk tab, and one line per
  computer on Health. It shows the time an answer takes now, the lowest,
  average and highest, and says **Smooth**, **Some stalls** (more than 1 in 20
  readings over 40 ms, or any lost packet) or **Slow** (more than 1 in 20 over
  100 ms, or 8 lost packets); amber means it is worth a look, and the word
  says the same without colour. An offline computer says "offline" and has no graph. The
  numbers come free from the connection itself: one-mk sends nothing extra
  and runs no timer for it. The window asks once a second, only while a
  graph is on screen (not on other tabs, minimised or closed), and keeps
  120 readings per computer in memory, nothing on disk;
- move the mouse and keyboard to a computer (**Switch to it**), send it this
  clipboard or get its clipboard, keep the pointer here and lock every
  screen;
- **Layout:** every computer's real monitors as a monitor wall; drag a
  computer next to one of your monitors and it snaps there, crossing only
  where the screens touch (the placement syncs to the other computer).
  Choose a side to let it cross anywhere along that side again. Save, use
  and delete layout profiles, and let one switch in by itself on a Wi-Fi
  network;
- **Clipboard & files:** what the clipboard shares and whether others may
  get it, notifications, the received files folder, the clipboard's size
  limit, and each transfer with a progress bar;
- **Keys & mouse:** a drawn keyboard: click a key (or move to it with the
  arrows and press Enter), then press the key it should become, pick it from
  a searchable list, or make it do nothing; "hold for another key" is under
  More. Changed keys are marked on the drawing and listed as sentences
  ("Caps Lock → Esc when tapped, Ctrl when held"), with quick setups (Caps
  Lock as Esc/Ctrl, swap Alt and Super, swap Ctrl and Cmd for Mac keyboards,
  arrow keys on Right Alt + HJKL), problems from `one-mk keymap check`, and
  **For:** all computers, one OS or one computer. A **Test it** box runs your
  rules right there (nothing is sent) and shows what the other computer would
  get, and whether a tap/hold key counted as a tap or a hold. On the Mouse
  side: what each button does (another button, a key or shortcut, switching
  computers, locking, a media key, a command), pointer speed, and scrolling
  (direction, speed, steps, natural scrolling, Shift to scroll sideways).
  Acceleration Off and Curve show why they can't be used yet;
- **Health:** the results of `one-mk doctor` (run when the page opens and on
  **Recheck**), each with its fix and a button to copy the command (or, for
  starting and pairing, the window's own button), plus what one-mk says
  right now (for example a permission still to approve) and the live
  connection to each computer;
- **Advanced:** settings most people never change: edge rules (not crossing
  near corners, waiting at the edge, a key to hold, staying in fullscreen
  windows), keeping controlled computers awake, sleep and lock sync, gaming
  mode, keyboard shortcuts on or off, Ctrl/Cmd swapping per computer, and
  commands to run on events (edit them in `settings.toml`).

Every change goes through the same `one-mk` commands you would type, so
the command line and the window always agree. Ctrl+1 to Ctrl+6 switch
tabs.

When the mouse and keyboard move to another computer, the pointer is drawn
crossing to it. The window only talks to the running `one-mk`, so closing
it changes nothing, and it costs nothing when it is not open. It must sit
in the same folder as `one-mk`.

## Tray icon (optional)

`one-mk-tray` is a small separate program that puts a one-mk icon in the
system tray (menu bar on macOS). one-mk works the same without it; it only
saves you typing commands. Start it by running `one-mk-tray` (or turn on
"Start at login" in its menu). It must sit in the same folder as `one-mk`.
The menu shows:

- whether one-mk is running and who it is connected to ("Connected to desk
  (on your right)");
- each paired computer, with a submenu to move it left, right, above or
  below (applied at once, like `one-mk layout`);
- a line for each file transfer that is running ("Receiving 3 files from
  desk: 45%");
- **Switch to <computer>** and **Send clipboard to <computer>** for each
  connected computer (**Switch back here** while another one has control);
- **Open one·mk…**, which opens the window;
- **Broadcast to all computers** (a checkmark; while a computer is
  connected), and the headline "Broadcasting to N computers" while it is on;
- **Keep the pointer here** (a checkmark), **Lock every screen**, **Take
  back control**, **Start one·mk** (when it is not running), **Pair a
  computer…** (when none is paired; it opens the window) and **Start at
  login**;
- **Quit tray**, which closes only the tray, not one-mk.

On Windows `one-mk install` then starts the tray at login and the tray
starts one-mk hidden, so no console window appears (without the tray, the
login entry runs `one-mk run` and shows one). On Linux and macOS
`one-mk install` also starts the tray at login if it is installed. Use
`one-mk-tray --home <dir>` for a non-default settings folder.

It is light: no window, no web view, and it never polls. It sleeps until
the system wakes it, and the cost is a few MB of memory.

- **Linux** uses the StatusNotifierItem protocol directly (no GTK). KDE
  Plasma shows it natively; GNOME needs the "AppIndicator and KStatusNotifierItem
  Support" extension (preinstalled on Ubuntu). The menu is refreshed each time
  you open it. The icon colour (pink connected, grey not) updates at those
  moments too.
- **Windows** refreshes when the pointer reaches the icon.
- **macOS** cannot tell us when the menu opens, so while one-mk is running
  the tray looks at it every 5 seconds (one tiny local request); it does
  nothing when one-mk is not running.

## Security

- Mutual TLS 1.3 over QUIC. Computers are pinned by certificate fingerprint
  at pairing, using a commit-then-reveal numeric code (like Bluetooth) that
  defeats man-in-the-middle attacks. Always compare the code on both screens.
- Only paired computers can connect; others are rejected.
- Received files can't escape the download folder or overwrite anything, and
  every file is checked with BLAKE3.
- The control channel for `one-mk status`/`layout`/`takeback` is private to your user:
  a Unix socket with mode 0600 on Linux and macOS; on Windows a localhost
  port protected by a random token in a file in your private local
  settings folder.
- Settings and the private key live in your user's config folder
  (`~/.config/one-mk` on Linux).
- Hooks (`one-mk hooks`) run commands with your rights from `settings.toml`;
  keep that file writable by you only (one-mk warns if it isn't), and prefer
  the array form, which never goes near a shell.

## Measure it

```bash
one-mk bench                                # loopback
one-mk bench --serve                        # on machine B
one-mk bench --connect 192.168.1.20:24901   # on machine A
```

## Troubleshooting

Start with `one-mk doctor`. It checks that one-mk is running and paired,
the port, discovery, the firewall, the desktop permission (Linux), the
keyboard shortcuts, broadcast, the pointer drawn on a computer with no mouse, the
network and how fast each connected computer answers, and prints a plain fix
for each problem.
It only reads: it never changes your firewall or settings, needs no root,
and exits with status 1 if something failed.

- **Computers don't find each other.** Both must be on the same network
  (not a guest or "client isolation" Wi-Fi). Allow one-mk through the
  firewall: **UDP port 24900** (and mDNS, UDP 5353, for discovery). On
  Windows, accept the firewall prompt the first time. Without discovery,
  connect directly: `one-mk run --connect 192.168.1.20:24900`, or
  `one-mk pair --connect <ip>:<port>` as `pair` prints.
- **`one-mk status` says it is not running.** Start it with `one-mk run`, or
  use `one-mk install` so it starts at login. On Linux with systemd, the log
  is `journalctl --user -u one-mk`.
- **The desktop asks for permission (Linux).** On Wayland, one-mk prints
  "Waiting for your approval: ..." while the desktop's dialog is open, and
  waits as long as you need. The remote-control permission is remembered
  (`remote-desktop.token` in the config folder, readable by you only), so it
  is asked once. GNOME 50 can't remember the input-capture permission yet,
  so that dialog comes up each time one-mk starts (KDE, and newer desktops
  with InputCapture version 2, remember it in `input-capture.token`). If
  you declined, `one-mk status` says so; restart one-mk to be asked again.
  If the desktop no longer takes the saved permission, one-mk deletes it
  and asks again by itself.
- **The pointer or a key seems stuck (Linux).** Press Ctrl+Alt+Shift+Esc.
  one-mk also lets go by itself whenever the desktop session drops (portal
  restart, session bus restart, a monitor change that closes the session),
  when the screen locks and before the computer sleeps; it then reconnects
  in the background (after 1, 2, 4 ... up to 30 seconds). `one-mk status`
  shows a line while it is reconnecting or waiting for you. With the X11
  and wlroots backends, stopping one-mk in any way (even `kill -9`) lets go
  of the pointer and keyboard; on X11, keys a killed one-mk was pressing on
  a peer's behalf stay down until one-mk starts again, which lets go of
  them.
- **A rule for one keyboard or mouse does nothing.** Run `one-mk doctor`:
  its "Per-device rules" check says whether this desktop can tell devices
  apart (not on GNOME, KDE or wlroots Wayland: log in to an X11 session to
  use them). Then `one-mk input-devices`: the device must be listed as
  connected, and `keymap list` must show the rule for it ("for the device
  ..."). Two devices that can't be told apart (same model, no serial; on
  Windows, devices on one USB receiver) share their rules; the list says
  so. A one-mk older than this feature can't read a `settings.toml` with
  such rules: remove them (`--device ...` on `keymap remove` and the
  others) before going back to an older version.
- **macOS.** Allow one-mk under System Settings > Privacy & Security >
  Accessibility and Input Monitoring, then start it again. (Not yet tested on
  real hardware.)
- **The mouse stutters or lags on another computer.** Look at the graph on
  the window's Desk or Health tab, or run `one-mk status` (`rtt 4 ms` per
  connected computer) and `one-mk doctor` ("Connection quality"). Under about
  40 ms is smooth. Higher, or **Some stalls** with lost packets, is the network
  between the two computers, usually Wi-Fi: move closer to the router, use the
  5 GHz band or a cable, and pause big downloads. `one-mk bench --connect`
  measures the pointer's whole path.
- **A paired computer is on the wrong side.** `one-mk layout "<name>" <side>`.
- **The pointer jumps to the other side of the screen.** Pointer wrap is
  on: `one-mk edge --wrap off`, or `--wrap on` to wrap only the left and
  right edges, or add a dwell (`one-mk edge --dwell 300`) so a quick flick
  to an edge doesn't wrap.
- **"Is laptop on your left?" but it isn't.** Choose No in the window, or
  `one-mk edge --dismiss`; place it where it really is (`one-mk layout`) and
  it is never asked about again. `one-mk edge --learn off` stops the
  questions. If `learn = auto` moved it, the notification and the daemon's
  output give the command that puts it back.
- **The pointer won't cross part of an edge.** With an offset, only where
  the other computer's screens touch yours leads there (`one-mk status` and
  `one-mk devices` show the offset). `--offset none` makes all of the edge
  lead there again.
- **What I type appears on every computer.** Broadcast is on: `one-mk status`
  starts with "Broadcasting to ...". Stop it with `Ctrl+Alt+B`,
  `Ctrl+Alt+Shift+Esc` or `one-mk broadcast off` (from another terminal or
  over SSH: this computer's own mouse and keyboard are captured meanwhile, so
  the window and tray can't be clicked). It also stops by itself after 15
  minutes without input. If `Ctrl+Alt+B` does nothing, another shortcut may
  already use it: `one-mk hotkeys` says so; set another with
  `one-mk hotkeys --broadcast 'ctrl+alt+shift+b'`.
- **Broadcast says no computer is connected.** It needs at least one paired
  computer online (`one-mk status`); see "Computers don't find each other".
- **Another computer already controls this one.** Only one computer can
  control another at a time. Take it back with `one-mk takeback` on the
  controlled computer (on Windows and macOS, using its mouse or keyboard
  works too), or wait a minute without input from the other one.

## Layout of the code

```
crates/proto      wire format + pure pointer/edge/key logic (no I/O, unit-tested)
crates/net        identity, QUIC, pairing, trusted devices, mDNS discovery
crates/input      platform input: Linux (XDG portals + libei, wlroots, X11), Windows, macOS
crates/clipboard  system clipboard on its own thread (arboard)
crates/ctl        client for the daemon's control channel, status format, login settings
crates/tray       one-mk-tray: the optional tray icon
crates/app        one-mk-app: the optional window (egui)
docs/brand        logo, app and tray icons, colours, brand guideline
crates/one-mk     the daemon and CLI
```

## Building

```bash
cargo build --release      # target/release/one-mk, one-mk-tray and one-mk-app
cargo test
```

On Linux you need GNOME 45+ or KDE Plasma 6.1+ on Wayland, a wlroots
compositor, or an X11 session (see [Supported desktops](#supported-desktops)). Clipboard sharing
uses the Wayland data-control protocol where the compositor has it (KDE,
wlroots) and X11 otherwise; GNOME has no Wayland clipboard access for
background apps, so there it goes through XWayland.

## Roadmap

1. Done: Linux to Linux mouse and keyboard, pairing, discovery, benchmark
2. Windows backend: written and unit-tested; needs testing on real machines
   (service for the UAC and login screens is still to do)
3. macOS backend: written and unit-tested; needs testing on real machines
4. Done: clipboard (text, HTML, images), file transfer, live layout sync,
   Ctrl/Cmd swapping, start at login
5. Done: a small tray icon (status, sides, take back, start at login).
   Done: a drag-to-place layout editor in the window (each computer's real
   monitors; drop a computer beside a monitor and it crosses only where the
   screens touch), offsets (`one-mk layout --offset DX,DY`), layout
   profiles that switch by Wi-Fi network. **Next:** "push toward it"
   layout learning
6. Done: gaming mode (relative motion), switching without an edge, edge
   rules, lock sync, connection doctor (`one-mk doctor`), keyboard shortcuts
   (on Linux through the GlobalShortcuts portal). **Next:** auto-update

## Updating

`one-mk update --check` checks for a signed release; `one-mk update` explicitly
installs it for a recognized portable installation. Package-managed installs
show instructions instead; the updater never elevates or restarts the daemon.
`one-mk update --rollback` restores the previous portable executable. Restart
manually afterward with `one-mk stop && one-mk run`.

Startup checks run at most daily, once at startup, with no background timer.
Disable them with `one-mk update --check-on-start off`. The window shows cached
availability and an Update / Release notes banner. **Publisher key setup is
required before this feature can contact release servers or install anything**;
the committed placeholder is deliberately disabled. See [signed updates](docs/UPDATES.md)
for signing, safety, troubleshooting and platform validation limitations.

## Contributing and license

Issues and pull requests are welcome. Run `cargo test` before you open a
PR; CI runs it on Linux, macOS and Windows.

one-mk is licensed under either of [Apache-2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option. Unless you say otherwise, any
contribution you submit is licensed the same way, with no extra terms.

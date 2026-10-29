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

## Quick start

Get it first. Every release has a package for each system (the
[latest release](https://github.com/samer2373/one-mk/releases/latest)):

| System | Get it | Notes |
|---|---|---|
| Debian, Ubuntu | `sudo apt install ./one-mk_*_amd64.deb` | Installs nothing into your login; run `one-mk install` as yourself |
| Any Linux | `one-mk-*-x86_64.AppImage` | `chmod +x` it and run it; with no arguments it opens the window, otherwise it is `one-mk` |
| macOS | `one-mk-*-arm64.dmg` or `-x64.dmg` | Drag `one-mk.app` to Applications; or `brew install --cask samer2373/tap/one-mk` |
| Windows | `one-mk-*-x64-setup.exe` | Installs to Program Files, opens the firewall for private networks, optionally starts at sign-in |
| macOS, Linux, Windows | the plain `.tar.gz` / `.zip` | `curl -fsSL https://raw.githubusercontent.com/samer2373/one-mk/main/install.sh \| sh` installs it into `~/.local/bin` |

Or build it yourself (see Building). How each package is made, and which are
untested, is in [docs/PACKAGING.md](docs/PACKAGING.md).

Install once, pair once, done. On **each** computer:

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

Screens of different sizes, or not lined up? Say where the other edge
starts, in pixels (see [Arranging the computers](#arranging-the-computers)):

```bash
one-mk layout "laptop" bottom --offset 300   # its left edge 300 px right of yours
one-mk layout "laptop" bottom --offset center
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

X11 barriers (or fallback edge windows) and wlroots edge surfaces cover only
the spans facing a peer, clipped to each outer monitor boundary. A wall beside
a partially overlapping peer stays local, including after re-arming.

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
| `one-mk status` | Is it running? Who is paired, online, where, and who controls whom |
| `one-mk takeback` | Take this computer back from the one controlling it |
| `one-mk switch <pc\|here\|next>` | Move the mouse and keyboard to another computer without crossing an edge (`here`: back to this one) |
| `one-mk lock-pointer on\|off\|toggle` | Keep the pointer on this computer: edges lead nowhere |
| `one-mk hotkeys [options]` | Show or change the keyboard shortcuts (see below) |
| `one-mk edge [options]` | Rules for crossing edges (see below) |
| `one-mk gaming on\|off\|auto` | Raw mouse movement for games on another computer |
| `one-mk lock-all` | Lock this computer's screen and every connected one's |
| `one-mk lock-sync on\|off` | Locking any computer locks the others (those with lock sync on) |
| `one-mk hooks [--on-enter CMD] [--on-leave CMD] [--on-peer-connect CMD] [--on-peer-disconnect CMD] [--off]` | Run a command when the pointer arrives or leaves, or a computer connects or disconnects (see below) |
| `one-mk power [--block-screensaver on\|off] [--sleep-sync on\|off]` | Keep other computers awake while you control them; sleep displays together (see below) |
| `one-mk keymap [list\|add\|remove\|clear\|check]` | Change what keys do on the other computers: remaps, combos, tap/hold, layers, actions (see below) |
| `one-mk buttons [list\|map N TO\|preset side-back-forward\|clear]` | Remap mouse buttons on the other computers |
| `one-mk pointer [--speed X]` / `one-mk scroll [options]` | Pointer speed; scroll direction, natural scrolling, speed, notches, Shift-scroll |
| `one-mk install` / `uninstall` | Start automatically at login (and add the file manager's "Send to" entries) / stop doing that |
| `one-mk integrate [--remove]` | Add or remove "Send to <computer>" in the file manager only (see below) |
| `one-mk pair` | Pair with another computer (run on both) |
| `one-mk layout <pc> <left\|right\|top\|bottom> [--offset PX\|center\|stretch]` | Say where a paired computer sits, and how its edge lines up with yours |
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
Content a password manager
marks as sensitive is skipped (`--skip-sensitive on`, the default), and never
handed out by `clip-get`; `clip-send` is you asking for it, so it sends.
**Limitation:** the clipboard library one-mk uses cannot read those markers
(`x-kde-passwordManagerHint`, `org.nspasteboard.ConcealedType`,
`ExcludeClipboardContentFromMonitorProcessing`) from the system clipboard, so
nothing is detected as sensitive on a real computer yet. The skip is in place
and tested, and waits for a way to read the markers.

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

Each paired computer sits on one side of this one. By default its whole
facing edge is stretched over yours: wherever you leave your screen, you
arrive at the same fraction of the way along its edge, whatever the two
sizes.

With `--offset` the two edges sit against each other pixel for pixel, like
two monitors of one computer, and the pointer only crosses where they
really overlap. Next to the overlap the edge is a wall, as beside a real
monitor:

```bash
one-mk layout laptop bottom --offset 300    # laptop below, its left edge 300 px right of yours
one-mk layout laptop right --offset -200    # laptop on the right, its top edge 200 px above yours
one-mk layout laptop right --offset center  # the two edges centred on each other
one-mk layout laptop --offset stretch       # back to stretching (the side stays)
```

Moving a computer to another side without `--offset` starts it stretched
again (an offset means something else on another side).

The offset is in logical pixels (what your display settings call the
resolution after scaling), measured from the start of your edge to the start
of the other: the top for left and right, the left end for top and bottom;
negative means before yours. Offsets must be between **-100000 and 100000**
logical pixels; an empty value is rejected (use `stretch` explicitly).
The other computer gets the mirror image
(`--offset 300` here is `-300` there), and as with sides the newer change
wins. Both computers need this version of one-mk for the offset to work
both ways: an older one keeps stretching the edges for the pointer it sends.

Several computers can share one side if their edges don't overlap, for
example two laptops below a wide monitor:

```bash
one-mk layout laptop-a bottom --offset 0
one-mk layout laptop-b bottom --offset 1920
```

**Several monitors.** Only the outer edge of the whole desktop leads to
another computer, and only where a monitor actually reaches it: with an
L-shaped arrangement, or a smaller monitor beside a taller one, the part of
the edge with no monitor on it leads nowhere. The pointer arrives on the
monitor of the other computer that faces the point you left from, whichever
monitor that is, and comes back to the same point.

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
back at once, and typing or clicking there goes to this computer. A click
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

**Gaming mode.** Games capture the pointer and want raw movement, not
positions. With `one-mk gaming on`, while the pointer is on another computer
the mouse sends relative movement, which never stops at that computer's
screen edges, so it doesn't lead back either: come back with
`one-mk switch here` (or the emergency shortcut). `auto` does this only while
the other computer has a fullscreen window in front, where it can tell (see
above). Both computers need this version of one-mk; with an older one,
positions are sent as before. On Windows, the pointer speed setting also
applies to these movements outside games.

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
- Only input sent to other computers is changed: on Linux one-mk sees your
  keyboard only while it forwards it (local remapping is planned for
  Windows and macOS).
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
- move the mouse and keyboard to a computer (**Switch to it**), send it this
  clipboard or get its clipboard, keep the pointer here and lock every
  screen;
- **Layout:** see every computer's real monitors on a flat plan; drag one
  along the edge it shares with this one to line them up (it snaps to the
  ends and the middle), or type an offset in pixels, or stretch the edges
  end to end. The part of the edge the pointer crosses is drawn in amber,
  dashed where the pointer stops. Save, use and delete layout profiles, and let one
  switch in by itself on a Wi-Fi network;
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
  right now (for example a permission still to approve);
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
keyboard shortcuts and the network, and prints a plain fix for each problem.
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
- **macOS.** Allow one-mk under System Settings > Privacy & Security >
  Accessibility and Input Monitoring, then start it again. (Not yet tested on
  real hardware.)
- **A paired computer is on the wrong side.** `one-mk layout "<name>" <side>`.
- **The pointer won't cross part of an edge.** With an `--offset`, only the
  part facing the other computer's screen leads there (`one-mk status` shows
  the offset). `--offset stretch` makes all of the edge lead there again.
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
   Done: offsets and per-monitor crossing, layout profiles that switch by
   Wi-Fi network. **Next:** a drag-and-drop layout editor and "push toward
   it" layout learning
6. Done: gaming mode (relative motion), switching without an edge, edge
   rules, lock sync, connection doctor (`one-mk doctor`), keyboard shortcuts
   (on Linux through the GlobalShortcuts portal). **Next:** auto-update

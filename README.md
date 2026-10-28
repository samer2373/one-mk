<img src="assets/icon.svg" width="64" height="64" alt="">

# one-mk

Share one mouse and keyboard across your Linux, Windows and Mac computers.
Move the pointer off the edge of one screen and it appears on the next. The
clipboard and files follow. Setup takes about a minute.

> **Status: early.** Linux (Wayland: GNOME 45+ or KDE Plasma 6.1+) is tested
> end to end on real hardware. The Windows and macOS backends compile and
> are unit-tested, but have **not yet run on real Windows or macOS
> machines**: expect rough edges there.

## Quick start

Get it first. On macOS and Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/samer2373/one-mk/main/install.sh | sh
```

On Windows, download the `.zip` from the
[latest release](https://github.com/samer2373/one-mk/releases/latest) and
unzip it. Or build it yourself (see Building).

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
soon as they connect.

## Everyday use

| Command | What it does |
|---|---|
| `one-mk status` | Is it running? Who is paired, online, where, and who controls whom |
| `one-mk takeback` | Take this computer back from the one controlling it |
| `one-mk install` / `uninstall` | Start automatically at login / stop doing that |
| `one-mk pair` | Pair with another computer (run on both) |
| `one-mk layout <pc> <left\|right\|top\|bottom>` | Say where a paired computer sits |
| `one-mk layout <pc> --swap-ctrl-cmd on\|off\|auto` | Ctrl/Cmd swapping (see below) |
| `one-mk send <files or folders> [--to <pc>]` | Send files; they arrive in `Downloads/one-mk` |
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
`one-mk run --no-clipboard`).

**Files.** `one-mk send photos/ notes.pdf` goes to your only paired computer
(use `--to` if you have several). They land in `Downloads/one-mk` there
(change with `one-mk run --downloads <dir>`). An interrupted transfer resumes
when you send the same file again.

**Mac and PC keyboards.** When keys go between a Mac and a Windows or Linux
computer, Ctrl and Cmd trade places by default, so Ctrl+C on the PC keyboard
copies on the Mac and the other way round. The sending computer does the swap.
Set it per computer with `--swap-ctrl-cmd on`, `off`, or `auto` (the
default: swap only between a Mac and something else). It is stored
for each device on the computer where you set it.

**Emergency:** `Ctrl+Alt+Shift+Esc` always brings the pointer back to the
computer you are sitting at.

**Taking a computer back.** When someone else's pointer is on your computer,
you can take it back:

- On Windows and macOS, just use your computer's own mouse or keyboard.
- On Linux this doesn't work: the Wayland portals one-mk uses can't tell
  your own mouse and keyboard apart from the input one-mk injects, so one-mk
  can't notice you. Run `one-mk takeback` instead. It's worth binding it to
  a keyboard shortcut in your desktop's settings (GNOME: Settings >
  Keyboard > Custom Shortcuts; KDE: System Settings > Shortcuts).
- On every system, a computer that sends no input at all (no key, click,
  scroll or pointer movement) for a minute is let go by itself; its pointer
  goes home and its next movement comes back in. So a forgotten session
  never locks you out for long.

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
- choose Ctrl/Cmd swapping per computer, and turn on start at login.

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
- **Open one·mk…**, which opens the window;
- **Take back control**, **Start one-mk** (when it is not running) and
  **Start at login**;
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

## Measure it

```bash
one-mk bench                                # loopback
one-mk bench --serve                        # on machine B
one-mk bench --connect 192.168.1.20:24901   # on machine A
```

## Troubleshooting

Start with `one-mk doctor`. It checks that one-mk is running and paired,
the port, discovery, the firewall, the desktop permission (Linux) and the
network, and prints a plain fix for each problem. It only reads: it never
changes your firewall or settings, needs no root, and exits with status 1 if
something failed.

- **Computers don't find each other.** Both must be on the same network
  (not a guest or "client isolation" Wi-Fi). Allow one-mk through the
  firewall: **UDP port 24900** (and mDNS, UDP 5353, for discovery). On
  Windows, accept the firewall prompt the first time. Without discovery,
  connect directly: `one-mk run --connect 192.168.1.20:24900`, or
  `one-mk pair --connect <ip>:<port>` as `pair` prints.
- **`one-mk status` says it is not running.** Start it with `one-mk run`, or
  use `one-mk install` so it starts at login. On Linux with systemd, the log
  is `journalctl --user -u one-mk`.
- **GNOME asks for permission.** On Wayland, GNOME shows a dialog the first
  time asking to allow input capture and remote control. Allow it. If you
  dismissed it, run `one-mk run` again.
- **macOS.** Allow one-mk under System Settings > Privacy & Security >
  Accessibility and Input Monitoring, then start it again. (Not yet tested on
  real hardware.)
- **A paired computer is on the wrong side.** `one-mk layout "<name>" <side>`.
- **Another computer already controls this one.** Only one computer can
  control another at a time. Take it back with `one-mk takeback` on the
  controlled computer (on Windows and macOS, using its mouse or keyboard
  works too), or wait a minute without input from the other one.

## Layout of the code

```
crates/proto      wire format + pure pointer/edge/key logic (no I/O, unit-tested)
crates/net        identity, QUIC, pairing, trusted devices, mDNS discovery
crates/input      platform input: Linux (XDG portals + libei), Windows, macOS
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

On Linux you need GNOME 45+ or KDE Plasma 6.1+ on Wayland. Clipboard sharing
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
   **Next:** a drag-and-drop layout editor and "push toward it" layout learning
6. Done: connection doctor (`one-mk doctor`). **Next:** gaming mode (relative motion), auto-update

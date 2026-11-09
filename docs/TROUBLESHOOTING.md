# Troubleshooting

Start with `one-mk doctor`, or open **Health** in the window (it runs the same checks and gives
each problem a fix button). It checks that oneMK is running and paired, the port, discovery, the
firewall, the desktop permission (Linux), the keyboard shortcuts, broadcast, the pointer drawn on a
computer with no mouse, a layout the pointer cannot cross, a computer that removed this one, the
network, and how fast each connected computer answers, and prints a plain fix for each problem.
It only reads: it never changes your firewall or settings, needs no root, and exits with status 1
if something failed. Checks that do not apply to you are left out; `one-mk doctor --all` shows
them. `one-mk doctor --json` is for programs.

When you report a bug, attach the output of `one-mk doctor` from each computer involved, the
operating system and desktop, and `one-mk --version`.

## Connecting

- **Computers don't find each other.** Both must be on the same network (not a guest or "client
  isolation" Wi-Fi). Allow oneMK through the firewall: **UDP port 24900** (and mDNS, UDP 5353, for
  discovery). On Windows, accept the firewall prompt the first time. Without discovery, connect
  directly: `one-mk run --connect 192.168.1.20:24900`, or `one-mk pair --connect <ip>:<port>` as
  `pair` prints.
- **`one-mk status` says it is not running.** Start it with `one-mk run`, or use `one-mk install`
  so it starts at login. On Linux with systemd the log is `journalctl --user -u one-mk`.
- **A computer shows "removed this computer".** You forgot it here, or it forgot this one. Forget it
  on the other side too (`one-mk forget`), or pair again. oneMK retries such a computer only
  every 10 minutes.
- **"The path of the settings folder is too long".** oneMK's control socket lives in the settings
  folder and a Unix socket path is limited to about 100 characters; use a shorter `--home` (or
  `ONE_MK_HOME`).
- **Another computer already controls this one.** Only one computer can control another at a
  time. Take it back with `one-mk takeback` on the controlled computer (on Windows, macOS and X11,
  using its own mouse or keyboard works too), or wait: a controlled computer that receives no input is
  let go after 10 minutes by default (`one-mk edge --idle-return 30m` to change it).
- **Settings cannot be read because the file is too large.** `settings.toml` is limited to
  4 MiB. Reduce it before retrying; oneMK leaves the original untouched. A single bad line costs
  only its own section or rule: the rest is read and the line is named in `status`, `doctor` and
  `keymap check`.

## Permissions

- **The desktop asks for permission (Linux).** On Wayland, oneMK prints "Waiting for your
  approval: ..." while the desktop's dialog is open, and waits as long as you need. The
  remote-control permission is remembered (`remote-desktop.token` in the config folder, readable
  only by you), so it is asked once. GNOME 50 can't remember the input-capture permission yet, so
  that dialog comes up each time oneMK starts (KDE and newer desktops with InputCapture version 2
  remember it in `input-capture.token`). If you declined, `one-mk status` says so; restart oneMK to
  be asked again. If the desktop no longer takes the saved permission, oneMK deletes it and asks
  again by itself.
- **macOS.** Allow oneMK under System Settings > Privacy & Security > Accessibility and Input
  Monitoring, then start it again. The window's guide walks through it
  ([PERMISSION-GUIDE.md](PERMISSION-GUIDE.md)). macOS support has not been tried on real hardware.

## Pointer and keys

- **The pointer or a key seems stuck (Linux).** Press `Ctrl+Alt+Shift+Esc`. oneMK also lets go by
  itself whenever the desktop session drops (portal restart, session bus restart, a monitor change
  that closes the session), when the screen locks and before the computer sleeps; it then reconnects
  in the background (after 1, 2, 4 ... up to 30 seconds), and `one-mk status` shows a line while it
  is reconnecting or waiting for you. With the X11 and wlroots backends, stopping oneMK in any way
  (even `kill -9`) lets go of the pointer and keyboard; on X11, keys a killed oneMK was pressing on a
  peer's behalf stay down until oneMK starts again, which lets go of them.
- **The mouse stutters or lags on another computer.** Look at the graph on the window's Desk or
  Health tab, or run `one-mk status` (`rtt 4 ms` per connected computer) and `one-mk doctor`
  ("Connection quality"). Under about 40 ms is smooth. Higher, or **Some stalls** with lost packets,
  is the network between the two computers, usually Wi-Fi: move closer to the router, use the 5 GHz
  band or a cable, and pause big downloads. `one-mk bench --connect` measures the pointer's whole
  path.
- **A paired computer is on the wrong side.** `one-mk layout "<name>" <side>`, or drag it in
  **Desk > Arrange**.
- **The pointer jumps to the other side of the screen.** Pointer wrap is on: `one-mk edge --wrap off`,
  or `--wrap on` to wrap only the left and right edges, or add a dwell (`one-mk edge --dwell 300`) so a
  quick flick to an edge doesn't wrap.
- **"Is laptop on your left?" but it isn't.** Choose No in the window, or `one-mk edge --dismiss`;
  place it where it really is (`one-mk layout`) and it is never asked about again.
  `one-mk edge --learn off` stops the questions.
- **The pointer won't cross part of an edge.** With an offset, only where the other computer's screens
  touch yours leads there (`one-mk status` and `one-mk devices` show the offset). `--offset none` makes
  all of the edge lead there again. If none of its screens touch yours, `layout`, `status` and
  `doctor` warn about it.
- **What I type appears on every computer.** Broadcast is on: `one-mk status` includes "Broadcasting
  to ...". Stop it with `Ctrl+Alt+B`, `Ctrl+Alt+Shift+Esc` or `one-mk broadcast off` (from another
  terminal or over SSH: this computer's own mouse and keyboard are captured meanwhile, so the window
  and tray can't be clicked). It also stops by itself after 15 minutes without input. If
  `Ctrl+Alt+B` does nothing, another shortcut may already use it: `one-mk hotkeys` says so.
- **Broadcast says no computer is connected.** It needs at least one paired computer online
  (`one-mk status`).
- **A rule for one keyboard or mouse does nothing.** `one-mk doctor`'s "Per-device rules" check says
  whether this desktop can tell devices apart (not on GNOME, KDE or wlroots Wayland: log in to an X11
  session to use them). Then `one-mk input-devices`: the device must be listed as connected, and
  `keymap list` must show the rule for it. Two devices that can't be told apart (same model, no
  serial; on Windows, devices on one USB receiver) share their rules. A oneMK older than this feature
  can't read a `settings.toml` with such rules: remove them first.
- **A per-app rule never applies.** The controlled computer cannot name its focused app (Wayland
  windows on GNOME and KDE); `one-mk status` says so.
- **Wrong letters or AltGr characters on the other computer.** Typing forwards physical keys by
  default, so the receiving OS chooses the letters. Select the same keyboard layout on both computers and
  check the receiving OS's Right Alt/AltGr mapping. `one-mk typing --keys characters|auto` (experimental)
  sends the printed letters, but only toward Windows and Mac computers, and has not been tried on real
  hardware. Japanese kana and IME composition are not translated.

## Clipboard, files, wallpaper

- Clipboard and history problems: [CLIPBOARD.md](CLIPBOARD.md#troubleshooting).
- File transfers, partial files: [FILES.md](FILES.md).
- Wallpaper messages ("This picture cannot be opened", "Install GStreamer ...", your own wallpaper
  did not come back): [LIVE-WALLPAPER.md](LIVE-WALLPAPER.md#troubleshooting).
- Update problems ("update server returned HTTP 403", "release downgrade refused"):
  [UPDATES.md](UPDATES.md).

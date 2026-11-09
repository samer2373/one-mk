# Command reference

Every `one-mk` command, with its options, exactly as `--help` prints them. This page is
generated from the built program (`one-mk --help` and each command's `--help`) for 0.1.29 on
Linux, so it cannot drift from the code; run `one-mk <command> --help` on your computer for the
version you have. Commands that exist only on Windows and macOS builds (for example
`keymap local` and `keymap add --local`, the experimental local key rules) do not appear in
Linux help.

Every command also accepts `--home DIR` (where settings and keys live; also the environment
variable `ONE_MK_HOME`) and `-v` / `-vv` (more detail). `--port`, `--connect`, `--no-discovery`,
`--backend` and the other service options belong to `one-mk run`; give them after `run`.
Guides: [README](../README.md), [ARRANGING.md](ARRANGING.md), [INPUT.md](INPUT.md),
[CLIPBOARD.md](CLIPBOARD.md), [FILES.md](FILES.md), [AUTOMATION.md](AUTOMATION.md).

## All commands

| Command | What it does |
|---|---|
| [`run`](#one-mk-run) | Run the sharing service (the default) |
| [`update`](#one-mk-update) | Check for or explicitly install a signed release; never updates in the background |
| [`stop`](#one-mk-stop) | Stop the sharing service for this settings folder before restarting it |
| [`drag`](#one-mk-drag) | Show or switch on file edge drops (experimental; off by default; not on Wayland) |
| [`partials`](#one-mk-partials) | Show, or with --clear delete, the unfinished received files kept so an interrupted send can resume |
| [`send`](#one-mk-send) | Send files or folders to a paired computer running `one-mk run` |
| [`clipboard`](#one-mk-clipboard) | Show or change what the clipboard shares: which formats, and how much (applies at once) |
| [`clip-send`](#one-mk-clip-send) | Send this computer's clipboard to one paired computer now, without moving the pointer |
| [`clip-get`](#one-mk-clip-get) | Copy a paired computer's clipboard to this one now |
| [`notify`](#one-mk-notify) | Show or set whether a notification appears when files arrive |
| [`invite`](#one-mk-invite) | Create a one-time invitation link and QR code |
| [`join`](#one-mk-join) | Join using an invitation link (or read it from stdin) |
| [`pair`](#one-mk-pair) | Pair with another computer (run it on both) |
| [`devices`](#one-mk-devices) | List paired computers |
| [`status`](#one-mk-status) | Show whether one-mk is running, who is connected (with how long each answer takes, "rtt 4 ms") and who controls whom |
| [`takeback`](#one-mk-takeback) | Take this computer back from the computer controlling it (on Linux, where using this computer's own mouse or keyboard can't be noticed; bind it to a keyboard shortcut) |
| [`install`](#one-mk-install) | Start one-mk automatically when you log in |
| [`uninstall`](#one-mk-uninstall) | Stop starting one-mk automatically, turn the wallpaper off (your own comes back) and remove oneMK's GNOME wallpaper extension |
| [`integrate`](#one-mk-integrate) | Add "Send to <computer>" to the file manager's menu (Nautilus, Nemo, Dolphin, Explorer, Finder); it follows the computers you pair or forget |
| [`typing`](#one-mk-typing) | Show or choose what the other computer types for your keys: the same physical keys (default), or the letters printed on them. Experimental: letters (characters, auto) only work toward Windows and Mac computers and have not been tested on real hardware |
| [`layout`](#one-mk-layout) | Say which side of this computer a paired computer is on, and optionally exactly where (applies at once if one-mk is running), and how Ctrl/Cmd are treated |
| [`profile`](#one-mk-profile) | Named layouts: save where every computer is, switch between them, and switch by Wi-Fi network automatically |
| [`forget`](#one-mk-forget) | Unpair a computer |
| [`switch`](#one-mk-switch) | Move the mouse and keyboard to another computer without crossing an edge: a paired computer's name or id, `here`, or `next` (round and round) |
| [`lock-pointer`](#one-mk-lock-pointer) | Keep the pointer on this computer: edges lead nowhere while on |
| [`broadcast`](#one-mk-broadcast) | Type on every connected computer at once: everything you type, click and scroll here goes to all of them until you stop it (also with Ctrl+Alt+B, or Ctrl+Alt+Shift+Esc). Without arguments, says whether it is on |
| [`edge`](#one-mk-edge) | Show or change the rules for crossing edges: dead corners, a dwell time, a key to hold, not leaving a fullscreen window, and what edges with no computer do (pointer wrap, layout learning) |
| [`gaming`](#one-mk-gaming) | Gaming mode: on another computer, send raw mouse movement (for games) instead of positions |
| [`hotkeys`](#one-mk-hotkeys) | Show or change the keyboard shortcuts: jump to computer N, the next one, lock the pointer, come back here, broadcast or fetch a clipboard |
| [`lock-all`](#one-mk-lock-all) | Lock this computer's screen and every connected computer's |
| [`lock-sync`](#one-mk-lock-sync) | Whether locking this computer locks the connected ones too: on or off |
| [`hooks`](#one-mk-hooks) | Show or set commands to run when the pointer arrives on or leaves this computer, or a computer connects or disconnects |
| [`power`](#one-mk-power) | Keep the screensaver and sleep off on computers this one controls, and put displays to sleep together |
| [`keymap`](#one-mk-keymap) | Change what keys do on the computers this one controls: remaps, combos, tap/hold keys, layers, actions (see docs/INPUT.md) |
| [`pointer`](#one-mk-pointer) | Pointer speed on the computers this one controls |
| [`pointer-overlay`](#one-mk-pointer-overlay) | Draw the pointer here while another computer controls this one and this one shows none (no mouse attached): auto (the default), always or never. Without a value: show the setting |
| [`pointer-extension`](#one-mk-pointer-extension) | GNOME only: the optional extension that hides this computer's pointer while the mouse is on another computer (it stays drawn at the edge otherwise): install, enable, status, uninstall |
| [`wallpaper`](#one-mk-wallpaper) | A video or picture as the wallpaper across your screens, or turn it off |
| [`scroll`](#one-mk-scroll) | Scrolling on the computers this one controls: direction, natural, speed, smooth or by notches, modifier keys |
| [`autoscroll`](#one-mk-autoscroll) | Click a button, then move away to scroll the other computer continuously |
| [`gestures`](#one-mk-gestures) | Hold a mouse button and swipe to type a shortcut on the other computer |
| [`buttons`](#one-mk-buttons) | Remap mouse buttons on the computers this one controls |
| [`input-devices`](#one-mk-input-devices) | List the keyboards and mice this computer has seen, for rules that apply to one of them (--device on keymap, buttons, pointer and scroll). Says whether this desktop can tell them apart |
| [`id`](#one-mk-id) | Show this computer's name and id |
| [`doctor`](#one-mk-doctor) | Find out why computers are not connecting, and how to fix it |
| [`bench`](#one-mk-bench) | Measure latency and CPU cost |

## Service options (`one-mk run`)

These come before or after `run`; they are the options of `one-mk` itself.

```text
Options:
      --port <PORT>
          UDP port to listen on (default: 24900)

      --connect <IP:PORT>
          Also try this address, e.g. 192.168.1.20:24900 (for networks that block discovery; repeatable)

      --no-discovery
          Don't advertise or look for computers on the local network

      --backend <BACKEND>
          Input backend (default: native)

          Possible values:
          - native:  This system's real mouse and keyboard, through the best backend it has (on Linux: the portals, else wlroots, else X11)
          - x11:     Linux: X11 (XInput2, XFixes barriers, XTest)
          - wlroots: Linux: the wlroots Wayland protocols (Sway, Hyprland, labwc ...)
          - null:    No real input; for testing the network side

      --no-clipboard
          Don't share the clipboard

      --downloads <DIR>
          Where received files go (default: Downloads/one-mk)

      --start-locked
          Start with the pointer locked to this computer (see `lock-pointer`)

      --no-notify
          Don't show a notification when files arrive (for this run; `one-mk notify off` is permanent)

  -V, --version
          Print version
```

## one-mk run

```text
Run the sharing service (the default)

Usage: one-mk run [OPTIONS]

Options:
      --port <PORT>
          UDP port to listen on (default: 24900)

      --connect <IP:PORT>
          Also try this address, e.g. 192.168.1.20:24900 (for networks that block discovery; repeatable)

      --no-discovery
          Don't advertise or look for computers on the local network

      --backend <BACKEND>
          Input backend (default: native)

          Possible values:
          - native:  This system's real mouse and keyboard, through the best backend it has (on Linux: the portals, else wlroots, else X11)
          - x11:     Linux: X11 (XInput2, XFixes barriers, XTest)
          - wlroots: Linux: the wlroots Wayland protocols (Sway, Hyprland, labwc ...)
          - null:    No real input; for testing the network side

      --no-clipboard
          Don't share the clipboard

      --downloads <DIR>
          Where received files go (default: Downloads/one-mk)

      --start-locked
          Start with the pointer locked to this computer (see `lock-pointer`)

      --no-notify
          Don't show a notification when files arrive (for this run; `one-mk notify off` is permanent)
```

## one-mk update

```text
Check for or explicitly install a signed release; never updates in the background

Usage: one-mk update [OPTIONS]

Options:
      --status-json
          

      --startup-check
          

      --install-automatically <INSTALL_AUTOMATICALLY>
          [possible values: on, off]

      --move-to-applications
          

      --started-version <STARTED_VERSION>
          

      --check
          Check for a signed release without downloading or replacing the executable

      --rollback
          Restore one-mk.old for a portable installation; restart manually afterward

      --check-on-start <CHECK_ON_START>
          Enable or disable the daily startup check (never applies automatically)
          
          [possible values: on, off]

      --url <URL>
          GitHub-compatible releases/latest endpoint (HTTPS; signatures still required)
```

## one-mk stop

```text
Stop the sharing service for this settings folder before restarting it

Usage: one-mk stop [OPTIONS]

Options:
```

## one-mk drag

```text
Show or switch on file edge drops (experimental; off by default; not on Wayland)

Usage: one-mk drag [OPTIONS]

Options:
      --on
          Switch file edge drops on: dropping a file on the screen edge facing another computer sends it there with no confirmation. Experimental: X11, Windows and macOS only, and only X11 has been tried (with scripted drags)

      --off
          Disable file edge-drop zones

      --zone <PX>
          Width of the edge-drop zone, from 1 to 128 pixels
```

## one-mk partials

```text
Show, or with --clear delete, the unfinished received files kept so an interrupted send can resume

Usage: one-mk partials [OPTIONS]

Options:
      --clear
          Delete them all now (any not being received right now). Files older than 7 days are deleted by themselves
```

## one-mk send

```text
Send files or folders to a paired computer running `one-mk run`

Usage: one-mk send [OPTIONS] <PATH>...

Arguments:
  <PATH>...
          Files or folders to send

Options:
      --to <DEVICE>
          Name or id of the paired computer (needed if there are several)

      --connect <IP:PORT>
          Also try this address, e.g. 192.168.1.20:24900 (for networks that block discovery; repeatable)

      --notify-errors
          Also show a desktop notification if sending fails (the file manager's "Send to" entries use this)
```

## one-mk clipboard

```text
Show or change what the clipboard shares: which formats, and how much (applies at once)

Usage: one-mk clipboard [OPTIONS] [COMMAND]

Commands:
  history  Recall local clipboard history. Kept in memory unless history_persist is enabled
  help     Print this message or the help of the given subcommand(s)

Options:
      --text <on|off>
          Share plain text

      --html <on|off>
          Share HTML (rich text)

      --image <on|off>
          Share images

      --max-mb <N>
          Largest clipboard to send or accept, in MB (1 to 256, default 25)

      --skip-sensitive <on|off>
          Skip what a password manager marked as sensitive (default on; `clip-send` also refuses it unless --allow-sensitive; not detectable on every system yet, see the README)

      --allow-fetch <on|off>
          Let other computers fetch this clipboard with `one-mk clip-get` (default off)
```

### one-mk clipboard history

```text
Recall local clipboard history. Kept in memory unless history_persist is enabled

Usage: one-mk clipboard history [OPTIONS] [COMMAND]

Commands:
  on       Retain clipboard copies in memory (explicit privacy opt-in)
  off      Stop retaining clipboard copies and clear retained history
  persist  Keep history across restarts, or remove its saved copy
  list     List newest first, with stable item numbers
  show     Print full text or HTML (terminal controls escaped); --json preserves exact text
  copy     Restore an item here only; does not send it to another computer
  delete   Delete one item by its number in list
  clear    Forget every item, including saved history when enabled
  help     Print this message or the help of the given subcommand(s)

Options:
      --json
          Print machine-readable JSON; text content is included only by show
```

#### one-mk clipboard history on

```text
Retain clipboard copies in memory (explicit privacy opt-in)

Usage: one-mk clipboard history on [OPTIONS]

Options:
      --json
          Print machine-readable JSON; text content is included only by show
```

#### one-mk clipboard history off

```text
Stop retaining clipboard copies and clear retained history

Usage: one-mk clipboard history off [OPTIONS]

Options:
      --json
          Print machine-readable JSON; text content is included only by show
```

#### one-mk clipboard history persist

```text
Keep history across restarts, or remove its saved copy

Usage: one-mk clipboard history persist [OPTIONS] <ENABLED>

Arguments:
  <ENABLED>
          

Options:
      --json
          Print machine-readable JSON; text content is included only by show
```

#### one-mk clipboard history list

```text
List newest first, with stable item numbers

Usage: one-mk clipboard history list [OPTIONS]

Options:
      --json
          Print machine-readable JSON; text content is included only by show
```

#### one-mk clipboard history show

```text
Print full text or HTML (terminal controls escaped); --json preserves exact text

Usage: one-mk clipboard history show [OPTIONS] <NUMBER>

Arguments:
  <NUMBER>
          Item number from one-mk clipboard history

Options:
      --json
          Print machine-readable JSON; text content is included only by show
```

#### one-mk clipboard history copy

```text
Restore an item here only; does not send it to another computer

Usage: one-mk clipboard history copy [OPTIONS] <NUMBER>

Arguments:
  <NUMBER>
          Item number from one-mk clipboard history

Options:
      --json
          Print machine-readable JSON; text content is included only by show
```

#### one-mk clipboard history delete

```text
Delete one item by its number in list

Usage: one-mk clipboard history delete [OPTIONS] <NUMBER>

Arguments:
  <NUMBER>
          Item number from one-mk clipboard history

Options:
      --json
          Print machine-readable JSON; text content is included only by show
```

#### one-mk clipboard history clear

```text
Forget every item, including saved history when enabled

Usage: one-mk clipboard history clear [OPTIONS]

Options:
      --json
          Print machine-readable JSON; text content is included only by show
```

## one-mk clip-send

```text
Send this computer's clipboard to one paired computer now, without moving the pointer

Usage: one-mk clip-send [OPTIONS]

Options:
      --to <DEVICE>
          Name or id of the paired computer (needed if several are connected)

      --allow-sensitive
          Send the clipboard even if a password manager marked it sensitive (refused otherwise). The other computer keeps it out of its history
```

## one-mk clip-get

```text
Copy a paired computer's clipboard to this one now

Usage: one-mk clip-get [OPTIONS]

Options:
      --from <DEVICE>
          Name or id of the paired computer (needed if several are connected)
```

## one-mk notify

```text
Show or set whether a notification appears when files arrive

Usage: one-mk notify [OPTIONS] [STATE]

Arguments:
  [STATE]
          on or off; leave out to see the current setting

Options:
```

## one-mk invite

```text
Create a one-time invitation link and QR code

Usage: one-mk invite [OPTIONS]

Options:
      --expires <EXPIRES>
          Invitation lifetime: e.g. 30s, 10m, 1h (maximum 1 hour)
          
          [default: 10m]

      --confirm
          Require y on this computer before trusting the joiner (the default)

      --no-confirm
          Trust the first valid joiner without a local prompt. Protect the link!

      --listen <LISTEN>
          Pairing listener address; default is all interfaces, a free port

      --no-discovery
          Skip mDNS (the link still contains direct addresses)

      --service-port <SERVICE_PORT>
          Port `run` uses on this computer (default: the running one-mk's, or 24900)
```

## one-mk join

```text
Join using an invitation link (or read it from stdin)

Usage: one-mk join [OPTIONS] [LINK]

Arguments:
  [LINK]
          Invitation link. Omit to read one line from stdin (avoids shell history)

Options:
      --yes
          Trust the pinned host without prompting (required for non-terminal stdin)

      --service-port <SERVICE_PORT>
          Port `run` uses on this computer (default: the running one-mk's, or 24900)
```

## one-mk pair

```text
Pair with another computer (run it on both)

Usage: one-mk pair [OPTIONS]

Options:
      --connect <IP:PORT>
          Pair with the computer at this address instead of searching the network, e.g. 192.168.1.20:40123 (the other side prints its pairing port)

      --no-discovery
          Pair only through direct addresses; do not use multicast discovery
```

## one-mk devices

```text
List paired computers

Usage: one-mk devices [OPTIONS]

Options:
```

## one-mk status

```text
Show whether one-mk is running, who is connected (with how long each answer takes, "rtt 4 ms") and who controls whom

Usage: one-mk status [OPTIONS]

Options:
```

## one-mk takeback

```text
Take this computer back from the computer controlling it (on Linux, where using this computer's own mouse or keyboard can't be noticed; bind it to a keyboard shortcut)

Usage: one-mk takeback [OPTIONS]

Options:
```

## one-mk install

```text
Start one-mk automatically when you log in

Usage: one-mk install [OPTIONS]

Options:
```

## one-mk uninstall

```text
Stop starting one-mk automatically, turn the wallpaper off (your own comes back) and remove oneMK's GNOME wallpaper extension

Usage: one-mk uninstall [OPTIONS]

Options:
```

## one-mk integrate

```text
Add "Send to <computer>" to the file manager's menu (Nautilus, Nemo, Dolphin, Explorer, Finder); it follows the computers you pair or forget

Usage: one-mk integrate [OPTIONS]

Options:
      --remove
          Remove the entries instead
```

## one-mk typing

```text
Show or choose what the other computer types for your keys: the same physical keys (default), or the letters printed on them. Experimental: letters (characters, auto) only work toward Windows and Mac computers and have not been tested on real hardware

Usage: one-mk typing [OPTIONS]

Options:
      --keys <KEYS>
          physical: the same keys, so letters follow the other computer's layout (default). characters (experimental): the letters printed on your keys. auto (experimental): characters only when the layouts differ. Letters work only toward Windows and Mac computers

          Possible values:
          - physical:   The same physical keys; letters follow the other computer's layout (default)
          - characters: The letters printed on your keys (experimental)
          - auto:       Characters only when the two layouts differ (experimental)

      --peer <NAME_OR_ID>
          Change or show only this paired computer's preference
```

## one-mk layout

```text
Say which side of this computer a paired computer is on, and optionally exactly where (applies at once if one-mk is running), and how Ctrl/Cmd are treated

Usage: one-mk layout [OPTIONS] <DEVICE> [SIDE]

Arguments:
  <DEVICE>
          Name or id of the paired computer (see `one-mk devices`)

  [SIDE]
          Where it is: left, right, top or bottom

Options:
      --offset <DX,DY|none>
          Where its (0,0) sits in this computer's screen coordinates, so the pointer crosses only where the screens touch; `none` crosses anywhere along the side

      --swap-ctrl-cmd <on|off|auto>
          Swap Ctrl and Cmd for keys sent to it, so copy and paste feel the same between a Mac and a PC (default: auto, which swaps only between a Mac and another computer)
```

## one-mk profile

```text
Named layouts: save where every computer is, switch between them, and switch by Wi-Fi network automatically

Usage: one-mk profile [OPTIONS] <COMMAND>

Commands:
  save    Save where every paired computer is now as a profile (replacing one of the same name, keeping its automatic switching)
  use     Put a saved profile's layout in place (applies at once if one-mk is running; the other computers pick it up too)
  list    List the saved profiles
  delete  Delete a profile
  auto    Switch to a profile by itself when this computer joins a Wi-Fi network or a network (needs NetworkManager on Linux)
  help    Print this message or the help of the given subcommand(s)

Options:
```

### one-mk profile save

```text
Save where every paired computer is now as a profile (replacing one of the same name, keeping its automatic switching)

Usage: one-mk profile save [OPTIONS] <NAME>

Arguments:
  <NAME>
          Name for the saved layout

Options:
```

### one-mk profile use

```text
Put a saved profile's layout in place (applies at once if one-mk is running; the other computers pick it up too)

Usage: one-mk profile use [OPTIONS] <NAME>

Arguments:
  <NAME>
          Saved layout to activate

Options:
```

### one-mk profile list

```text
List the saved profiles

Usage: one-mk profile list [OPTIONS]

Options:
```

### one-mk profile delete

```text
Delete a profile

Usage: one-mk profile delete [OPTIONS] <NAME>

Arguments:
  <NAME>
          Saved layout to delete

Options:
```

### one-mk profile auto

```text
Switch to a profile by itself when this computer joins a Wi-Fi network or a network (needs NetworkManager on Linux)

Usage: one-mk profile auto [OPTIONS] <NAME>

Arguments:
  <NAME>
          Saved layout whose automatic switching rules change

Options:
      --ssid <WIFI-NAME>
          A Wi-Fi network name (repeatable)

      --network <GATEWAY-MAC|SUBNET>
          A router's MAC address (aa:bb:cc:dd:ee:ff) or a subnet (192.168.1.0/24) (repeatable)

      --clear
          Clear earlier automatic rules before adding these rules

      --off
          Stop switching to it by itself
```

## one-mk forget

```text
Unpair a computer

Usage: one-mk forget [OPTIONS] <DEVICE>

Arguments:
  <DEVICE>
          Name or id of the paired computer (see `one-mk devices`)

Options:
```

## one-mk switch

```text
Move the mouse and keyboard to another computer without crossing an edge: a paired computer's name or id, `here`, or `next` (round and round)

Usage: one-mk switch [OPTIONS] <COMPUTER|here|next>

Arguments:
  <COMPUTER|here|next>
          Name or id, `here` or `next`

Options:
```

## one-mk lock-pointer

```text
Keep the pointer on this computer: edges lead nowhere while on

Usage: one-mk lock-pointer [OPTIONS] <on|off|toggle>

Arguments:
  <on|off|toggle>
          Enable, disable or toggle pointer locking
          
          [possible values: on, off, toggle]

Options:
```

## one-mk broadcast

```text
Type on every connected computer at once: everything you type, click and scroll here goes to all of them until you stop it (also with Ctrl+Alt+B, or Ctrl+Alt+Shift+Esc). Without arguments, says whether it is on

Usage: one-mk broadcast [OPTIONS] [on|off|toggle]

Arguments:
  [on|off|toggle]
          Enable, disable or toggle broadcasting; omit to show its state
          
          [possible values: on, off, toggle]

Options:
      --idle-off <MINUTES>
          Stop by itself after this many minutes without any key, click, scroll or movement (0 = never; default 15)

Examples:
  one-mk broadcast on
  one-mk broadcast off
  one-mk broadcast --idle-off 30    stop by itself after 30 minutes without input (0 = never)

See docs/BROADCAST.md for what is sent where and how it stops.
```

## one-mk edge

```text
Show or change the rules for crossing edges: dead corners, a dwell time, a key to hold, not leaving a fullscreen window, and what edges with no computer do (pointer wrap, layout learning)

Usage: one-mk edge [OPTIONS]

Options:
      --side <SIDE>
          Change only this edge (left, right, top or bottom); without it, every edge

      --corner <PX>
          Ignore crossings within this many pixels of a screen corner (0: off)

      --dwell <MS>
          The pointer must stay at the edge this long before it crosses (0: off)

      --modifier <KEY>
          Only cross while holding this key: none, shift, ctrl or alt

      --fullscreen-guard <on|off>
          Don't cross while a fullscreen window has the focus here: on or off

      --fullscreen-allow <APP>
          A program that may always be left, even fullscreen (repeatable)

      --fullscreen-block <APP>
          A program that is never left while it has the focus (repeatable)

      --fullscreen-forget <APP>
          Remove a program from both lists (repeatable)

      --reset
          With --side: drop that edge's own rules, so it follows the rules for every edge again. Without: turn every rule off, and --wrap, --learn and --idle-return go back to off, suggest and default

      --wrap <off|on|all>
          Where no computer sits, bring the pointer in at the opposite edge: off, on (left and right edges) or all (every edge). For every edge

          Possible values:
          - off: An edge where no computer sits stops the pointer (the default)
          - on:  The left and right edges where no computer sits lead round to the opposite edge
          - all: Every edge where no computer sits does, the top and bottom too

      --learn <off|suggest|auto>
          Pushing the pointer twice toward a computer that isn't placed yet: off, suggest (say where it seems to be) or auto (place it). For every edge

          Possible values:
          - off:     Pushing toward a computer does nothing
          - suggest: Say where a computer seems to be, and how to place it there (the default)
          - auto:    Place it there (beside you on the left or right; above or below is only suggested), and say how to undo that

      --dismiss
          Hide the layout suggestion shown now (in `one-mk status` and the window)

      --idle-return <TIME|off|default>
          While another computer controls this one but sends no input, let it go after this long: seconds, or with s, m or h (10s to 24h); off to never; default for 10 minutes. Its pointer then goes home and its next movement enters this computer again
```

## one-mk gaming

```text
Gaming mode: on another computer, send raw mouse movement (for games) instead of positions

Usage: one-mk gaming [OPTIONS] [MODE]

Arguments:
  [MODE]
          Enable, disable or automatically choose raw mouse movement

          Possible values:
          - off:  Absolute positions, like a normal mouse (the default)
          - on:   Relative movement whenever the pointer is on another computer
          - auto: Relative movement while the other computer has a fullscreen window in front (where it can tell: Windows, and X11 windows on Linux)

Options:
```

## one-mk hotkeys

```text
Show or change the keyboard shortcuts: jump to computer N, the next one, lock the pointer, come back here, broadcast or fetch a clipboard

Usage: one-mk hotkeys [OPTIONS]

Options:
      --jump <CHORD>
          Jump to computer N: use ctrl+alt+ followed by n in braces (none: off)

      --cycle <CHORD>
          The next computer, like 'ctrl+alt+pagedown' (none: off)

      --lock-pointer <CHORD>
          Lock the pointer to this computer or unlock it, like 'ctrl+alt+p' (none: off)

      --home-key <CHORD>
          Back to this computer, like 'ctrl+alt+h' (none: off)

      --broadcast <CHORD>
          Start or stop broadcast to every connected computer, like 'ctrl+alt+b' (none: off)

      --fetch-clipboard <CHORD>
          Get the next online computer's clipboard here; press Ctrl+V yourself (none: off)

      --off
          Turn every shortcut off

      --on
          Turn the shortcuts on again
```

## one-mk lock-all

```text
Lock this computer's screen and every connected computer's

Usage: one-mk lock-all [OPTIONS]

Options:
```

## one-mk lock-sync

```text
Whether locking this computer locks the connected ones too: on or off

Usage: one-mk lock-sync [OPTIONS] [on|off]

Arguments:
  [on|off]
          Enable or disable shared screen locking

Options:
```

## one-mk hooks

```text
Show or set commands to run when the pointer arrives on or leaves this computer, or a computer connects or disconnects

Usage: one-mk hooks [OPTIONS]

Options:
      --computer <COMPUTER>
          With a command given here: run it only for this paired computer (name or id; "" for every computer)

      --event-timeout <SECS>
          With a command given here: seconds it may run before it is killed (1-60), instead of --timeout

      --event-shell <on|off>
          With a command given here: run it through the system shell (on) or not (off), instead of --shell

      --on-enter <CMD>
          Run when the pointer arrives on this computer

      --on-leave <CMD>
          Run when the pointer leaves this computer

      --on-peer-connect <CMD>
          Run when a computer connects

      --on-peer-disconnect <CMD>
          Run when a computer disconnects

      --timeout <SECS>
          Seconds a command may run before it is killed (1-60, default 5)

      --shell <on|off>
          Run commands through the system shell (`on`), which expands and interprets them, or without (`off`, the default)

      --trust-permissions
          Explicitly allow hooks despite unsafe Unix ownership or permissions

      --off
          Remove every hook
```

## one-mk power

```text
Keep the screensaver and sleep off on computers this one controls, and put displays to sleep together

Usage: one-mk power [OPTIONS]

Options:
      --block-screensaver <on|off>
          Keep the screensaver and sleep off on computers this one controls

      --sleep-sync <on|off>
          Put the displays of connected computers to sleep when this one's sleeps (and follow theirs)
```

## one-mk keymap

```text
Change what keys do on the computers this one controls: remaps, combos, tap/hold keys, layers, actions (see docs/INPUT.md)

Usage: one-mk keymap [OPTIONS] [COMMAND]

Commands:
  list    Show the key rules
  add     Make FROM (a key, or a chord like ctrl+h) type TO: a key, a chord (ctrl+shift+esc), several (a sequence), `none`, `layer:NAME`, `back`/`forward`, or an action (`switch next`, `lock-pointer`, `lock-all`, `scroll down`, `run: COMMAND`). Replaces a rule for the same key in the same scope and layer
  remove  Remove the rule for FROM (in that scope and layer)
  clear   Remove every key rule
  check   Check the rules (also the pointer, scroll and button settings)
  help    Print this message or the help of the given subcommand(s)

Options:
```

### one-mk keymap list

```text
Show the key rules

Usage: one-mk keymap list [OPTIONS]

Options:
```

### one-mk keymap add

```text
Make FROM (a key, or a chord like ctrl+h) type TO: a key, a chord (ctrl+shift+esc), several (a sequence), `none`, `layer:NAME`, `back`/`forward`, or an action (`switch next`, `lock-pointer`, `lock-all`, `scroll down`, `run: COMMAND`). Replaces a rule for the same key in the same scope and layer

Usage: one-mk keymap add [OPTIONS] <FROM> [TO]...

Arguments:
  <FROM>
          Key or chord to change, for example capslock or ctrl+h

  [TO]...
          What it becomes; several make a sequence. Leave out with --tap and --hold

Options:
      --tap <KEY>
          Tapped, it types this (with --hold)

      --hold <KEY>
          Held (or pressed with another key), this (with --tap)

      --layer <NAME>
          The rule only applies while this layer is on

      --timeout-ms <MS>
          For --tap/--hold: how long before it counts as held (default 200 ms)

      --peer <COMPUTER>
          Only for this paired computer (name or id)

      --os <OS>
          Only for computers running this OS: mac, windows or linux

      --app <NAME>
          Only while this app has the focus on the other computer: a program file on Windows (chrome.exe), a bundle id on a Mac (com.apple.Terminal), a window class on Linux (firefox). `*` matches anything, `!` in front means every app but this one

      --device <NAME|FINGERPRINT>
          Only for input from this keyboard or mouse, on this computer: its name (or a part of it) or fingerprint, as `one-mk input-devices` lists it. Combines with --peer or --os
```

### one-mk keymap remove

```text
Remove the rule for FROM (in that scope and layer)

Usage: one-mk keymap remove [OPTIONS] <FROM>

Arguments:
  <FROM>
          Key or chord whose rule should be removed

Options:
      --layer <NAME>
          

      --peer <COMPUTER>
          Only for this paired computer (name or id)

      --os <OS>
          Only for computers running this OS: mac, windows or linux

      --app <NAME>
          Only while this app has the focus on the other computer: a program file on Windows (chrome.exe), a bundle id on a Mac (com.apple.Terminal), a window class on Linux (firefox). `*` matches anything, `!` in front means every app but this one

      --device <NAME|FINGERPRINT>
          Only for input from this keyboard or mouse, on this computer: its name (or a part of it) or fingerprint, as `one-mk input-devices` lists it. Combines with --peer or --os
```

### one-mk keymap clear

```text
Remove every key rule

Usage: one-mk keymap clear [OPTIONS]

Options:
```

### one-mk keymap check

```text
Check the rules (also the pointer, scroll and button settings)

Usage: one-mk keymap check [OPTIONS]

Options:
```

## one-mk pointer

```text
Pointer speed on the computers this one controls

Usage: one-mk pointer [OPTIONS]

Options:
      --speed <X>
          How far the pointer moves on other computers, times this (0.1 to 10)

      --reset
          Forget the pointer settings for this scope

      --peer <COMPUTER>
          Only for this paired computer (name or id)

      --os <OS>
          Only for computers running this OS: mac, windows or linux

      --app <NAME>
          Only while this app has the focus on the other computer: a program file on Windows (chrome.exe), a bundle id on a Mac (com.apple.Terminal), a window class on Linux (firefox). `*` matches anything, `!` in front means every app but this one

      --device <NAME|FINGERPRINT>
          Only for input from this keyboard or mouse, on this computer: its name (or a part of it) or fingerprint, as `one-mk input-devices` lists it. Combines with --peer or --os
```

## one-mk pointer-overlay

```text
Draw the pointer here while another computer controls this one and this one shows none (no mouse attached): auto (the default), always or never. Without a value: show the setting

Usage: one-mk pointer-overlay [OPTIONS] [auto|always|never]

Arguments:
  [auto|always|never]
          [possible values: auto, always, never]

Options:
```

## one-mk pointer-extension

```text
GNOME only: the optional extension that hides this computer's pointer while the mouse is on another computer (it stays drawn at the edge otherwise): install, enable, status, uninstall

Usage: one-mk pointer-extension [OPTIONS] <COMMAND>

Commands:
  install    Copy the extension into your GNOME extensions folder (it is not enabled)
  enable     Ask GNOME to enable the installed extension
  status     Show whether it is installed and active
  uninstall  Remove the installed files (disable it first: `gnome-extensions disable one-mk-pointer@one-mk.org`)
  help       Print this message or the help of the given subcommand(s)

Options:
```

### one-mk pointer-extension install

```text
Copy the extension into your GNOME extensions folder (it is not enabled)

Usage: one-mk pointer-extension install [OPTIONS]

Options:
```

### one-mk pointer-extension enable

```text
Ask GNOME to enable the installed extension

Usage: one-mk pointer-extension enable [OPTIONS]

Options:
```

### one-mk pointer-extension status

```text
Show whether it is installed and active

Usage: one-mk pointer-extension status [OPTIONS]

Options:
```

### one-mk pointer-extension uninstall

```text
Remove the installed files (disable it first: `gnome-extensions disable one-mk-pointer@one-mk.org`)

Usage: one-mk pointer-extension uninstall [OPTIONS]

Options:
```

## one-mk wallpaper

```text
A video or picture as the wallpaper across your screens, or turn it off.

Stable: video on X11 desktops, pictures on GNOME. Experimental (not yet tested on real hardware there): macOS, Windows, layer-shell Wayland desktops (KDE, Sway, Hyprland), video on GNOME Wayland through oneMK's GNOME extension, one wallpaper shared across several computers, and the smaller copy made of large videos.

Usage: one-mk wallpaper [OPTIONS] <COMMAND>

Commands:
  set        Show a video or picture as the wallpaper across your screens and computers (sharing across computers is experimental)
  off        Turn the wallpaper off everywhere and put your own wallpaper back
  status     Say what is shown, where, and anything experimental here
  pause      Pause the video on its current frame
  play       Play a paused video again
  extension  Set up oneMK's GNOME extension, which video needs on GNOME Wayland (experimental; GNOME 50)
  help       Print this message or the help of the given subcommand(s)

Options:
      --json
          Print the result as JSON (only `extension` commands)
```

### one-mk wallpaper set

```text
Show a video or picture as the wallpaper across your screens and computers (sharing across computers is experimental)

Usage: one-mk wallpaper set [OPTIONS] <FILE>

Arguments:
  <FILE>
          A video (MP4, MOV, M4V, WebM, MKV) or picture (PNG, JPEG, WebP, HEIC)

Options:
      --json
          Print the result as JSON (only `extension` commands)
```

### one-mk wallpaper off

```text
Turn the wallpaper off everywhere and put your own wallpaper back

Usage: one-mk wallpaper off [OPTIONS]

Options:
      --json
          Print the result as JSON (only `extension` commands)
```

### one-mk wallpaper status

```text
Say what is shown, where, and anything experimental here

Usage: one-mk wallpaper status [OPTIONS]

Options:
      --json
          Print the result as JSON (only `extension` commands)
```

### one-mk wallpaper pause

```text
Pause the video on its current frame

Usage: one-mk wallpaper pause [OPTIONS]

Options:
      --json
          Print the result as JSON (only `extension` commands)
```

### one-mk wallpaper play

```text
Play a paused video again

Usage: one-mk wallpaper play [OPTIONS]

Options:
      --json
          Print the result as JSON (only `extension` commands)
```

### one-mk wallpaper extension

```text
Set up oneMK's GNOME extension, which video needs on GNOME Wayland (experimental; GNOME 50)

Usage: one-mk wallpaper extension [OPTIONS] <COMMAND>

Commands:
  install    Copy oneMK's GNOME extension into your extensions folder (nothing is downloaded) and switch it on for your next login; then log out and back in once. Also updates an older copy oneMK installed
  enable     Switch the installed extension on (at your next login if GNOME has not loaded it yet). It never starts a video by itself
  uninstall  Switch the extension off and remove oneMK's copy of it (a copy you changed is left alone)
  status     Show whether the extension is installed, switched on and supported by this GNOME version
  help       Print this message or the help of the given subcommand(s)

Options:
      --json
          Print the result as JSON (only `extension` commands)
```

#### one-mk wallpaper extension install

```text
Copy oneMK's GNOME extension into your extensions folder (nothing is downloaded) and switch it on for your next login; then log out and back in once. Also updates an older copy oneMK installed

Usage: one-mk wallpaper extension install [OPTIONS]

Options:
      --json
          Print the result as JSON (only `extension` commands)

      --renderer <PATH>
          Use this wallpaper helper (an absolute path) instead of the one installed with oneMK
```

#### one-mk wallpaper extension enable

```text
Switch the installed extension on (at your next login if GNOME has not loaded it yet). It never starts a video by itself

Usage: one-mk wallpaper extension enable [OPTIONS]

Options:
      --json
          Print the result as JSON (only `extension` commands)
```

#### one-mk wallpaper extension uninstall

```text
Switch the extension off and remove oneMK's copy of it (a copy you changed is left alone)

Usage: one-mk wallpaper extension uninstall [OPTIONS]

Options:
      --json
          Print the result as JSON (only `extension` commands)
```

#### one-mk wallpaper extension status

```text
Show whether the extension is installed, switched on and supported by this GNOME version

Usage: one-mk wallpaper extension status [OPTIONS]

Options:
      --json
          Print the result as JSON (only `extension` commands)
```

## one-mk scroll

```text
Scrolling on the computers this one controls: direction, natural, speed, smooth or by notches, modifier keys

Usage: one-mk scroll [OPTIONS]

Options:
      --reverse-vertical <on|off>
          Flip vertical scrolling

      --reverse-horizontal <on|off>
          Flip horizontal scrolling

      --speed <X>
          Scroll this much faster or slower (0.1 to 10)

      --mode <MODE>
          auto (as it comes), smooth (in pixels) or discrete (whole notches)
          
          [possible values: auto, smooth, discrete]

      --natural <NATURAL>
          Natural scrolling on the other computers: on, off, or auto (the other computer's own default: natural on a Mac, traditional elsewhere). Assumes this computer uses its OS's default direction
          
          [possible values: on, off, auto]

      --modifier <MOD=WHAT>
          What holding a modifier does: shift=horizontal, alt=x5, ctrl=vertical, meta=unchanged (repeatable; unchanged removes it)

      --kind <KIND>
          Only for wheels or only for touchpads (`--device wheel|touchpad` still works too)

          Possible values:
          - wheel:    Notched mouse wheels
          - touchpad: Touchpads and other smooth scrolling

      --reset
          Forget the scroll settings for this scope (and device)

      --peer <COMPUTER>
          Only for this paired computer (name or id)

      --os <OS>
          Only for computers running this OS: mac, windows or linux

      --app <NAME>
          Only while this app has the focus on the other computer: a program file on Windows (chrome.exe), a bundle id on a Mac (com.apple.Terminal), a window class on Linux (firefox). `*` matches anything, `!` in front means every app but this one

      --device <NAME|FINGERPRINT>
          Only for input from this keyboard or mouse, on this computer: its name (or a part of it) or fingerprint, as `one-mk input-devices` lists it. Combines with --peer or --os
```

## one-mk autoscroll

```text
Click a button, then move away to scroll the other computer continuously

Usage: one-mk autoscroll [OPTIONS]

Options:
      --on
          Enable click-to-scroll on the other computer

      --off
          Disable click-to-scroll

      --button <BUTTON>
          Toggle button, for example middle or button4 (default middle)

      --speed <SPEED>
          Scroll rate multiplier, from 0.1 to 10 (default 1)

      --dead-zone <DEAD_ZONE>
          Movement before scrolling starts, 0 to 1000 backend motion units (default 8)

      --vertical-only <on|off>
          Scroll only up and down, or in both directions

      --reset
          Forget the autoscroll settings for this scope (and app), so the ones for everything else apply again

      --peer <COMPUTER>
          Only for this paired computer (name or id)

      --os <OS>
          Only for computers running this OS: mac, windows or linux

      --app <NAME>
          Only while this app has the focus on the other computer: a program file on Windows (chrome.exe), a bundle id on a Mac (com.apple.Terminal), a window class on Linux (firefox). `*` matches anything, `!` in front means every app but this one
```

## one-mk gestures

```text
Hold a mouse button and swipe to type a shortcut on the other computer

Usage: one-mk gestures [OPTIONS] [COMMAND]

Commands:
  list    Show held-button gestures
  add     Hold BUTTON and swipe DIRECTION to type TO on the other computer
  remove  Remove a button's gesture in this scope
  clear   Remove all held-button gestures in this scope
  preset  Hold the back side button and swipe left or right between desktops
  help    Print this message or the help of the given subcommand(s)

Options:
```

### one-mk gestures list

```text
Show held-button gestures

Usage: one-mk gestures list [OPTIONS]

Options:
```

### one-mk gestures add

```text
Hold BUTTON and swipe DIRECTION to type TO on the other computer

Usage: one-mk gestures add [OPTIONS] <BUTTON> <DIRECTION> <TO>...

Arguments:
  <BUTTON>
          Button to hold, for example button4 (not left or right)

  <DIRECTION>
          Direction to swipe while holding the button
          
          [possible values: up, down, left, right]

  <TO>...
          A key, chord, quoted action, or several chords for a sequence; desktop-left and desktop-right switch virtual desktops on any OS

Options:
      --threshold <THRESHOLD>
          Minimum swipe distance, 1 to 10000 backend motion units
          
          [default: 60]

      --peer <COMPUTER>
          Only for this paired computer (name or id)

      --os <OS>
          Only for computers running this OS: mac, windows or linux

      --app <NAME>
          Only while this app has the focus on the other computer: a program file on Windows (chrome.exe), a bundle id on a Mac (com.apple.Terminal), a window class on Linux (firefox). `*` matches anything, `!` in front means every app but this one
```

### one-mk gestures remove

```text
Remove a button's gesture in this scope

Usage: one-mk gestures remove [OPTIONS] <BUTTON> <DIRECTION>

Arguments:
  <BUTTON>
          Button whose gesture should be removed

  <DIRECTION>
          Direction whose gesture should be removed
          
          [possible values: up, down, left, right]

Options:
      --peer <COMPUTER>
          Only for this paired computer (name or id)

      --os <OS>
          Only for computers running this OS: mac, windows or linux

      --app <NAME>
          Only while this app has the focus on the other computer: a program file on Windows (chrome.exe), a bundle id on a Mac (com.apple.Terminal), a window class on Linux (firefox). `*` matches anything, `!` in front means every app but this one
```

### one-mk gestures clear

```text
Remove all held-button gestures in this scope

Usage: one-mk gestures clear [OPTIONS]

Options:
      --peer <COMPUTER>
          Only for this paired computer (name or id)

      --os <OS>
          Only for computers running this OS: mac, windows or linux

      --app <NAME>
          Only while this app has the focus on the other computer: a program file on Windows (chrome.exe), a bundle id on a Mac (com.apple.Terminal), a window class on Linux (firefox). `*` matches anything, `!` in front means every app but this one
```

### one-mk gestures preset

```text
Hold the back side button and swipe left or right between desktops

Usage: one-mk gestures preset [OPTIONS] <NAME>

Arguments:
  <NAME>
          Gesture preset to install
          
          [possible values: side-swipe]

Options:
      --peer <COMPUTER>
          Only for this paired computer (name or id)

      --os <OS>
          Only for computers running this OS: mac, windows or linux

      --app <NAME>
          Only while this app has the focus on the other computer: a program file on Windows (chrome.exe), a bundle id on a Mac (com.apple.Terminal), a window class on Linux (firefox). `*` matches anything, `!` in front means every app but this one
```

## one-mk buttons

```text
Remap mouse buttons on the computers this one controls

Usage: one-mk buttons [OPTIONS] [COMMAND]

Commands:
  list    Show the button rules
  map     Make button N (1 left, 2 right, 3 middle, 4 back, 5 forward, 6 to 8) do TO: another button (button1), a key or chord, several (a sequence), back/forward, an action, or none
  unmap   Remove the rule for button N
  preset  side-back-forward: buttons 4 and 5 go back and forward (Alt+Left/Right; Cmd+[ and Cmd+] on a Mac)
  clear   Remove every button rule
  help    Print this message or the help of the given subcommand(s)

Options:
```

### one-mk buttons list

```text
Show the button rules

Usage: one-mk buttons list [OPTIONS]

Options:
```

### one-mk buttons map

```text
Make button N (1 left, 2 right, 3 middle, 4 back, 5 forward, 6 to 8) do TO: another button (button1), a key or chord, several (a sequence), back/forward, an action, or none

Usage: one-mk buttons map [OPTIONS] <BUTTON> <TO>...

Arguments:
  <BUTTON>
          Mouse button to remap, for example button4

  <TO>...
          Output button, key, chord, quoted action or sequence

Options:
      --peer <COMPUTER>
          Only for this paired computer (name or id)

      --os <OS>
          Only for computers running this OS: mac, windows or linux

      --app <NAME>
          Only while this app has the focus on the other computer: a program file on Windows (chrome.exe), a bundle id on a Mac (com.apple.Terminal), a window class on Linux (firefox). `*` matches anything, `!` in front means every app but this one

      --device <NAME|FINGERPRINT>
          Only for input from this keyboard or mouse, on this computer: its name (or a part of it) or fingerprint, as `one-mk input-devices` lists it. Combines with --peer or --os
```

### one-mk buttons unmap

```text
Remove the rule for button N

Usage: one-mk buttons unmap [OPTIONS] <BUTTON>

Arguments:
  <BUTTON>
          Mouse button whose rule should be removed

Options:
      --peer <COMPUTER>
          Only for this paired computer (name or id)

      --os <OS>
          Only for computers running this OS: mac, windows or linux

      --app <NAME>
          Only while this app has the focus on the other computer: a program file on Windows (chrome.exe), a bundle id on a Mac (com.apple.Terminal), a window class on Linux (firefox). `*` matches anything, `!` in front means every app but this one

      --device <NAME|FINGERPRINT>
          Only for input from this keyboard or mouse, on this computer: its name (or a part of it) or fingerprint, as `one-mk input-devices` lists it. Combines with --peer or --os
```

### one-mk buttons preset

```text
side-back-forward: buttons 4 and 5 go back and forward (Alt+Left/Right; Cmd+[ and Cmd+] on a Mac)

Usage: one-mk buttons preset [OPTIONS] <NAME>

Arguments:
  <NAME>
          [possible values: side-back-forward]

Options:
      --peer <COMPUTER>
          Only for this paired computer (name or id)

      --os <OS>
          Only for computers running this OS: mac, windows or linux

      --app <NAME>
          Only while this app has the focus on the other computer: a program file on Windows (chrome.exe), a bundle id on a Mac (com.apple.Terminal), a window class on Linux (firefox). `*` matches anything, `!` in front means every app but this one

      --device <NAME|FINGERPRINT>
          Only for input from this keyboard or mouse, on this computer: its name (or a part of it) or fingerprint, as `one-mk input-devices` lists it. Combines with --peer or --os
```

### one-mk buttons clear

```text
Remove every button rule

Usage: one-mk buttons clear [OPTIONS]

Options:
```

## one-mk input-devices

```text
List the keyboards and mice this computer has seen, for rules that apply to one of them (--device on keymap, buttons, pointer and scroll). Says whether this desktop can tell them apart

Usage: one-mk input-devices [OPTIONS]

Options:
      --json
          Machine-readable output for a program to read
```

## one-mk id

```text
Show this computer's name and id

Usage: one-mk id [OPTIONS]

Options:
```

## one-mk doctor

```text
Find out why computers are not connecting, and how to fix it

Usage: one-mk doctor [OPTIONS]

Options:
      --json
          Machine-readable output for a program to read

      --all
          Also list the checks that did not apply here
```

## one-mk bench

```text
Measure latency and CPU cost

Usage: one-mk bench [OPTIONS]

Options:
      --count <COUNT>
          Number of mouse events to send
          
          [default: 5000]

      --rate <RATE>
          Events per second (gaming mice report 1000-8000)
          
          [default: 1000]

      --serve [<PORT>]
          Echo for a bench on another machine

      --connect <IP:PORT>
          Measure against `one-mk bench --serve` on another machine, e.g. 192.168.1.20:24901

      --peer <DEVICE>
          Measure against the running `one-mk run` on a paired computer (name or id)
```

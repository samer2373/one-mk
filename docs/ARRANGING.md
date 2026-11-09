# Arranging computers, switching and edge rules

Everything here is also in the window: **Desk > Arrange** for placement and
profiles, **Settings** for edge rules, gaming mode and shortcuts. The window runs
the same `one-mk` commands, so the two always agree. Every command takes effect
at once, with no restart.

## Where each computer sits

Pairing gives a new computer the first free side (left, right, above, below).
Move it to match your desk, on either computer:

```bash
one-mk layout "other-pc" left      # the other computer records the mirror image
```

The other computer is told, so it records the mirror image (you put it on your
left, so you are on its right). If the two ever disagree, for example because one
was switched off when you changed it, the newer change wins as soon as they
connect. If you change both in the very same millisecond, the one with the larger
certificate fingerprint wins, so they always end up as mirror images.

By default the pointer crosses anywhere along that side: wherever you leave your
screen, you arrive at the same fraction of the way along its edge, whatever the
two sizes.

To place it exactly, drag it in **Desk > Arrange** next to one of your monitors,
or give `--offset`: where its (0,0), its top-left corner, sits in your screen
coordinates (logical pixels, what your display settings call the resolution after
scaling). The pointer then crosses only where its screens touch yours, pixel for
pixel, like two monitors of one computer; beside that the edge is a wall:

```bash
one-mk layout laptop bottom --offset 300,1080   # below a 1080-high screen, its left edge 300 px right of yours
one-mk layout laptop right --offset 1920,-200   # right of a 1920-wide screen, its top 200 px above yours
one-mk layout laptop --offset none              # anywhere along its side again (the side stays)
```

Moving a computer to another side without `--offset` drops its offset (it means
something else on another side). Offsets must be within -16777216 and 16777216 on
each axis. `one-mk devices` shows each offset. The other computer gets the mirror
image (`1920,-200` here is `-1920,200` there), and as with sides the newer change
wins. A computer whose screens touch yours nowhere is warned about in
`one-mk layout`, `one-mk status`, `one-mk doctor` and the window; `one-mk switch`
still goes there, and all of its facing edge leads back.

Several computers can share one side if their screens touch different parts of
it, for example two laptops below a wide monitor:

```bash
one-mk layout laptop-a bottom --offset 0,1440
one-mk layout laptop-b bottom --offset 1920,1440
```

**Several monitors.** The pointer crosses only through a monitor edge with no
monitor of yours beyond it: without an offset, the outer edge of the whole desktop
where a monitor reaches it; with an offset, wherever the other computer's screens
touch such an edge, even one set back from the outer boundary. The pointer arrives
where it left, measured along the monitors of that edge, and comes back to the
same point.

What happens right after pairing, including when all four sides are taken, is in
[PAIRING-LAYOUT.md](PAIRING-LAYOUT.md).

## Profiles

Save the arrangement under a name and switch between them:

```bash
one-mk profile save home        # where every paired computer is now
one-mk layout laptop left       # rearrange at the office...
one-mk profile save office
one-mk profile use home         # back home: everything moves at once
one-mk profile list
one-mk profile delete office
```

A profile holds each computer's side and offset, nothing else: edge rules, gaming
mode and the other settings stay the same everywhere. Using a profile counts as a
new change, so the other computers pick it up. Names use 1-64 letters, digits,
spaces or `._-`.

To switch by itself when this computer joins a network:

```bash
one-mk profile auto office --ssid "Office WiFi"
one-mk profile auto home --network 192.168.1.0/24          # a subnet this computer is on
one-mk profile auto home --network aa:bb:cc:dd:ee:ff       # the router's MAC address
one-mk profile auto home --off
```

Repeated `profile auto` calls accumulate rules; `--clear` removes earlier rules
before adding replacements, `--off` removes every automatic rule for that profile.
The first profile that matches is used, once each time the network changes, so a
`one-mk layout` change afterwards stays until you move on. Set this up on one of
the two computers. Profiles live in `profiles.toml` next to `devices.toml`. How
each OS reports network changes, the Location permission that macOS 14+ and
Windows 11 24H2 need to read Wi-Fi names, and the checks still to be run on real
hardware are in [WIFI-PROFILES.md](WIFI-PROFILES.md).

## Switching without an edge

`one-mk switch desk` puts the pointer in the middle of desk's main screen;
`one-mk switch next` goes round every online computer and back to this one;
`one-mk switch here` brings the mouse and keyboard back (and, if another computer
is controlling this one, takes it back, like `takeback`). Held keys are released
on the computer left behind, as with any crossing.

`one-mk lock-pointer on` (like Scroll Lock in Barrier) stops the edges from
leading anywhere, for example while playing a game here; `one-mk switch` still
works. `one-mk run --start-locked` starts that way.

## Keyboard shortcuts

They work wherever the pointer is:

| Shortcut | What it does |
|---|---|
| `Ctrl+Alt+1` ... `Ctrl+Alt+9` (Windows: `Ctrl+Alt+Shift+1` ... `9`) | Jump to computer 1 to 9 |
| `Ctrl+Alt+PageDown` | The next computer (`one-mk switch next`) |
| `Ctrl+Alt+P` | Lock the pointer to this computer, or unlock it |
| `Ctrl+Alt+H` | Back to this computer (`one-mk switch here`) |
| `Ctrl+Alt+B` (Windows: `Ctrl+Alt+Shift+B`) | Broadcast to every connected computer, or stop ([BROADCAST.md](BROADCAST.md)) |
| `Ctrl+Alt+Shift+Esc` | Emergency: always brings the pointer back to the computer you are sitting at. Cannot be changed or remapped |

Computers are numbered by name, this one included, so when every computer is
paired with every other, `Ctrl+Alt+2` means the same computer on every keyboard;
`one-mk hotkeys` shows the numbers. Only computers that exist get a shortcut. Change
the keys with, for example, `one-mk hotkeys --jump 'ctrl+shift+{n}' --cycle
'super+f12' --lock-pointer none --home-key 'ctrl+alt+end'` (modifiers `ctrl`,
`alt`, `shift`, `super`, then one key: a letter, digit, `f1`-`f24`, an arrow,
`pageup`, `space` and so on; `none` turns one off), or turn them all off with
`--off` (the emergency chord keeps working). They are kept in the `[hotkeys]` table
of `settings.toml`.

While the pointer is on this computer the system delivers them: the
GlobalShortcuts portal on Linux (GNOME 48+, KDE Plasma 6), `RegisterHotKey` on
Windows, a keyboard event tap on macOS (the Accessibility permission oneMK already
has). While it is on another computer, oneMK recognizes them in the keys it
captures: a shortcut fires only when its key goes down with exactly its modifiers
and no other key held, and that key is never sent to the other computer. Ctrl and
Alt are already there by then, so the switch releases them. The right Alt key
doesn't count as Alt there, because it is AltGr on many keyboards. While another
computer controls this one, this computer's own shortcuts are off (on Windows
unregistered, on macOS let through), so a chord typed over there reaches the
program here; on Linux the desktop still takes the chord, and oneMK ignores it.

- **Linux:** the portal accepts only installed programs, so run `one-mk install`
  once (it also writes a hidden desktop entry, `io.github.onemk.OneMk`), then
  restart oneMK. GNOME then asks you to confirm the shortcuts in a dialog and lets
  you change their keys under Settings > Keyboard (this dialog step has not been
  tried on a real desktop). Where the desktop has no GlobalShortcuts portal,
  `one-mk status` and `one-mk doctor` say so: the shortcuts then work only while
  the pointer is on another computer, so bind `one-mk switch next` and
  `one-mk switch here` to keys in your desktop settings instead.
- **Windows:** Windows reports AltGr as Ctrl+Alt, and AltGr+digit types `{ [ ] }`
  on German keyboards, so the jump shortcuts add Shift there by default. If AltGr+P
  or AltGr+H types a character on your keyboard, pick other keys, for example
  `one-mk hotkeys --lock-pointer 'ctrl+alt+shift+p'`. A shortcut another program
  already uses is skipped and `one-mk status` names it.
- Windows and macOS shortcuts have not been tried on real hardware.

## Edge rules

For every edge, or one edge with `--side left|right|top|bottom`:

```bash
one-mk edge --corner 20          # ignore crossings within 20 px of a screen corner
one-mk edge --dwell 250          # the pointer must rest at the edge for 250 ms
one-mk edge --modifier shift     # only cross while holding Shift (none|shift|ctrl|alt)
one-mk edge --fullscreen-guard on --fullscreen-allow vlc
one-mk edge                      # show the rules; --reset turns them off
one-mk edge --side left --reset  # that edge follows the shared rules again
```

While the pointer waits at an edge (dwell or modifier), moving away hands it back
at once, and typing or clicking there goes to this computer. Corners are those of
the desktop's outline. A click there is replayed as a whole click (press and
release at once), so no button can stay held; the catch is that a drag cannot
start at such an edge: start it away from the edge. The fullscreen guard keeps the
pointer here while the focused window fills its screen, except for programs in the
allow list (`--fullscreen-block` never leaves a program, fullscreen or not).
Windows can always tell; on Linux only X11 windows can be seen, which on Wayland
means programs running through XWayland (most games); macOS can't tell yet. Where
it can't tell, the pointer crosses as usual.

## Edges where no computer sits

Pointer wrap brings the pointer in at the opposite edge when you push it out
through an edge with no computer; layout learning notices when you push twice
toward a computer that isn't placed yet:

```bash
one-mk edge --wrap on            # left and right edges wrap round (all: top and bottom too; off: the default)
one-mk edge --learn suggest      # the default: "Is laptop on your left? Run: one-mk layout laptop left"
one-mk edge --learn auto         # place it there (left or right only) and say how to undo
one-mk edge --dismiss            # hide the question shown now
```

Details, limits and a manual test script: [EDGE-PUSH.md](EDGE-PUSH.md).

## Gaming mode

Games capture the pointer and want raw movement, not positions. With
`one-mk gaming on`, while the pointer is on another computer the mouse sends
relative movement, which never stops at that computer's screen edges, so it doesn't
lead back either: come back with `one-mk switch here` (or the emergency chord).
`auto` does this only while the other computer has a fullscreen window in front,
where it can tell. Both computers need a version of oneMK that has gaming mode; with
an older one, positions are sent as before. On Windows, the pointer speed setting
also applies to these movements outside games.

## Taking a computer back

When someone else's pointer is on your computer:

- On Windows and macOS, just use your computer's own mouse or keyboard.
- On Linux this doesn't work: the Wayland portals oneMK uses can't tell your own
  mouse and keyboard apart from the input oneMK injects. Run `one-mk takeback` (or
  `one-mk switch here`, or press `Ctrl+Alt+H`) instead. Without the shortcut portal,
  bind it to a key in your desktop's settings (GNOME: Settings > Keyboard > Custom
  Shortcuts; KDE: System Settings > Shortcuts). On X11 using your own mouse or
  keyboard takes the computer back.
- On every system, a computer that is controlled but receives no input (no key,
  click, scroll or pointer movement) is let go by itself, so a forgotten session
  never locks you out. The wait is **10 minutes** by default, set on the computer
  being controlled: `one-mk edge --idle-return 30m` (10 seconds to 24 hours; `60s`
  gives the old one-minute behaviour; `off` never lets go). Its pointer goes home
  and the next movement comes back in.

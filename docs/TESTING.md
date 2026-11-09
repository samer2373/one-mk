# Testing oneMK on real hardware

oneMK is verified end to end on Linux (GNOME on Wayland). Windows, macOS, KDE Plasma, wlroots
compositors and X11 desktops other than the ones tried are built and unit-tested but have **not been
run on real machines** ([README](../README.md#platform-support)). If you have one, this is the most
useful thing you can do for the project: run the steps below between two computers, note what
works, and report it.

You need two computers on the same network, one of them the system you are testing, running the
current release from the [releases page](https://github.com/samer2373/one-mk/releases/latest). You do not
need the source code.

## 1. Install and pair

- **Windows:** run `one-mk-x86_64-pc-windows-msvc-setup.exe`. SmartScreen will warn that the
  installer is unsigned: that is expected today. Allow oneMK through the Windows firewall when asked.
- **macOS:** open the `.dmg`, drag oneMK onto Applications, open it. The window guides you through
  Accessibility and Input Monitoring ([PERMISSION-GUIDE.md](PERMISSION-GUIDE.md)); say yes if macOS asks
  to find devices on the local network.
- **Linux:** `sudo apt install ./one-mk-x86_64-unknown-linux-gnu.deb`, the AppImage, or the
  one-line installer in the README. On Wayland, approve the two desktop permission dialogs.

Open oneMK on both computers and use **Pair a computer** (or `one-mk pair` in a terminal on each).
Check that both show the same six digits and confirm. Then place the computer where it really is
(drag it on the desk, or `one-mk layout "<name>" left`).

`one-mk status` on either computer should now say the other is online. **Emergency:** if the pointer
or keyboard ever gets stuck, `Ctrl+Alt+Shift+Esc` on the computer with the real mouse brings it home;
on the controlled computer, `one-mk takeback` takes it back.

If something fails, run `one-mk doctor` and keep its output. For detailed logs, stop oneMK and run
`one-mk -v run` in a terminal while you reproduce.

## 2. Checklist

Write pass or fail (and a note) for each.

**Mouse**
- [ ] Crossing works at the edge you configured, and the pointer appears at the same height.
- [ ] Movement feels smooth and immediate, slow and very fast. Note whether each computer is wired or
      on Wi-Fi.
- [ ] Click, double-click, triple-click (select a paragraph), right-click.
- [ ] Drag to select text, and drag a window on the other computer.
- [ ] Scroll both ways with a wheel and a trackpad: direction and speed comparable to local.

**Keyboard**
- [ ] Letters, numbers, symbols, Enter, Backspace and arrows on the other computer.
- [ ] Holding Backspace or an arrow repeats.
- [ ] Between a Mac and a PC, Ctrl+C/V on the PC keyboard acts as Cmd+C/V on the Mac, and back.
- [ ] Caps Lock. Hold Shift while moving back across the edge: no key stays stuck on either side.
- [ ] `Ctrl+Alt+Shift+Esc` brings the pointer home.
- [ ] Windows: AltGr characters on German or French layouts, and the shortcuts
      `Ctrl+Alt+Shift+1`, `Ctrl+Alt+Shift+B`.

**Clipboard and files**
- [ ] Copy text on one computer, move over, paste on the other. Both ways. Then an image.
- [ ] `one-mk send <file> --to "<name>"` arrives in `Downloads/one-mk`, both ways, including 1 GB or more.
- [ ] Right-click a file in the file manager: **Send to <computer>** (Explorer, Finder).

**Robustness**
- [ ] Close a laptop lid for a minute and open it: it reconnects by itself.
- [ ] Turn Wi-Fi off and on: it reconnects by itself.
- [ ] Lock the screen of a computer while its pointer is on the other: the pointer comes back.
- [ ] Touch the controlled computer's own mouse or keyboard: it takes control back (Windows, macOS, X11).

**Window and tray**
- [ ] The window opens from the tray; both computers stand on the desk on the right sides.
- [ ] Moving to the other computer draws the pointer crossing and the headline says "Controlling ...".
- [ ] Dragging the other computer to the opposite side changes where the mouse crosses, on both.
- [ ] Dropping a file on the other computer sends it. The tray icon and menu work.
- [ ] `one-mk install`, log out and back in: oneMK starts by itself.

**Lightness**
- [ ] With nothing happening, Task Manager or Activity Monitor shows oneMK near 0% CPU.

**Extras, if you want to go further** (all experimental): wallpaper video across screens
([LIVE-WALLPAPER.md](LIVE-WALLPAPER.md)), network profiles ([WIFI-PROFILES.md](WIFI-PROFILES.md)),
broadcast ([BROADCAST.md](BROADCAST.md)), a pointer on a computer with no mouse
([NO-MOUSE-POINTER.md](NO-MOUSE-POINTER.md)), and, on Linux, pointer wrap and layout learning with the
manual test at the end of [EDGE-PUSH.md](EDGE-PUSH.md).

## 3. Report it

Open an issue with the **Bug report** or **Feature request** form
([CONTRIBUTING.md](../CONTRIBUTING.md)). Include: your system and desktop, the oneMK version
(`one-mk --version`), the output of `one-mk doctor` from both computers, which checklist items failed and what
you saw, and, for speed or lag, the result of

```sh
one-mk bench --peer "<other computer's name>"
```

once wired if possible and once on Wi-Fi, and once while a large `one-mk send` is running.

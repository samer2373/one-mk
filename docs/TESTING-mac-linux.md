# Testing one-mk between a Mac and a Linux PC

This is the first run on real hardware for the macOS side, so expect some
rough edges. When something goes wrong, note what you did and copy the
output of the terminal running `one-mk -v run` around that moment.

## 1. Get the code onto the Mac

On the Linux PC (this creates one file containing the whole repository):

```sh
cd ~/.t3/projects/one-mk && git bundle create ~/one-mk.bundle --all
```

Copy `~/one-mk.bundle` to the Mac (USB stick, AirDrop from a phone, or
`scp you@linux-pc:one-mk.bundle .` if SSH is enabled). Then on the Mac:

```sh
git clone one-mk.bundle one-mk && cd one-mk
sh scripts/build-mac.sh
```

If the build fails, send the error output; the Mac-only code could not be
compiled on Linux.

## 2. Linux side

```sh
cd ~/.t3/projects/one-mk && cargo build --release -j 4
cp target/release/one-mk target/release/one-mk-tray target/release/one-mk-app ~/.local/bin/
```

## 3. Pair (both computers on the same network)

Run `one-mk pair` on both. Check that both show the same 6-digit code and
press `y` on both.

Then tell one of them where the other one physically is, for example on the
Linux PC if the Mac is on its left:

```sh
one-mk layout "<mac name>" left
```

The Mac picks up the mirrored side by itself (check with `one-mk devices`).

## 4. Start sharing

Run `one-mk -v run` in a terminal on both computers.

- **Linux (GNOME):** two permission dialogs appear, one for input capture
  and one for remote control. Allow both.
- **Mac:** System Settings opens. Turn on your terminal app (Terminal or
  iTerm) under *Privacy & Security → Accessibility* and *Input Monitoring*.
  one-mk notices within a second; no restart needed. If macOS asks whether
  to accept incoming network connections, allow it.

`one-mk status` on either computer should now say the other one is online.

**Emergency:** if the pointer or keyboard ever gets stuck,
`Ctrl+Alt+Shift+Esc` on the computer with the real mouse brings it home.
On the controlled computer, `one-mk takeback` takes it back.

## 5. Checklist

Write ✅ or ❌ (plus a note) for each.

**Mouse**
- [ ] Linux mouse → Mac: crossing works at the edge you configured.
- [ ] The pointer appears at the same height it left at.
- [ ] Movement feels smooth and immediate, with no lag or stutter. Try slow
      and very fast movements.
- [ ] Mac trackpad → Linux and back.
- [ ] Click, double-click, triple-click (select a paragraph), right-click.
- [ ] Drag to select text, and drag a window on the other computer.
- [ ] Scroll both ways with a wheel and a two-finger trackpad swipe:
      direction correct? Speed comparable to scrolling locally?
- [ ] Wi-Fi: does it still feel smooth? (Note whether each machine is wired
      or on Wi-Fi.)

**Keyboard**
- [ ] Typing on the other computer: letters, numbers, symbols, Enter,
      Backspace, arrows.
- [ ] Hold Backspace or an arrow key: it repeats.
- [ ] Copy/paste shortcuts: Ctrl+C/V on the Linux keyboard become Cmd+C/V on
      the Mac, and Cmd+C/V on the Mac become Ctrl+C/V on Linux.
- [ ] Caps Lock.
- [ ] Hold Shift while moving back across the edge: no key stays stuck on
      either side.
- [ ] `Ctrl+Alt+Shift+Esc` brings the pointer home.

**Clipboard and files**
- [ ] Copy text on one computer, move over, paste on the other. Both ways.
- [ ] Copy an image (a screenshot), move over, paste. Both ways.
- [ ] `one-mk send <file> --to "<name>"`: it arrives in `Downloads/one-mk`.
      Both ways, including a large file (1 GB or more).

**Robustness**
- [ ] Close the Mac lid for a minute and open it: reconnects by itself.
- [ ] Turn Wi-Fi off and on: reconnects by itself.
- [ ] On the Mac, click into a password field, then try to move the pointer
      to Linux: one-mk refuses and explains why.
- [ ] Lock the Mac screen while its pointer is on Linux: the pointer comes back.
- [ ] Touch the Mac's own trackpad while Linux controls it: the Mac takes
      control back.

**Tray and status**
- [ ] `one-mk-tray` on both: the icon appears and the menu shows the status.
      Changing a side in the menu works.
- [ ] `one-mk install`, then log out and back in: one-mk starts by itself.

**The window**
- [ ] "Open one·mk…" in the tray opens the window; both computers stand
      on the desk on the right sides.
- [ ] Move the mouse to the other computer: the window draws the pointer
      crossing over, and the headline says "Controlling …".
- [ ] Drag the other computer to the opposite side: the headline changes,
      and the mouse now crosses at that edge (both computers).
- [ ] Drop a file onto the other computer: it arrives in Downloads/one-mk.
- [ ] Pair a device from the window, against `one-mk pair` (or the window)
      on another computer: both show the same code; confirming pairs them.

**Lightness**
- [ ] With nothing happening, Activity Monitor on the Mac and `top` on Linux
      show one-mk near 0% CPU.

## 6. Numbers to send back

From the Linux PC, with one-mk running on both:

```sh
one-mk bench --peer "<mac name>"
```

Run it once with both computers wired if possible, and once on Wi-Fi.
Also run it while a large `one-mk send` is in progress.

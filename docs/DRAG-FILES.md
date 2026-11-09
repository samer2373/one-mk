# Dropping files on a screen edge (experimental)

Hold a file you are dragging over the thin strip at the screen edge that faces
another computer and let go: the file is copied into that computer's
received-files folder. It uses the same resumable transfer as `one-mk send`.

**It is off by default.** Nothing asks before a dropped file is sent, so it is a
switch you turn on yourself:

```
one-mk drag --on        # or the "Drop on screen edges" switch in the window
one-mk drag --off
one-mk drag --zone 20   # width of the strip, 1 to 128 pixels (default 12)
one-mk drag             # show whether it is on, and what works on this desktop
```

## Where it works

| Desktop | Status |
|---|---|
| Linux on X11 | Experimental. Tried with scripted drags on a private X server, not yet with a file manager on a real desktop. |
| Windows | Experimental. Built, never tried on a real Windows desktop. |
| macOS | Experimental. Built, never tried on a real Mac. It needs Accessibility and Input Monitoring permission. |
| Linux on Wayland | Not available: Wayland has no way to place a drop target at the screen edge. `one-mk drag --on` says so and changes nothing, and the window does not show the switch. |

Everywhere, `one-mk send` and the file manager's "Send to" entry
(`one-mk integrate`) are the supported way to send files, and they work the same
on every desktop.

## Things to know

- Only files and folders you can read are sent, at most 10000 files and 1 TiB in
  one drop. Symbolic links, pipes and empty folders are not sent.
- Any program that can talk to your X server can fake a drop, so on X11 this is
  not a protection boundary against hostile programs running as you. That is one
  reason it is opt-in.
- The setting is never chosen for you: it is off in a fresh settings file, on
  every desktop, until you switch it on.

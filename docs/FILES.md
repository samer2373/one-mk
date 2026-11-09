# Sending files

```bash
one-mk send photos/ notes.pdf            # to your only paired computer
one-mk send big.iso --to other-pc        # --to when you have several
one-mk notify on|off                     # a desktop notification when files arrive
one-mk partials [--clear]                # what an interrupted transfer left behind
```

Files land in `Downloads/one-mk` on the other computer (change it with
`one-mk run --downloads <dir>`). The same transfer is behind dropping files on a computer
in the window's **Desk**, the **Clipboard & files** tab, the file manager entry below, and the
experimental edge drop ([DRAG-FILES.md](DRAG-FILES.md)).

- **Resumable.** An interrupted transfer resumes when you send the same file again; the
  command says so when it is interrupted. A transfer that cannot finish leaves a partial
  file; partials older than 7 days are swept when the daemon starts and when a transfer
  finishes, and `one-mk partials` (also shown in `status` and `doctor`) lists what is kept.
- **Progress.** While files move, `one-mk status` shows a line such as
  `Receiving 12 files from desk: 45% (12.3 MB of 27.1 MB), 8.1 MB/s, 3 s left` (and
  `Sending ...` on the other computer); the window shows a progress bar per transfer.
  Nothing runs while idle.
- **Arrival notice.** When a transfer ends, the receiving computer shows one notification
  naming the files. On Linux it has a **Show in folder** button (over D-Bus); macOS shows
  the notification without the button; Windows does not notify yet. Turn it off with
  `one-mk notify off`, or for one run with `one-mk run --no-notify`.
- **Safe on arrival.** Received files cannot escape the download folder or overwrite
  anything (a numbered copy is made, names stay within 255 bytes), and every file is
  checked with BLAKE3 before it is kept; a file that cannot be placed is removed, not left
  hidden.
- **Limits.** At most 10000 files and 1 TiB in one send (also the per-file limit);
  inconsistent or oversized incoming batches are rejected before any progress is counted.
  A symbolic link named on the command line is followed when it points at a file; other
  links, pipes, empty folders and names that cannot be sent are skipped and reported in a
  summary line. The same path named twice is sent once.

## Send from the file manager

`one-mk install` (or `one-mk integrate`) adds **Send to <computer>** to the right-click
menu: Nautilus and Nemo (a script in `Scripts`), Dolphin (a service menu), Explorer (the
*Send to* menu) and Finder (a Quick Action, under *Services*). With several paired
computers there is one entry each, plus **Send with oneMK**, which takes the first. The
entries are rewritten whenever you pair or forget a computer, and `one-mk uninstall` (or
`one-mk integrate --remove`) removes them; files that are not oneMK's are never touched. If
sending fails, a desktop notification says why (Windows keeps the console window open
instead). On Linux an entry only appears for file managers that are installed.

Tried on a real system: the Nautilus, Nemo and Dolphin entries. The Windows and macOS
entries are written and unit-tested but **experimental**: they have not been seen in
Explorer or Finder.

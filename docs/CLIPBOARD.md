# Clipboard sharing and history

## Sharing

When the pointer moves to another computer, your clipboard (text, rich text,
images) goes along. `one-mk clip-send` and `one-mk clip-get` do the same on
demand. What is shared is up to you:

```
one-mk clipboard                          # show the settings
one-mk clipboard --text|--html|--image on|off
one-mk clipboard --max-mb 25              # largest clipboard, 1 to 256 MB
one-mk clipboard --skip-sensitive on|off  # default on
one-mk clipboard --allow-fetch on|off     # default off
```

- **Passwords stay put.** A copy that a password manager marked as sensitive is
  not pushed to other computers and is not handed out when another computer
  asks for your clipboard. The mark is read on X11 (the KDE password-manager
  hint) and has been tried there. Reading it on Wayland, Windows and macOS is
  built but has not been tried on real desktops. A password that no program
  marked cannot be recognised.
- **Sending one yourself is refused by default.** `one-mk clip-send` of a
  marked copy says so and sends nothing. `one-mk clip-send --allow-sensitive`
  sends it anyway; the other computer then puts it on its clipboard but never
  keeps it in its clipboard history, and does not pass it on. (A computer running
  an older oneMK does not know the mark and may record it.)
- **Others fetching your clipboard is off** until you run
  `one-mk clipboard --allow-fetch on`.

## History

Off by default. `one-mk clipboard history on` keeps your last 20 copies in
memory so you can bring one back.

```
one-mk clipboard history on|off
one-mk clipboard history list|show N|copy N|delete N|clear
one-mk clipboard history persist on|off   # keep it across restarts
```

- The list shows a masked placeholder, never the content; `show N` prints it.
- History is never recorded for copies a password manager marked as sensitive.
  It can still keep a password that no program marked, so leave it off if you
  copy those.
- **`persist on` saves the history to disk in plain text, not encrypted**
  (`clipboard-history.bin` in the settings folder, readable only by you but by
  every program you run). `persist off`, `history off` and `clear` delete or
  empty that file before the command returns. Deleting a file does not wipe the
  disk blocks.

## On demand and by shortcut

```
one-mk clip-send --to other-pc    # push mine to it now (--to is optional when only one is connected)
one-mk clip-get --from other-pc   # fetch its clipboard now
one-mk hotkeys --fetch-clipboard 'ctrl+alt+v'
```

`clip-send` tells you if the other computer ignored it (a kind switched off there,
or over its limit). `clip-get` works only if the computer being read has run
`one-mk clipboard --allow-fetch on`; fetching is off by default on every computer.

The fetch shortcut gets the clipboard of the next online computer (name order,
wrapping after this computer), then reports "Got text/html/image from NAME". It does
not type Ctrl+V: press your normal paste shortcut afterwards. Repeated presses select
the same peer; it is not a round-robin selector and does not fall back if that peer
refuses. Use `clip-get --from NAME` to pick another source. Linux and macOS default to
Ctrl+Alt+V; Windows uses Ctrl+Alt+Shift+V to avoid AltGr conflicts. On Windows the
result is printed to the console, because oneMK does not show Windows toast
notifications yet.

A switched-off kind is neither sent nor accepted; a clipboard over the limit (default
25 MB, at most 256) is not sent, and a bigger one that arrives is refused before it is
read. `one-mk run --no-clipboard` turns clipboard sharing off for one run.

## History settings

Retention is configured in `settings.toml` (restart, or run
`one-mk clipboard --text on`, to apply changes):

```toml
[clipboard]
history_max = 20           # 0 disables history; hard maximum 1000
history_max_mb = 16        # total retained bytes; hard maximum 256 MB
history_persist = false    # opt in only if you accept the disk privacy risk
history_skip_apps = ["keepassxc", "keepass", "1password", "bitwarden", "org.keepassxc.KeePassXC"]
```

- Retention has its own cap and is not the sharing size limit. Lowering the sharing
  limit does not delete history; restoring an oversized item needs the limit raised
  first. Consecutive identical copies are stored once, and the oldest images are evicted
  before text when a cap is reached.
- Without `history_persist`, history is held in memory only and forgotten at shutdown.
  With it, text and HTML are plain text on disk in `clipboard-history.bin` (PNG images
  alongside), written atomically with mode 0600 on Unix. Clear removes the retained
  contents and the saved file but does not wipe disk blocks or backups. The window asks
  for a second click to confirm Clear. Saves are debounced on a separate thread, so a
  crash can lose the last few changes.
- Lists and window rows are masked until you choose **Show** (or `history show N`).
  Opt-in history may retain up to 16 MiB by default; this is not a memory ceiling
  (decoded images and buffers add to it). With history off, ordinary local copies are
  neither retained nor PNG-encoded for history.
- Copies marked sensitive are never retained, even with `--skip-sensitive off`. The mark
  is read from X11's `x-kde-passwordManagerHint` (the only path tried on a real session),
  and, built but **not tried** on real desktops, the KDE hint on the Wayland data-control
  path, Windows' `ExcludeClipboardContentFromMonitorProcessing` and
  `CanIncludeInClipboardHistory`, and macOS' `org.nspasteboard.ConcealedType`. No
  mechanism detects every secret.
- `history_skip_apps` works on X11 only, and only where the desktop can name the focused
  app; it is best effort, not a guarantee about the real source app. On Windows and macOS
  the contents are captured when the clipboard is read, not at every copy, and anything
  already on the clipboard when oneMK starts enters history on the next access or change.
- Native Wayland monitoring and Windows/macOS clipboard behaviour are **experimental**.

## Troubleshooting

- Missing history items: a disabled format, the size cap, a password-manager window in
  focus, or `history_max = 0`. `one-mk status` and `one-mk doctor` show retention and what
  the platform covers.
- Saved history that cannot be read is reported, never silently overwritten; inspect the
  error, then run `one-mk clipboard history clear` to discard it.
- Fetch refused: run `one-mk clipboard --allow-fetch on` on the computer named in the
  message.
- With `--skip-sensitive on` (the default), sensitive or skipped-app content is withheld
  from pointer-follow pushes and `clip-get`. An explicit `clip-send` of such a copy is
  refused unless you add `--allow-sensitive`.
- Old settings (before `settings_version = 2`) that said `allow_pull = true` do not count
  as a choice to allow fetching: fetching is off until you run
  `one-mk clipboard --allow-fetch on`.

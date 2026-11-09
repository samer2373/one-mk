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
